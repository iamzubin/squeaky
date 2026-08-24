//! Director feed writer — how this sidecar drives the buddy overlay.
//!
//! Appends one JSON event per line to ~/.local/state/heyclicky/director.jsonl;
//! rust-buddy tails it and executes:
//!   {"cmd":"fly","x":..,"y":..,"label":"click here!"}  -> bezier flight + bubble
//!   {"cmd":"say","text":"researching…"}                -> bubble at the buddy

use anyhow::Result;
use std::fs::OpenOptions;
use std::io::Write;

fn path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".local/state/heyclicky/director.jsonl")
}

pub fn append(event: serde_json::Value) -> Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&p)?;
    writeln!(f, "{}", event)?;
    Ok(())
}

pub fn fly(x: i32, y: i32, label: &str) -> Result<()> {
    append(serde_json::json!({ "cmd": "fly", "x": x, "y": y, "label": label }))
}

pub fn say(text: &str) -> Result<()> {
    append(serde_json::json!({ "cmd": "say", "text": text }))
}
