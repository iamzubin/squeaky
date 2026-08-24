//! Shared app state: buddy modes + the per-run context.

use std::collections::VecDeque;

// buddy (logical px) — triangle drawn in cairo, color from settings cursor_color
pub const SIZE: i32 = 24;
// tip position inside the 24px triangle for flight mode pointing
pub const HOT_X: i32 = 5;
pub const HOT_Y: i32 = 3;
pub const HIDDEN: i32 = -1000; // park-offscreen margin

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Hidden, // idle (ink may still be fading)
    Draw,   // hotkey held: buddy is the pen, ink trails the cursor
    Fly { sx: f64, sy: f64, tx: f64, ty: f64, t0: i64, dur: f64, ret: bool }, // demo: bezier flight
    Point { x: i32, y: i32, until: i64 }, // demo: bubble "right here!"
}

// synthetic pen driver for the `scribble` self-test (no real hotkey needed)
pub struct Syn {
    pub cx: f64,
    pub cy: f64,
    pub i: i32,
    pub n: i32,
}

pub struct Ctx {
    pub mode: Mode,
    pub buddy_pos: (i32, i32),   // tip position
    pub queue: VecDeque<(i32, i32)>, // demo flight targets
    pub trail: Vec<(f64, f64, i64)>, // ink points w/ birth time (us) — fades tail-first
    pub syn: Option<Syn>,
    pub pts_session: usize,      // points laid this pen session (tap w/ 0 movement = no capture)
    pub session_start_mono: i64, // pen-down time (us, monotonic)
    pub session_start_wall: u64, // pen-down time (ms, epoch)
    // settings-driven (hot-reloaded from ~/.config/heyclicky/settings.json)
    pub gap_x: i32,              // buddy float distance right of pointer
    pub gap_y: i32,              // buddy float distance below pointer
    pub voice_enabled: bool,     // capture + transcribe while hotkey held
    pub cursor_color: String,    // hex e.g. "#A78BFA"
    pub mic: Option<crate::stt::MicCapture>,
    pub stt: Option<crate::stt::SttHandle>,
    pub settings_mtime: u64,     // last-seen settings.json mtime (hot-reload trigger)
    pub last_session_id: Option<String>, // ink session the next transcript belongs to
    pub model_name: String,      // active whisper model (for status.json)
    pub last_transcript: String, // most recent voice transcript (for status.json)
}
