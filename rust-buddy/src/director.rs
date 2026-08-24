//! Agent command feed (~/.local/state/heyclicky/director.jsonl).
//!
//! The squeaky-agent sidecar appends one JSON event per line:
//!   {"cmd":"fly","x":800,"y":500,"label":"click here"}
//!   {"cmd":"say","text":"researching…"}
//! We poll the mtime each tick (same pattern as settings.json) and hand
//! back the lines we haven't executed yet. Line-count tracking; a shrunk
//! file means it was truncated/rotated -> replay from the top.

use crate::state::DirectorCmd;
use serde_json::Value;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

pub fn path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/state/heyclicky/director.jsonl")
}

fn mtime_us(p: &PathBuf) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Collect unconsumed commands, updating the cursor in place.
pub fn poll(seen: &mut usize, last_mtime: &mut u64) -> Vec<DirectorCmd> {
    let p = path();
    let mt = mtime_us(&p);
    if mt == *last_mtime {
        return Vec::new();
    }
    *last_mtime = mt;
    let txt = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => {
            *seen = 0;
            return Vec::new();
        }
    };
    let lines: Vec<&str> = txt.lines().collect();
    if lines.len() < *seen {
        *seen = 0;
    }
    let mut out = Vec::new();
    for line in &lines[*seen..] {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["cmd"].as_str() {
            Some("fly") => out.push(DirectorCmd::Fly {
                x: v["x"].as_i64().unwrap_or(0) as i32,
                y: v["y"].as_i64().unwrap_or(0) as i32,
                label: v["label"].as_str().unwrap_or("look here").to_string(),
            }),
            Some("say") => out.push(DirectorCmd::Say {
                text: v["text"].as_str().unwrap_or_default().to_string(),
            }),
            _ => {}
        }
    }
    *seen = lines.len();
    out
}
