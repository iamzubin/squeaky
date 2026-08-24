//! Demo agent-pointing flights — port of the heyclicky OverlayWindow.swift
//! bezier math (OverlayWindow.swift:495/542). The buddy flies to targets on
//! an arc and holds a "right here!" bubble. Unused by the draw hotkey;
//! kept for the `demo` mode.

use crate::state::Mode;

const FLIGHT_DIST_DIV: f64 = 800.0;
const FLIGHT_MIN: f64 = 0.6;
const FLIGHT_MAX: f64 = 1.4;
pub const ARC_HEIGHT_FACTOR: f64 = 0.2;
pub const ARC_HEIGHT_MAX: f64 = 80.0;
pub const POINT_HOLD_MS: i64 = 1200; // bubble hold (heyclicky: 3s, shortened for demo)

pub fn flight_params(sx: i32, sy: i32, tx: i32, ty: i32, now: i64, ret: bool) -> Mode {
    let dist = dist(sx as f64, sy as f64, tx as f64, ty as f64);
    let dur = (dist / FLIGHT_DIST_DIV).clamp(FLIGHT_MIN, FLIGHT_MAX);
    Mode::Fly { sx: sx as f64, sy: sy as f64, tx: tx as f64, ty: ty as f64, t0: now, dur, ret }
}

// quadratic bezier B(t) with control point above midpoint
pub fn bezier(sx: f64, sy: f64, tx: f64, ty: f64, t: f64) -> (f64, f64) {
    let mx = (sx + tx) / 2.0;
    let my = (sy + ty) / 2.0;
    let arc = (dist(sx, sy, tx, ty) * ARC_HEIGHT_FACTOR).min(ARC_HEIGHT_MAX);
    let cx = mx;
    let cy = my - arc;
    let om = 1.0 - t;
    (om * om * sx + 2.0 * om * t * cx + t * t * tx,
     om * om * sy + 2.0 * om * t * cy + t * t * ty)
}

pub fn dist(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt()
}

pub fn smoothstep(p: f64) -> f64 {
    p * p * (3.0 - 2.0 * p)
}
