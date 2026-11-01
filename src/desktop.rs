use anyhow::{Context, Result};
use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/optic-chat-{}", unsafe { libc::getuid() })));
    base.join("optic-chat").join("control.sock")
}
pub fn send(command: &str) -> Result<String> {
    let mut stream = UnixStream::connect(socket_path()).context("Optic Chat is not running")?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(command.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}
pub fn listen(
    alive: Arc<AtomicBool>,
    callback: impl Fn(String) -> String + Send + 'static,
) -> Result<()> {
    let path = socket_path();
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    std::thread::spawn(move || {
        while alive.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut peer, _)) => {
                    let _ = peer.set_read_timeout(Some(Duration::from_secs(1)));
                    let mut buf = [0; 256];
                    if let Ok(n) = peer.read(&mut buf) {
                        let reply = callback(String::from_utf8_lossy(&buf[..n]).trim().to_owned());
                        let _ = peer.write_all(reply.as_bytes());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(80))
                }
                Err(_) => break,
            }
        }
        let _ = std::fs::remove_file(path);
    });
    Ok(())
}
fn gsettings(args: &[&str]) -> Result<String> {
    let r = Command::new("gsettings").args(args).output()?;
    anyhow::ensure!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    Ok(String::from_utf8_lossy(&r.stdout).trim().into())
}
pub fn install_shortcut() -> Result<()> {
    install_shortcuts(&crate::config::Config::load()?.shortcuts)
}

pub fn validate_shortcuts(bindings: &[String]) -> Result<()> {
    anyhow::ensure!(bindings.len() == 8, "Expected eight shortcuts");
    let mut seen = std::collections::HashSet::new();
    for binding in bindings {
        if binding.is_empty() {
            continue;
        }
        let mut key = binding.as_str();
        let mut modified = false;
        while key.starts_with('<') {
            let end = key.find('>').context("Unclosed shortcut modifier")?;
            let modifier = &key[1..end];
            anyhow::ensure!(
                ["Control", "Alt", "Shift", "Super"].contains(&modifier),
                "Use Control, Alt, Shift or Super modifiers"
            );
            modified |= modifier != "Shift";
            key = &key[end + 1..];
        }
        let valid = key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric()
            || [
                "space", "Left", "Right", "Up", "Down", "Home", "End", "Return", "Escape",
            ]
            .contains(&key)
            || key
                .strip_prefix('F')
                .and_then(|s| s.parse::<u8>().ok())
                .is_some_and(|n| (1..=24).contains(&n));
        anyhow::ensure!(
            modified && valid,
            "Use a modifier and key, for example <Control><Super>h or <Control><Super>Left"
        );
        let mut modifiers: Vec<_> = binding[..binding.len() - key.len()]
            .split('>')
            .filter(|s| !s.is_empty())
            .collect();
        modifiers.sort();
        anyhow::ensure!(
            seen.insert(format!("{}{}", modifiers.join(">"), key.to_lowercase())),
            "Each shortcut must be different"
        );
    }
    Ok(())
}
pub fn install_shortcuts(bindings: &[String]) -> Result<()> {
    validate_shortcuts(bindings)?;
    anyhow::ensure!(
        std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_lowercase()
            .contains("gnome"),
        "Global shortcuts currently require GNOME."
    );
    let schema = "org.gnome.settings-daemon.plugins.media-keys";
    let list = gsettings(&["get", schema, "custom-keybindings"])?;
    let trimmed = list.trim_start_matches("@as ").trim();
    anyhow::ensure!(
        trimmed.starts_with('[') && trimmed.ends_with(']'),
        "Unexpected shortcut format"
    );
    let mut items = trimmed[1..trimmed.len() - 1].trim().to_owned();
    for (i, (id, name, arg)) in [
        ("optic-chat-window", "show or hide", "--toggle-window"),
        ("optic-chat", "toggle response", "--toggle-bubble"),
        ("optic-chat-tl", "move left", "--move=left"),
        ("optic-chat-tr", "move up", "--move=up"),
        ("optic-chat-bl", "move down", "--move=down"),
        ("optic-chat-br", "move right", "--move=right"),
        ("optic-chat-input", "focus input", "--focus-input"),
        ("optic-chat-mic", "start or stop dictation", "--toggle-mic"),
    ]
    .iter()
    .enumerate()
    {
        let path =
            format!("/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/{id}/");
        let custom =
            format!("org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:{path}");
        let command = format!("\"{}\" {arg}", std::env::current_exe()?.display());
        gsettings(&["set", &custom, "name", &format!("Optic Chat · {name}")])?;
        let quoted = format!("'{}'", command.replace('\\', "\\\\").replace('\'', "\\'"));
        gsettings(&["set", &custom, "command", &quoted])?;
        gsettings(&["set", &custom, "binding", &format!("'{}'", bindings[i])])?;
        if !list.contains(&path) {
            if !items.is_empty() {
                items.push_str(", ");
            }
            items.push_str(&format!("'{path}'"));
        }
    }
    gsettings(&["set", schema, "custom-keybindings", &format!("[{items}]")])?;
    Ok(())
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    #[test]
    fn rejects_duplicates_and_bare_keys() {
        let mut keys = crate::config::Config::default().shortcuts;
        validate_shortcuts(&keys).unwrap();
        keys[1] = keys[0].clone();
        assert!(validate_shortcuts(&keys).is_err());
        keys[1] = "h".into();
        assert!(validate_shortcuts(&keys).is_err());
        keys[1] = "".into();
        validate_shortcuts(&keys).unwrap();
    }
}
