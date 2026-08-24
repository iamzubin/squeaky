//! heyclicky for Omarchy — cursor buddy + draw-to-highlight for your agent.
//!
//! Hold ctrl+alt: the buddy becomes your pen, red ink trails the cursor and
//! decays comet-style. Release: screenshot (with ink) -> file + clipboard,
//! session record -> ~/.local/state/heyclicky/sessions.jsonl.
//!
//! Modes: (none)|hotkey = draw · scribble = self-test · demo = agent mock ·
//! <secs> = plain buddy-follow.

mod flight;
mod hotkey;
mod hypr;
mod ink;
mod layershell;
mod session;
mod settings;
mod state;
mod stt;

use flight::{bezier, dist, flight_params, smoothstep, ARC_HEIGHT_FACTOR, ARC_HEIGHT_MAX, POINT_HOLD_MS};
use gtk4::prelude::*;
use gdk4 as gdk;
use gtk4::{glib, Application, ApplicationWindow, CssProvider, DrawingArea};
use ink::render_trail;
use session::{capture_for_agent, session_record, wall_ms};
use state::{Ctx, Mode, Syn, HIDDEN, HOT_X, HOT_Y, SIZE};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::env;
use std::f64::consts::PI;
use std::rc::Rc;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use cairo::{FontSlant, FontWeight};

