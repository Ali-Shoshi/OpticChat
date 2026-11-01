//! GNOME/Wayland screen capture via one permissioned portal session.
//! GStreamer is a small system helper, not a browser or bundled runtime.
use crate::{buffer::FrameBuffer, config::Config};
use anyhow::{Context, Result};
use ashpd::desktop::{
    PersistMode,
    screencast::{CursorMode, Screencast, SourceType},
};
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

pub async fn run(
    config: Arc<Mutex<Config>>,
    buffer: Arc<Mutex<FrameBuffer>>,
    stop: Arc<AtomicBool>,
    status: impl Fn(String) + Send + 'static,
) -> Result<()> {
    status("Choose a screen or window to share…".into());
    let portal = Screencast::new()
        .await
        .context("Screen sharing portal is unavailable")?;
    let session = portal.create_session().await?;
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
        "!",
        "videorate",
        "drop-only=true",
        "!",
        "video/x-raw,framerate=1/1",
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
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .kill_on_drop(true);
    // Only the granted PipeWire remote is inherited by the capture helper.
    // The owned fd and portal session stay alive until after the child exits.
    unsafe {
        cmd.pre_exec(move || {
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
    let mut stdout = child.stdout.take().unwrap();
    let mut chunk = [0u8; 65536];
    let mut pending = Vec::new();
    let mut last = Instant::now() - Duration::from_secs(60);
    status("Sharing • waiting for a screen change".into());
    let result: Result<()> = async {
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let read = tokio::select! {
                r=stdout.read(&mut chunk)=>r?,
                _=tokio::time::sleep(Duration::from_millis(250))=>continue,
            };
            if read == 0 {
                anyhow::bail!("Screen sharing ended. Click Share to reconnect.")
            }
            pending.extend_from_slice(&chunk[..read]);
            while let Some(frame) = take_jpeg(&mut pending) {
                let cfg = config.lock().unwrap().clone();
                if last.elapsed() < Duration::from_secs(cfg.sample_seconds) {
                    continue;
                }
                last = Instant::now();
                let image = image::load_from_memory(&frame).context("Invalid capture frame")?;
                let mut b = buffer.lock().unwrap();
                b.consider(image, cfg.change_percent, cfg.image_width, cfg.jpeg_quality)?;
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
    .await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = session.close().await;
    result
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
