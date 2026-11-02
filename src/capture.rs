//! GNOME/Wayland screen capture via one permissioned portal session.
//! GStreamer is a small system helper, not a browser or bundled runtime.
use crate::{
    buffer::{Frame, FrameBuffer, PreparedFrame},
    config::Config,
};
use anyhow::{Context, Result};
use ashpd::desktop::{
    PersistMode,
    screencast::{CursorMode, Screencast, SourceType},
};
use futures_util::StreamExt;
use std::{
    os::fd::AsRawFd,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command};

const CAPTURE_RATE: &str = "video/x-raw,framerate=4/1";

pub async fn run(
    config: Arc<Mutex<Config>>,
    buffer: Arc<Mutex<FrameBuffer>>,
    stop: Arc<AtomicBool>,
    refresh: Arc<AtomicBool>,
    status: impl Fn(String) + Send + 'static,
) -> Result<()> {
    status("Choose a screen or window to share…".into());
    let portal = Screencast::new()
        .await
        .context("Screen sharing portal is unavailable")?;
    let session = portal.create_session().await?;
    let capturing = async {
        portal
            .select_sources(
                &session,
                CursorMode::Hidden,
                SourceType::Monitor | SourceType::Window,
                false,
                None,
                PersistMode::DoNot,
            )
            .await?
            .response()?;
        let response = portal.start(&session, None).await?.response()?;
        let stream = response
            .streams()
            .first()
            .context("No screen was selected")?;
        let remote = portal.open_pipe_wire_remote(&session).await?;
        let fd = remote.as_raw_fd();
        let cfg = config.lock().unwrap().clone();
        let (w, h) = stream.size().unwrap_or((1920, 1080));
        let width = (w as u32).min(cfg.image_width).max(1);
        let height = ((h as u64 * width as u64 / (w as u64).max(1)) as u32).max(1);
        let mut cmd = Command::new("gst-launch-1.0");
        cmd.args([
            "-q",
            "pipewiresrc",
            &format!("fd={fd}"),
            &format!("path={}", stream.pipe_wire_node_id()),
            "do-timestamp=true",
            // Damage-driven sources may stop producing when the screen is still.
            // Repeat the actual last frame so a final change cannot be dropped by
            // the frame limiter or by the configurable acceptance interval.
            "keepalive-time=100",
            "!",
            "videorate",
            "drop-only=true",
            "!",
            CAPTURE_RATE,
            "!",
            "queue",
            "max-size-buffers=1",
            "max-size-bytes=0",
            "max-size-time=0",
            "leaky=downstream",
            "!",
            "videoconvert",
            "!",
            "videoscale",
            "!",
            &format!("video/x-raw,width={width},height={height},pixel-aspect-ratio=1/1"),
            "!",
            "jpegenc",
            &format!("quality={}", cfg.jpeg_quality),
            "!",
            "fdsink",
            "fd=1",
            "sync=false",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
        // Only the granted PipeWire remote is inherited by the capture helper.
        // The owned fd and portal session stay alive until after the child exits.
        let parent = std::process::id() as libc::pid_t;
        unsafe {
            cmd.pre_exec(move || {
                #[cfg(target_os = "linux")]
                {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != parent {
                        libc::_exit(1);
                    }
                }
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd
            .spawn()
            .context("Install GStreamer tools and its PipeWire/JPEG plugins to enable capture")?;
        let stdout = child.stdout.take().unwrap();
        let result = collect_frames(stdout, &config, &buffer, &stop, &refresh, &status).await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        result
    };
    // Cancel even during the chooser, and always close the portal afterwards.
    let result = match session.receive_closed().await {
        Ok(closed) => {
            let mut closed = Box::pin(closed);
            tokio::select! {
                biased;
                _ = async { while !stop.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(50)).await; } } => Ok(()),
                _ = closed.next() => Err(anyhow::anyhow!("Screen sharing ended. Click Share to reconnect.")),
                result = capturing => result,
            }
        }
        Err(error) => Err(error.into()),
    };
    let _ = session.close().await;
    result
}

pub fn stop_and_clear(stop: &AtomicBool, buffer: &Mutex<FrameBuffer>) {
    stop.store(true, Ordering::Relaxed);
    buffer.lock().unwrap().clear();
}

async fn collect_frames(
    mut stdout: impl tokio::io::AsyncRead + Unpin,
    config: &Mutex<Config>,
    buffer: &Mutex<FrameBuffer>,
    stop: &AtomicBool,
    refresh: &AtomicBool,
    status: &impl Fn(String),
) -> Result<()> {
    let mut chunk = [0u8; 65536];
    let mut pending = Vec::new();
    let mut next_sample = Instant::now();
    let mut cached: Option<PreparedFrame> = None;
    let mut last_received = Instant::now();
    status("Sharing • waiting for a screen change".into());
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let read = tokio::select! {
            r=stdout.read(&mut chunk)=>r?,
            _=tokio::time::sleep(Duration::from_millis(250))=>{
                anyhow::ensure!(last_received.elapsed() < Duration::from_secs(10), "Screen capture stopped delivering frames. Click Share to reconnect.");
                continue;
            },
        };
        if read == 0 {
            anyhow::bail!("Screen sharing ended. Click Share to reconnect.")
        }
        pending.extend_from_slice(&chunk[..read]);
        // Skip obsolete frames if a read contains more than one full JPEG.
        let mut newest = None;
        while let Some(frame) = take_jpeg(&mut pending) {
            newest = Some(frame);
        }
        if let Some(frame) = newest {
            last_received = Instant::now();
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let cfg = config.lock().unwrap().clone();
            let requested = refresh.swap(false, Ordering::Relaxed);
            let received_at = Instant::now();
            if !requested && received_at < next_sample {
                continue;
            }
            // Keep a stable cadence; encoding/read jitter must not turn 0.5 s
            // into 0.75 s. A question doesn't delay the next background sample.
            let interval = Duration::from_secs_f64(cfg.sample_seconds.clamp(0.25, 30.0));
            if received_at >= next_sample {
                let overdue = received_at.duration_since(next_sample).as_secs_f64();
                next_sample += interval.mul_f64((overdue / interval.as_secs_f64()).floor() + 1.0);
            }
            let prepared = match &cached {
                Some(previous) if previous.matches(&frame) => previous.observed_at(received_at),
                _ => {
                    PreparedFrame::from_jpeg(frame, received_at).context("Invalid capture frame")?
                }
            };
            let mut b = buffer.lock().unwrap();
            // Clear sets stop before locking the buffer. Never refill it
            // from an in-flight decode after that clear has completed.
            if stop.load(Ordering::Relaxed) {
                break;
            }
            b.insert(prepared.clone(), cfg.change_percent);
            cached = Some(prepared);
            let (count, bytes) = b.stats();
            status(format!("Sharing • {count}/10 frames • {} KB", bytes / 1024));
        }
        anyhow::ensure!(
            pending.len() < 8_000_000,
            "Capture stream exceeded its memory limit"
        );
    }
    Ok(())
}

/// Do not silently send old images when a live capture has stalled. A question
/// bypasses the sampling interval and waits for an image received after it.
pub async fn fresh_frames(
    buffer: &Mutex<FrameBuffer>,
    stop: &AtomicBool,
    refresh: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<Vec<Frame>> {
    if stop.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }
    let requested_at = Instant::now();
    refresh.store(true, Ordering::Relaxed);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Request cancelled");
            anyhow::ensure!(!stop.load(Ordering::Relaxed), "Screen sharing stopped before a fresh screenshot arrived. Share again, or retry without screen sharing.");
            let frames = buffer.lock().unwrap().latest_five();
            if frames.last().is_some_and(|f| f.captured >= requested_at) { return Ok(frames); }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.context("No fresh screenshot arrived. Click Share to reconnect, then try again.")?
}
pub fn take_jpeg(pending: &mut Vec<u8>) -> Option<Vec<u8>> {
    let start = pending.windows(2).position(|x| x == [0xff, 0xd8])?;
    if start > 0 {
        pending.drain(..start);
    }
    let end = pending.windows(2).position(|x| x == [0xff, 0xd9])? + 2;
    Some(pending.drain(..end).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn live_stream_refreshes_on_question_and_cannot_refill_after_clear() {
        let mut child = Command::new("gst-launch-1.0")
            .args([
                "-q",
                "videotestsrc",
                "is-live=true",
                "pattern=ball",
                "!",
                "video/x-raw,width=160,height=90,framerate=10/1",
                "!",
                "videorate",
                "drop-only=true",
                "!",
                CAPTURE_RATE,
                "!",
                "jpegenc",
                "quality=80",
                "!",
                "fdsink",
                "fd=1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut cfg = Config::default();
        cfg.sample_seconds = 30.0;
        let config = Arc::new(Mutex::new(cfg));
        let buffer = Arc::new(Mutex::new(FrameBuffer::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let refresh = Arc::new(AtomicBool::new(false));
        let collector = {
            let (config, buffer, stop, refresh) = (
                config.clone(),
                buffer.clone(),
                stop.clone(),
                refresh.clone(),
            );
            tokio::spawn(async move {
                collect_frames(stdout, &config, &buffer, &stop, &refresh, &|_| {}).await
            })
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            while buffer.lock().unwrap().stats().0 == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut delays = Vec::new();
        for _ in 0..6 {
            let old = buffer.lock().unwrap().latest_five().pop().unwrap();
            let asked = Instant::now();
            let frames = fresh_frames(&buffer, &stop, &refresh, &AtomicBool::new(false))
                .await
                .unwrap();
            delays.push(asked.elapsed());
            let current = frames.last().unwrap();
            assert!(current.captured > old.captured);
            assert_ne!(
                current.jpeg, old.jpeg,
                "A question must capture the moving screen before the 30-second interval"
            );
        }
        let average = delays.iter().map(Duration::as_secs_f64).sum::<f64>() / delays.len() as f64;
        eprintln!(
            "Live GStreamer fresh-frame wait: mean {:.0} ms; maximum {:.0} ms",
            average * 1000.0,
            delays.iter().max().unwrap().as_secs_f64() * 1000.0
        );
        assert!(average < 0.7, "Capture refresh should be sub-second");
        stop_and_clear(&stop, &buffer);
        tokio::time::timeout(Duration::from_secs(1), collector)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(buffer.lock().unwrap().stats(), (0, 0));
        assert!(
            fresh_frames(&buffer, &stop, &refresh, &AtomicBool::new(false))
                .await
                .unwrap()
                .is_empty()
        );
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    #[tokio::test]
    async fn stalled_or_cancelled_capture_never_supplies_old_images() {
        let buffer = Mutex::new(FrameBuffer::default());
        buffer
            .lock()
            .unwrap()
            .consider(image::DynamicImage::new_rgb8(100, 60), 12, 1280, 60)
            .unwrap();
        let stop = AtomicBool::new(false);
        let refresh = AtomicBool::new(false);
        assert!(
            fresh_frames(&buffer, &stop, &refresh, &AtomicBool::new(true))
                .await
                .is_err()
        );
        let error = fresh_frames(&buffer, &stop, &refresh, &AtomicBool::new(false))
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("No fresh screenshot"));
        stop_and_clear(&stop, &buffer);
        assert_eq!(buffer.lock().unwrap().stats(), (0, 0));
    }
    #[test]
    fn jpeg_stream_boundaries() {
        let mut b = vec![0, 0xff, 0xd8, 1];
        assert!(take_jpeg(&mut b).is_none());
        b.extend([2, 0xff, 0xd9, 0xff, 0xd8, 3, 0xff, 0xd9]);
        assert_eq!(
            take_jpeg(&mut b).unwrap(),
            vec![0xff, 0xd8, 1, 2, 0xff, 0xd9]
        );
        assert!(take_jpeg(&mut b).is_some());
        assert!(b.is_empty());
    }
    #[test]
    fn installed_gstreamer_produces_decodable_frames() {
        let out = std::process::Command::new("gst-launch-1.0")
            .args([
                "-q",
                "videotestsrc",
                "num-buffers=3",
                "!",
                "video/x-raw,width=160,height=90",
                "!",
                "jpegenc",
                "quality=60",
                "!",
                "fdsink",
                "fd=1",
            ])
            .output()
            .expect("GStreamer tools are required for screen capture");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut bytes = out.stdout;
        let mut count = 0;
        while let Some(jpeg) = take_jpeg(&mut bytes) {
            let image = image::load_from_memory(&jpeg).unwrap();
            assert_eq!((image.width(), image.height()), (160, 90));
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