fn main() -> glib::ExitCode {
    let arg = env::args().nth(1).unwrap_or_default();
    // stt-test <wav>: load model + transcribe a file through the real worker, then exit
    if arg == "stt-test" {
        let wav = env::args().nth(2).expect("usage: rust-buddy stt-test <file.wav>");
        return stt_test(std::path::PathBuf::from(wav));
    }
    // no args = hotkey draw mode (the actual heyclicky interaction)
    let is_hotkey = arg == "hotkey" || arg.is_empty();
    let is_demo = arg == "demo";
    let is_scribble = arg == "scribble";
    let dur_secs: u64 = if is_hotkey || is_demo || is_scribble { 0 } else { arg.parse().unwrap_or(0) };

    let app = Application::builder()
        .application_id("com.heyclicky.buddy.rs")
        .build();

    app.connect_activate(move |app| {
        let provider = CssProvider::new();
        provider.load_from_data("window { background: transparent; background-color: transparent; }");
        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(), &provider, 800);

        // --- Buddy window (triangle, click-through, color from settings) ---
        let buddy = ApplicationWindow::builder()
            .application(app)
            .default_width(SIZE)
            .default_height(SIZE)
            .build();
        buddy.set_can_target(false);
        let buddy_da = DrawingArea::new();
        buddy_da.set_content_width(SIZE);
        buddy_da.set_content_height(SIZE);
        buddy_da.set_can_target(false);
        buddy.set_child(Some(&buddy_da));
        layershell::setup(&buddy, false);
        layershell::set_margins(&buddy, HIDDEN, HIDDEN);
        buddy.present();

        // --- Fullscreen overlay for ink/flights/bubble (click-through) ---
        let overlay = ApplicationWindow::builder().application(app).build();
        overlay.set_default_size(1600, 1000);
        overlay.set_can_target(false);
        let da = DrawingArea::new();
        da.set_content_width(1600);
        da.set_content_height(1000);
        da.set_can_target(false);
        overlay.set_child(Some(&da));
        layershell::setup(&overlay, true);
        overlay.present();

        // shared state
        let hotkey_active = Arc::new(AtomicBool::new(false));
        let cfg = settings::load();
        let model_path = stt::resolve_model(&cfg.model);
        let stt_handle = model_path.map(|p| stt::spawn(p, cfg.language.clone()));
        match &stt_handle {
            Some(_) => println!("STT: voice ready (model: {})", cfg.model),
            None => println!("STT: no model found — voice disabled (draw-only mode)"),
        }
        settings::publish_status(&settings::Status {
            listening: false,
            transcribing: false,
            model: cfg.model.clone(),
            last_transcript: String::new(),
        });
        let ctx = Rc::new(RefCell::new(Ctx {
            mode: Mode::Hidden,
            buddy_pos: (0, 0),
            queue: VecDeque::new(),
            trail: Vec::new(),
            syn: None,
            pts_session: 0,
            session_start_mono: 0,
            session_start_wall: 0,
            gap_x: cfg.buddy_gap,
            gap_y: cfg.buddy_gap,
            cursor_color: cfg.cursor_color.clone(),
            voice_enabled: cfg.voice_enabled && stt_handle.is_some(),
            mic: None,
            stt: stt_handle,
            settings_mtime: settings::config_mtime(),
            last_session_id: None,
            model_name: cfg.model.clone(),
            last_transcript: String::new(),
        }));

        // buddy triangle (curved bottom), color from settings cursor_color
        {
            let ctx_c = ctx.clone();
            let buddy_da_c = buddy_da.clone();
            buddy_da.set_draw_func(move |_, cr, w, h| {
                let c = ctx_c.borrow();
                let (r, g, b) = parse_hex_color(&c.cursor_color);
                let wf = w as f64;
                let hf = h as f64;
                cr.set_source_rgba(r, g, b, 1.0);
                cr.move_to(wf * 0.16, hf * 0.08);
                cr.line_to(wf * 0.86, hf * 0.50);
                // quadratic approximated via cubic
                let qx = wf * 0.40;
                let qy = hf * 0.52;
                let ex = wf * 0.28;
                let ey = hf * 0.92;
                // current point is (0.86w,0.50h), convert quad to cubic
                let cur_x = wf * 0.86;
                let cur_y = hf * 0.50;
                let c1x = cur_x + (qx - cur_x) * 2.0 / 3.0;
                let c1y = cur_y + (qy - cur_y) * 2.0 / 3.0;
                let c2x = ex + (qx - ex) * 2.0 / 3.0;
                let c2y = ey + (qy - ey) * 2.0 / 3.0;
                cr.curve_to(c1x, c1y, c2x, c2y, ex, ey);
                cr.close_path();
                let _ = cr.fill();
            });
            // keep handle for hot-reload redraw (clone above keeps it alive)
            let _ = buddy_da_c;
        }

        // overlay drawing: ink (draw/fade), bezier curve (fly), bubble (point)
        {
            let ctx_c = ctx.clone();
            da.set_draw_func(move |_, cr, _, _| {
                let c = ctx_c.borrow();
                let now = glib::monotonic_time();
                match c.mode {
                    Mode::Hidden | Mode::Draw => {
                        // comet trail: each point fades individually, tail-first
                        render_trail(cr, &c.trail, now);
                    }
                    Mode::Fly { sx, sy, tx, ty, t0, dur, .. } => {
                        // full bezier path faint + progress head glow
                        let t = (((now - t0) as f64 / 1_000_000.0) / dur).clamp(0.0, 1.0);
                        cr.set_source_rgba(0.55, 0.36, 0.96, 0.35);
                        cr.set_line_width(2.5);
                        cr.set_line_cap(cairo::LineCap::Round);
                        let mx = (sx + tx) / 2.0;
                        let my = (sy + ty) / 2.0;
                        let arc = (dist(sx, sy, tx, ty) * ARC_HEIGHT_FACTOR).min(ARC_HEIGHT_MAX);
                        cr.move_to(sx, sy);
                        cr.curve_to(mx, my - arc, mx, my - arc, tx, ty);
                        let _ = cr.stroke();
                        let (hx, hy) = bezier(sx, sy, tx, ty, smoothstep(t));
                        let pulse = 1.0 + (t * PI).sin() * 0.3; // heyclicky scale pulse
                        cr.set_source_rgba(0.55, 0.36, 0.96, 0.30);
                        cr.arc(hx, hy, 9.0 * pulse, 0.0, 2.0 * PI);
                        let _ = cr.fill();
                        cr.set_source_rgba(0.85, 0.71, 1.0, 0.9);
                        cr.arc(hx, hy, 3.0, 0.0, 2.0 * PI);
                        let _ = cr.fill();
                    }
                    Mode::Point { x, y, until } => {
                        // bubble "right here!" (streamed in real app; static here)
                        let left = (until - now).max(0) as f64 / 1_000_000.0;
                        let a = (left / (POINT_HOLD_MS as f64 / 1_000_000.0)).clamp(0.35, 1.0);
                        let bx = x as f64 + 12.0;
                        let by = y as f64 - 26.0;
                        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Bold);
                        cr.set_font_size(11.0);
                        let label = "right here!";
                        let te = cr.text_extents(label).unwrap();
                        cr.set_source_rgba(0.55, 0.36, 0.96, 0.95 * a);
                        let rw = te.width() + 14.0;
                        let rh = te.height() + 10.0;
                        // rounded rect
                        let rad = 6.0;
                        cr.new_sub_path();
                        cr.arc(bx + rw - rad, by + rad, rad, -PI / 2.0, 0.0);
                        cr.arc(bx + rw - rad, by + rh - rad, rad, 0.0, PI / 2.0);
                        cr.arc(bx + rad, by + rh - rad, rad, PI / 2.0, PI);
                        cr.arc(bx + rad, by + rad, rad, PI, 3.0 * PI / 2.0);
                        cr.close_path();
                        let _ = cr.fill();
                        // tail
                        cr.move_to(x as f64 + 6.0, by + rh - 2.0);
                        cr.line_to(x as f64 + 10.0, by + rh + 6.0);
                        cr.line_to(x as f64 + 14.0, by + rh - 2.0);
                        cr.close_path();
                        let _ = cr.fill();
                        cr.set_source_rgba(1.0, 1.0, 1.0, a);
                        cr.move_to(bx + 7.0, by + te.height() + 5.0);
                        let _ = cr.show_text(label);
                    }
                }
            });
        }

        // redraw ticker
        {
            let ctx_c = ctx.clone();
            let da_c = da.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                let c = ctx_c.borrow();
                if c.mode != Mode::Hidden || !c.trail.is_empty() {
                    da_c.queue_draw();
                }
                glib::ControlFlow::Continue
            });
        }

        if is_hotkey {
            hotkey::spawn(hotkey_active.clone());
        }

        // --- main state-machine tick ---
        {
            let ctx_c = ctx.clone();
            let buddy_c = buddy.clone();
            let buddy_da_c2 = buddy_da.clone();
            let da_c = da.clone();
            let hk = hotkey_active.clone();
            let is_demo_c = is_demo;
            let is_scribble_c = is_scribble;
            let mut demo_started = false;
            let mut scribble_started = false;
            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                let now = glib::monotonic_time();
                let held = hk.load(Ordering::SeqCst);
                let mut c = ctx_c.borrow_mut();

                // settings hot-reload (the omarchy settings panel writes this file)
                let mtime = settings::config_mtime();
                if mtime != c.settings_mtime {
                    c.settings_mtime = mtime;
                    let s = settings::load();
                    println!(
                        "SETTINGS: reloaded (gap {}, color {}, voice {}, model {})",
                        s.buddy_gap, s.cursor_color, s.voice_enabled, s.model
                    );
                    c.gap_x = s.buddy_gap;
                    c.gap_y = s.buddy_gap;
                    c.voice_enabled = s.voice_enabled && c.stt.is_some();
                    c.model_name = s.model.clone();
                    if c.cursor_color != s.cursor_color {
                        c.cursor_color = s.cursor_color.clone();
                        buddy_da_c2.queue_draw();
                    }
                    if let (Some(h), Some(p)) = (&c.stt, stt::resolve_model(&s.model)) {
                        let _ = h.tx.send(stt::SttMsg::LoadModel(p));
                    }
                }

                // voice transcripts arriving from the whisper worker
                let texts: Vec<String> = match &c.stt {
                    Some(h) => {
                        let mut v = Vec::new();
                        while let Ok(t) = h.transcript_rx.try_recv() {
                            v.push(t);
                        }
                        v
                    }
                    None => Vec::new(),
                };
                for text in texts {
                    println!("VOICE: \"{}\"", text);
                    c.last_transcript = text.clone();
                    if let Some(sid) = &c.last_session_id {
                        session::append_event(serde_json::json!({
                            "kind": "transcript",
                            "session": sid,
                            "text": text,
                        }));
                    }
                    settings::publish_status(&settings::Status {
                        listening: false,
                        transcribing: false,
                        model: c.model_name.clone(),
                        last_transcript: c.last_transcript.clone(),
                    });
                }

                // demo autostart: fake an "agent" pointing at 3 spots
                if is_demo_c && !demo_started && now > 2_000_000 {
                    demo_started = true;
                    println!("DEMO: agent points at 3 spots");
                    if let Some((cx, cy)) = hypr::cursor_pos() {
                        c.buddy_pos = (cx + c.gap_x, cy + c.gap_y);
                        c.queue = [(800, 500), (920, 560), (700, 620)].into_iter().collect();
                        c.mode = flight_params(c.buddy_pos.0, c.buddy_pos.1, 800, 500, now, false);
                    }
                }

                match c.mode {
                    Mode::Hidden => {
                        // prune fully-decayed points
                        let life = ink::trail_life_us();
                        let before = c.trail.len();
                        c.trail.retain(|p| now - p.2 < life);
                        if c.trail.len() != before {
                            da_c.queue_draw();
                        }
                        // scribble self-test: synthetic pen, no hotkey needed
                        if is_scribble_c && !scribble_started && now > 800_000 {
                            scribble_started = true;
                            println!("SCRIBBLE: synthetic pen down at 800,480");
                            pen_down(&mut c, now, None);
                            c.syn = Some(Syn { cx: 800.0, cy: 480.0, i: 0, n: 70 });
                            c.buddy_pos = (800 + c.gap_x, 480 + c.gap_y);
                            layershell::set_margins(&buddy_c, c.buddy_pos.0, c.buddy_pos.1);
                            c.mode = Mode::Draw;
                        } else if held && !is_demo_c && !is_scribble_c {
                            if let Some((x, y)) = hypr::cursor_pos() {
                                pen_down(&mut c, now, Some((x, y)));
                                layershell::set_margins(&buddy_c, c.buddy_pos.0, c.buddy_pos.1);
                                c.mode = Mode::Draw;
                                println!("DRAW: pen down at {},{}", x, y);
                            }
                        }
                    }
                    Mode::Draw => {
                        let release;
                        let syn_now = c.syn.as_ref().map(|s| (s.cx, s.cy, s.i, s.n));
                        if let Some((scx, scy, si, sn)) = syn_now {
                            // synthetic pen: loop-de-loop scribble
                            let t = si as f64 / sn as f64;
                            let ang = t * 2.5 * PI;
                            let r = 150.0 + 45.0 * (t * 12.0).sin();
                            let px = scx + r * ang.cos();
                            let py = scy + 0.65 * r * ang.sin();
                            c.trail.push((px, py, now));
                            c.pts_session += 1;
                            c.buddy_pos = (px as i32 + c.gap_x, py as i32 + c.gap_y);
                            layershell::set_margins(&buddy_c, c.buddy_pos.0, c.buddy_pos.1);
                            c.syn.as_mut().unwrap().i = si + 1;
                            release = si + 1 >= sn;
                        } else if !held {
                            release = true;
                        } else if let Some((x, y)) = hypr::cursor_pos() {
                            c.buddy_pos = (x + c.gap_x, y + c.gap_y);
                            layershell::set_margins(&buddy_c, c.buddy_pos.0, c.buddy_pos.1);
                            let p = (x as f64, y as f64);
                            if c.trail.last().map(|q| (q.0, q.1)) != Some(p) {
                                c.trail.push((p.0, p.1, now));
                                c.pts_session += 1;
                            }
                            release = false;
                        } else {
                            release = false;
                        }
                        if release {
                            c.syn = None;
                            c.mode = Mode::Hidden;
                            layershell::set_margins(&buddy_c, HIDDEN, HIDDEN);
                            if c.pts_session >= 2 {
                                println!("DRAW: pen up ({} pts) -> capture; trail decays", c.pts_session);
                                let rec = session_record(&c.trail, c.session_start_mono, c.session_start_wall, now);
                                c.last_session_id = Some(capture_for_agent(rec));
                            } else {
                                // chord tap w/o movement: not a stroke, skip the screenshot
                                println!("DRAW: tap w/o movement ({} pt) -> ignored", c.pts_session);
                            }
                            // voice: stop capture, hand samples to the whisper worker
                            if let Some(mic) = c.mic.take() {
                                let samples = mic.stop();
                                if samples.len() >= 8000 {
                                    if let Some(h) = &c.stt {
                                        let _ = h.tx.send(stt::SttMsg::Audio(samples));
                                    }
                                }
                            }
                            c.pts_session = 0;
                        }
                    }
                    Mode::Fly { sx, sy, tx, ty, t0, dur, ret } => {
                        let p = ((now - t0) as f64 / 1_000_000.0 / dur).clamp(0.0, 1.0);
                        let t = smoothstep(p);
                        let (bx, by) = bezier(sx, sy, tx, ty, t);
                        c.buddy_pos = (bx as i32 - HOT_X, by as i32 - HOT_Y);
                        layershell::set_margins(&buddy_c, c.buddy_pos.0, c.buddy_pos.1);
                        if p >= 1.0 {
                            if ret {
                                c.mode = Mode::Hidden;
                                layershell::set_margins(&buddy_c, HIDDEN, HIDDEN);
                                println!("agent done -> hidden");
                            } else {
                                c.mode = Mode::Point { x: tx as i32, y: ty as i32, until: now + POINT_HOLD_MS * 1000 };
                                println!("POINT at {},{} 'right here!'", tx as i32, ty as i32);
                            }
                        }
                    }
                    Mode::Point { x, y, until } => {
                        if now >= until {
                            if let Some((tx, ty)) = c.queue.pop_front() {
                                let (sx, sy) = c.buddy_pos;
                                c.mode = flight_params(sx, sy, tx, ty, now, false);
                            } else {
                                // fly back to live cursor (heyclicky startFlyingBackToCursor)
                                if let Some((cx, cy)) = hypr::cursor_pos() {
                                    let (sx, sy) = c.buddy_pos;
                                    c.mode = flight_params(sx, sy, cx - HOT_X, cy - HOT_Y, now, true);
                                } else {
                                    c.mode = Mode::Hidden;
                                    layershell::set_margins(&buddy_c, HIDDEN, HIDDEN);
                                }
                            }
                        }
                        let _ = (x, y);
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        // plain buddy-only mode: just follow
        if !is_hotkey && !is_demo && !is_scribble {
            // HEYCLICKY_PIN=x,y pins the buddy at fixed margins (alignment debugging)
            let pin: Option<(i32, i32)> = env::var("HEYCLICKY_PIN").ok().and_then(|v| {
                let mut it = v.split(',');
                let x: i32 = it.next()?.trim().parse().ok()?;
                let y: i32 = it.next()?.trim().parse().ok()?;
                Some((x, y))
            });
            if let Some((px, py)) = pin {
                layershell::set_margins(&buddy, px, py); // pin = sprite top-left
            }
            // dump realized geometry so we can see what GTK actually did
            {
                let buddy_c = buddy.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(1000), move || {
                    let (sw, sh, sf) = buddy_c
                        .surface()
                        .map(|s| (s.width(), s.height(), s.scale_factor()))
                        .unwrap_or((-1, -1, -1));
                    println!(
                        "GEO: window {}x{} scale={} | surface {}x{} scale={} | alloc {:?}",
                        buddy_c.width(), buddy_c.height(), buddy_c.scale_factor(),
                        sw, sh, sf,
                        (buddy_c.allocated_width(), buddy_c.allocated_height()),
                    );
                    glib::ControlFlow::Break
                });
            }
            let ctx_c = ctx.clone();
            let buddy_c = buddy.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                let mut c = ctx_c.borrow_mut();
                if pin.is_none() && c.mode == Mode::Hidden {
                    if let Some((x, y)) = hypr::cursor_pos() {
                        c.buddy_pos = (x + c.gap_x, y + c.gap_y);
                        layershell::set_margins(&buddy_c, c.buddy_pos.0, c.buddy_pos.1);
                    }
                }
                glib::ControlFlow::Continue
            });
            if dur_secs > 0 {
                let app_weak = app.downgrade();
                glib::timeout_add_seconds_local(dur_secs as u32, move || {
                    if let Some(a) = app_weak.upgrade() { a.quit(); }
                    glib::ControlFlow::Break
                });
            }
        }

        if is_scribble {
            let app_weak = app.downgrade();
            glib::timeout_add_seconds_local(5, move || {
                println!("SCRIBBLE done");
                if let Some(a) = app_weak.upgrade() { a.quit(); }
                glib::ControlFlow::Break
            });
        } else if is_demo {
            let app_weak = app.downgrade();
            glib::timeout_add_seconds_local(12, move || {
                println!("DEMO done");
                if let Some(a) = app_weak.upgrade() { a.quit(); }
                glib::ControlFlow::Break
            });
            println!("DEMO: buddy follows; @2s agent bezier-points 3 spots w/ 'right here!' bubbles");
        } else if is_hotkey {
            println!("HOTKEY: hold ctrl+alt = draw red ink w/ your mouse (buddy is the pen)");
            println!("HOTKEY: release = screenshot w/ ink copied for your agent; ink decays comet-style as you draw");
        }
    });

    app.run_with_args::<&str>(&[])
}

