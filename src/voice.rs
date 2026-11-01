use anyhow::{Context, Result};
use std::{io::{BufRead, BufReader, Write}, process::{Child, ChildStdin, Command, Stdio}, sync::{Arc, Mutex}};

struct Session { child: Child, stdin: ChildStdin }
#[derive(Clone, Default)]
pub struct Voice(Arc<Mutex<Option<Session>>>);
impl Voice {
    pub fn start(&self, update: impl Fn(serde_json::Value) + Send + 'static) -> Result<()> {
        use std::os::unix::process::CommandExt;
        let mut state = self.0.lock().unwrap();
        anyhow::ensure!(state.is_none(), "Voice input is already active");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let python = root.join(".work/voice-venv/bin/python");
        anyhow::ensure!(python.exists(), "Voice setup is missing. Run scripts/setup-voice.sh.");
        let mut child = Command::new(python)
            .arg(root.join("scripts/voice_input.py"))
            .env("HF_HUB_OFFLINE", "1").env("OMP_NUM_THREADS", "4")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .process_group(0).spawn().context("Could not start local transcription")?;
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        *state = Some(Session { child, stdin });
        drop(state);
        let shared = self.0.clone();
        std::thread::spawn(move || {
            let mut final_event = None;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) {
                    if event.get("text").is_some() || event.get("error").is_some() {
                        final_event = Some(event);
                    } else { update(event); }
                }
            }
            if let Some(mut session) = shared.lock().unwrap().take() { let _ = session.child.wait(); }
            update(final_event.unwrap_or_else(|| serde_json::json!({"error":"Voice input stopped unexpectedly. Check your microphone and local voice setup."})));
        });
        Ok(())
    }
    pub fn stop(&self) -> Result<()> {
        if let Some(session) = self.0.lock().unwrap().as_mut() {
            session.stdin.write_all(b"stop\n").context("Could not stop recording")?;
        }
        Ok(())
    }
    pub fn shutdown(&self) {
        if let Some(mut session) = self.0.lock().unwrap().take() {
            // Terminate the worker and any recorder it owns, including on app quit.
            unsafe { libc::kill(-(session.child.id() as i32), libc::SIGTERM); }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while matches!(session.child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if matches!(session.child.try_wait(), Ok(None)) {
                unsafe { libc::kill(-(session.child.id() as i32), libc::SIGKILL); }
            }
            let _ = session.child.wait();
        }
    }
}
