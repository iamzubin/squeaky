//! Session records + screenshot capture — the agent feed.
//!
//! On pen-up a product-shaped record is built, the screenshot (screen WITH
//! the overlay ink composited) lands in ~/Pictures/heyclicky + clipboard,
//! and the full record is appended to ~/.local/state/heyclicky/sessions.jsonl.

use std::env;
use std::process::Command;

use crate::hypr;

pub fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Product-shaped record of one pen session (what an agent hook would consume).
/// Points are logical px + ms since pen-down; bbox is the ink extent.
pub fn session_record(
    trail: &[(f64, f64, i64)],
    start_mono: i64,
    start_wall: u64,
    ended_mono: i64,
) -> serde_json::Value {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let pts: Vec<serde_json::Value> = trail.iter().map(|p| {
        x0 = x0.min(p.0); y0 = y0.min(p.1); x1 = x1.max(p.0); y1 = y1.max(p.1);
        serde_json::json!({
            "x": (p.0 * 10.0).round() / 10.0,
            "y": (p.1 * 10.0).round() / 10.0,
            "t_ms": (p.2 - start_mono) / 1000,
        })
    }).collect();
    if trail.is_empty() {
        (x0, y0, x1, y1) = (0.0, 0.0, 0.0, 0.0);
    }
    serde_json::json!({
        "id": format!("ink-{}", start_wall),
        "kind": "ink_session",
        "started_at_ms": start_wall,
        "ended_at_ms": wall_ms(),
        "duration_ms": (ended_mono - start_mono) / 1000,
        "point_count": trail.len(),
        "bbox_logical": [x0.round(), y0.round(), x1.round(), y1.round()],
        "points": pts,
    })
}

/// Grab screenshot w/ ink -> file + clipboard, attach path to the session
/// record, print it, and append the full record to the sessions jsonl.
/// Returns the session id so voice transcripts can reference it.
pub fn capture_for_agent(mut session: serde_json::Value) -> String {
    let id = session["id"].as_str().unwrap_or_default().to_string();
    std::thread::spawn(move || {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let dir = format!("{}/Pictures/heyclicky", home);
        if std::fs::create_dir_all(&dir).is_err() {
            println!("CAPTURE: can't create {}", dir);
            return;
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = format!("{}/ink-{}.png", dir, ts);
        // focused output (logical coords; grim composites our overlay ink too)
        let mut cmd = Command::new("grim");
        if let Some(mon) = hypr::focused_monitor() {
            cmd.arg("-o").arg(mon);
        }
        match cmd.arg(&path).status() {
            Ok(s) if s.success() => {
                let copy = Command::new("sh").arg("-c")
                    .arg(format!("exec wl-copy -t image/png < '{}'", path))
                    .status();
                match copy {
                    Ok(_) => println!("CAPTURE: {} (on clipboard, paste it to your agent)", path),
                    Err(e) => println!("CAPTURE: {} (wl-copy failed: {})", path, e),
                }
                session["screenshot"] = serde_json::json!(path);
            }
            _ => println!("CAPTURE: grim failed"),
        }
        // console: full record minus the point array
        let mut summary = session.clone();
        summary["points"] = serde_json::json!(format!("[{} pts]", session["point_count"]));
        println!("SESSION: {}", summary);
        // durable event log (agent integration reads this later)
        let state_dir = format!("{}/.local/state/heyclicky", home);
        let _ = std::fs::create_dir_all(&state_dir);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true).append(true)
            .open(format!("{}/sessions.jsonl", state_dir))
        {
            use std::io::Write;
            let _ = writeln!(f, "{}", session);
        }
    });
    id
}

/// Append a follow-up event (e.g. a voice transcript) tied to a session id.
pub fn append_event(event: serde_json::Value) {
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let state_dir = format!("{}/.local/state/heyclicky", home);
    let _ = std::fs::create_dir_all(&state_dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(format!("{}/sessions.jsonl", state_dir))
    {
        use std::io::Write;
        let _ = writeln!(f, "{}", event);
    }
}