fn parse_hex_color(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim().trim_start_matches('#');
    if h.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&h[0..2], 16),
            u8::from_str_radix(&h[2..4], 16),
            u8::from_str_radix(&h[4..6], 16),
        ) {
            return (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
        }
    }
    (0.65, 0.54, 0.98)
}

fn stt_test(wav: std::path::PathBuf) -> glib::ExitCode {
    let cfg = settings::load();
    let Some(model) = stt::resolve_model(&cfg.model) else {
        println!("stt-test: no model found");
        return glib::ExitCode::FAILURE;
    };
    let Ok(samples) = stt::load_wav_16k_mono(&wav) else {
        println!("stt-test: can't read {:?}", wav);
        return glib::ExitCode::FAILURE;
    };
    println!("stt-test: {} samples from {:?}", samples.len(), wav);
    let h = stt::spawn(model, cfg.language);
    h.tx.send(stt::SttMsg::Audio(samples)).unwrap();
    match h.transcript_rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(text) => {
            println!("stt-test: transcript: \"{}\"", text);
            glib::ExitCode::SUCCESS
        }
        Err(e) => {
            println!("stt-test: no transcript ({}):", e);
            glib::ExitCode::FAILURE
        }
    }
}

/// Start a fresh pen session. Each session is isolated: any leftover ink
/// from last time is wiped so a quick re-press never continues the old stroke.
fn pen_down(c: &mut Ctx, now: i64, at: Option<(i32, i32)>) {
    c.trail.clear();
    c.pts_session = 0;
    c.session_start_mono = now;
    c.session_start_wall = wall_ms();
    // real pen-down also starts voice capture (scribble's synthetic pen doesn't)
    if at.is_some() && c.voice_enabled && c.mic.is_none() {
        match stt::MicCapture::start() {
            Ok(m) => c.mic = Some(m),
            Err(e) => println!("MIC: start failed: {}", e),
        }
    }
    if let Some((x, y)) = at {
        c.trail.push((x as f64, y as f64, now));
        c.pts_session = 1;
        c.buddy_pos = (x + c.gap_x, y + c.gap_y);
    }
}
