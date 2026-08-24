//! gtk4-layer-shell FFI wrapped into safe helpers.
//!
//! Layer surfaces keep our windows out of `hyprctl clients` entirely —
//! zero tiling impact, OVERLAY layer, no keyboard grab, click-through.

use gtk4::prelude::*;
use gtk4::ApplicationWindow;

#[repr(C)]
enum Layer { Overlay = 3 } // zwlr_layer_shell_v1: 0=background 1=bottom 2=top 3=overlay
#[repr(C)]
enum Edge { Left = 0, Right = 1, Top = 2, Bottom = 3 }
#[repr(C)]
enum KeyboardMode { None = 0 }

#[link(name = "gtk4-layer-shell")]
unsafe extern "C" {
    fn gtk_layer_init_for_window(window: *mut gtk4::ffi::GtkWindow);
    fn gtk_layer_set_layer(window: *mut gtk4::ffi::GtkWindow, layer: Layer);
    fn gtk_layer_set_keyboard_mode(window: *mut gtk4::ffi::GtkWindow, mode: KeyboardMode);
    fn gtk_layer_set_anchor(window: *mut gtk4::ffi::GtkWindow, edge: Edge, anchor: i32);
    fn gtk_layer_set_margin(window: *mut gtk4::ffi::GtkWindow, edge: Edge, margin: i32);
    fn gtk_layer_set_exclusive_zone(window: *mut gtk4::ffi::GtkWindow, zone: i32);
}

/// Turn a plain GtkWindow into a click-through overlay layer surface.
/// `anchor_all_edges` pins it to the full output (drawing canvas);
/// otherwise only top-left (buddy sprite, moved via margins).
pub fn setup(win: &ApplicationWindow, anchor_all_edges: bool) {
    unsafe {
        let p = win.as_ptr() as *mut gtk4::ffi::GtkWindow;
        gtk_layer_init_for_window(p);
        gtk_layer_set_layer(p, Layer::Overlay);
        gtk_layer_set_keyboard_mode(p, KeyboardMode::None);
        gtk_layer_set_anchor(p, Edge::Left, 1);
        gtk_layer_set_anchor(p, Edge::Top, 1);
        if anchor_all_edges {
            gtk_layer_set_anchor(p, Edge::Right, 1);
            gtk_layer_set_anchor(p, Edge::Bottom, 1);
        }
        // -1 = ignore other surfaces' exclusive zones (omarchy waybar reserves
        // 26px at top; without this the compositor pushes us below the bar)
        gtk_layer_set_exclusive_zone(p, -1);
    }
    // empty input region -> clicks pass through to windows below
    win.connect_realize(|w| {
        if let Some(s) = w.surface() {
            s.set_input_region(&cairo::Region::create());
        }
    });
}

/// Position a top-left-anchored surface (logical px; negative = offscreen).
pub fn set_margins(win: &ApplicationWindow, x: i32, y: i32) {
    unsafe {
        let p = win.as_ptr() as *mut gtk4::ffi::GtkWindow;
        gtk_layer_set_margin(p, Edge::Left, x);
        gtk_layer_set_margin(p, Edge::Top, y);
    }
}
