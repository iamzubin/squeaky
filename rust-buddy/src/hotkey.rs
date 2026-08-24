//! evdev ctrl+alt chord watcher.
//!
//! Reads keyboards directly from /dev/input (user must be in the `input`
//! group). Works even when a window has focus — compositor binds can't
//! interfere with a plain held chord.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Spawn the background watcher; `active` reflects the chord state.
pub fn spawn(active: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        use evdev::{Device, KeyCode as K};
        let mut opened: Vec<Device> = Vec::new();
        let mut paths: Vec<String> = std::fs::read_dir("/dev/input")
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .filter(|p| p.contains("/event"))
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        for p in &paths {
            if let Ok(d) = Device::open(p) {
                let _ = d.set_nonblocking(true);
                if d.supported_keys().map(|k| k.contains(K::KEY_LEFTCTRL)).unwrap_or(false) {
                    println!("HOTKEY: listening on {} ({})", p, d.name().unwrap_or("?"));
                    opened.push(d);
                }
            }
        }
        println!("HOTKEY: {} keyboards, hold ctrl+alt to draw", opened.len());
        let (mut ctrl, mut alt) = (false, false);
        loop {
            let mut any = false;
            for dev in opened.iter_mut() {
                if let Ok(iter) = dev.fetch_events() {
                    for ev in iter {
                        any = true;
                        if ev.event_type() != evdev::EventType::KEY { continue; }
                        let (code, val) = (ev.code(), ev.value());
                        if code == 29 || code == 97 { ctrl = val != 0; }   // KEY_LEFTCTRL / KEY_RIGHTCTRL
                        if code == 56 || code == 100 { alt = val != 0; }   // KEY_LEFTALT / KEY_RIGHTALT
                        active.store(ctrl && alt, Ordering::SeqCst);
                    }
                }
            }
            if !any { std::thread::sleep(std::time::Duration::from_millis(5)); }
        }
    });
}
