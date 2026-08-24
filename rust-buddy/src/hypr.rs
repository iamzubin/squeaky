//! Hyprland IPC: cursor position + focused output.
//!
//! Socket quirks (see NOTES.md): request must have NO trailing newline +
//! shutdown(WRITE), then read to EOF. One connection per request is fine
//! (unix connect is ~µs, we poll at 60fps).

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;

fn socket_path() -> Option<String> {
    let his = env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let uid = nix::unistd::Uid::current().as_raw();
    Some(format!("/run/user/{}/hypr/{}/.socket.sock", uid, his))
}

fn query(request: &str) -> Option<Vec<u8>> {
    let mut stream = UnixStream::connect(socket_path()?).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 256];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    Some(buf)
}

/// Cursor position in logical px (same space as window `at`/`size`).
pub fn cursor_pos() -> Option<(i32, i32)> {
    let v: serde_json::Value = serde_json::from_slice(&query("j/cursorpos")?).ok()?;
    Some((v.get("x")?.as_i64()? as i32, v.get("y")?.as_i64()? as i32))
}

/// Name of the currently focused output (for grim `-o`).
pub fn focused_monitor() -> Option<String> {
    let out = Command::new("hyprctl").arg("-j").arg("monitors").output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.as_array()?
        .iter()
        .find(|m| m.get("focused").and_then(|f| f.as_bool()).unwrap_or(false))
        .and_then(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
}
