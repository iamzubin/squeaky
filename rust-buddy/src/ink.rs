//! Marker ink + comet-trail decay rendering, colored to match the buddy
//! (cursor_color from settings).
//!
//! The core heyclicky move: hold hotkey, ink trails your cursor, decaying
//! tail-first while you draw. Every point ages individually: full alpha for
//! TRAIL_HOLD_MS, then fades over TRAIL_FADE_MS.

use std::f64::consts::PI;

/// Fallback handled by main.rs parse_hex_color (matches default cursor_color).
pub const INK_W: f64 = 5.0; // core stroke width (logical px)

pub const TRAIL_HOLD_MS: i64 = 900;  // point stays full alpha this long
pub const TRAIL_FADE_MS: i64 = 1200; // then fades to nothing over this long
pub const TRAIL_BAND: usize = 6;     // points per alpha band (smooth gradient, cheap)

/// Total point lifetime in us (past this, prune from the trail).
pub fn trail_life_us() -> i64 {
    (TRAIL_HOLD_MS + TRAIL_FADE_MS) * 1000
}

// quadratic bezier segment (cairo only has cubics; current point = start)
fn quad_curve(cr: &cairo::Context, q: (f64, f64), end: (f64, f64)) {
    let cur = cr.current_point().unwrap_or((q.0, q.1));
    let c1 = (cur.0 + (q.0 - cur.0) * 2.0 / 3.0, cur.1 + (q.1 - cur.1) * 2.0 / 3.0);
    let c2 = (end.0 + (q.0 - end.0) * 2.0 / 3.0, end.1 + (q.1 - end.1) * 2.0 / 3.0);
    cr.curve_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
}

// smooth path through midpoint-averaged quadratic segments
fn ink_path(cr: &cairo::Context, pts: &[(f64, f64)]) {
    cr.new_path();
    cr.move_to(pts[0].0, pts[0].1);
    for i in 1..pts.len() - 1 {
        let mx = (pts[i].0 + pts[i + 1].0) / 2.0;
        let my = (pts[i].1 + pts[i + 1].1) / 2.0;
        quad_curve(cr, pts[i], (mx, my));
    }
    let last = pts[pts.len() - 1];
    cr.line_to(last.0, last.1);
}

fn draw_ink(cr: &cairo::Context, pts: &[(f64, f64)], alpha: f64, col: (f64, f64, f64)) {
    if alpha <= 0.0 || pts.is_empty() {
        return;
    }
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    if pts.len() == 1 {
        // click w/o move -> dot
        let (x, y) = pts[0];
        cr.set_source_rgba(col.0, col.1, col.2, 0.25 * alpha);
        cr.arc(x, y, INK_W * 1.6, 0.0, 2.0 * PI);
        let _ = cr.fill();
        cr.set_source_rgba(col.0, col.1, col.2, 0.92 * alpha);
        cr.arc(x, y, INK_W / 2.0, 0.0, 2.0 * PI);
        let _ = cr.fill();
        return;
    }
    // glow pass
    ink_path(cr, pts);
    cr.set_source_rgba(col.0, col.1, col.2, 0.25 * alpha);
    cr.set_line_width(INK_W * 2.2);
    let _ = cr.stroke();
    // core pass
    ink_path(cr, pts);
    cr.set_source_rgba(col.0, col.1, col.2, 0.92 * alpha);
    cr.set_line_width(INK_W);
    let _ = cr.stroke();
}

// per-point decay: full alpha while young, then fades tail-first
fn point_alpha(now: i64, t_birth: i64) -> f64 {
    let age = now - t_birth;
    let hold = TRAIL_HOLD_MS * 1000;
    let fade = TRAIL_FADE_MS * 1000;
    if age <= hold {
        1.0
    } else {
        (1.0 - (age - hold) as f64 / fade as f64).clamp(0.0, 1.0)
    }
}

/// Render the trail as overlapping-by-one-point bands, each at its tail's
/// alpha -> smooth tail-first gradient, every segment painted exactly once.
/// Color follows the buddy (cursor_color) so ink and sprite always match.
pub fn render_trail(cr: &cairo::Context, trail: &[(f64, f64, i64)], now: i64, col: (f64, f64, f64)) {
    if trail.is_empty() {
        return;
    }
    if trail.len() == 1 {
        let p = &trail[0];
        draw_ink(cr, &[(p.0, p.1)], point_alpha(now, p.2), col);
        return;
    }
    let mut i = 0;
    while i < trail.len() - 1 {
        let e = (i + TRAIL_BAND + 1).min(trail.len());
        let a = point_alpha(now, trail[i].2);
        if a > 0.0 {
            let pts: Vec<(f64, f64)> = trail[i..e].iter().map(|p| (p.0, p.1)).collect();
            draw_ink(cr, &pts, a, col);
        }
        i = e - 1;
    }
}
