# Heyclicky for Omarchy — Notes

Goal: a HeyClicky-style desktop buddy for Omarchy (Hyprland + Wayland).
Core interaction: hold `ctrl+alt`, red ink trails YOUR cursor over the screen
(decaying comet-style) to highlight something for your agent; release grabs
the screenshot + session record for the agent.

---

## Environment

- Omarchy / Arch, Hyprland **0.56.1**, Wayland, monitor `eDP-1` 2560x1600 @ **scale 1.6**
- MacBook Air (Apple SPI Keyboard = `/dev/input/event1`, SPI Trackpad)
- grim / wl-copy / wf-recorder / magick / python3 available
- User is in the `input` group -> evdev readable (hotkey watcher needs this)

## CRITICAL: Hyprland 0.56 syntax changes

- `hyprctl keyword ...` is **dead** ("can't work with non-legacy parsers. Use eval.")
- `hyprctl dispatch <legacy>` is **dead**. New syntax is Lua:
  - `hyprctl dispatch 'hl.dsp.window.close({ window = "address:0x..." })'`
  - `hyprctl eval 'hl.dispatch(hl.dsp.window.move({ x = 100, y = 200, window = "address:0x..." }))'`
- Namespaces seen: `hl.dsp.window.*`, `hl.dsp.cursor.*`, `hl.dsp.workspace.*`, `hl.dsp.group.*`
- `hl.rules` does **not** exist (no dynamic windowrules API found)

## Hyprland IPC socket quirks

- Socket: `/run/user/<uid>/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`
- Request format: **no trailing newline** + `shutdown(SHUT_WR)`, read to EOF
  - `j/cursorpos` (no \n) -> `{"x": .., "y": ..}` JSON
  - `j/cursorpos\n` -> "unknown request"
- Server closes conn after response -> one connection per request (unix
  connect is ~µs, fine at 60fps)

## Coordinate spaces

- `hyprctl cursorpos`, client `at`/`size`, layer-shell margins: **logical** px
- `grim -g "X,Y WxH"` takes **logical** coords, outputs **physical** px (x1.6 here)
- Layer-shell OVERLAY surfaces are invisible to `hyprctl clients` (zero tiling
  impact) but **ARE composited into grim captures** — that's how ink ends up
- Click-through: empty `wl_surface` input region (see `layershell.rs`)

---

## rust-buddy

GTK4 + gtk4-layer-shell Rust port. Run modes:
- (no args) / `hotkey` — **the product interaction**: hold `ctrl+alt`, buddy
  becomes the pen, ink trails your cursor; release -> screenshot w/ ink ->
  `~/Pictures/heyclicky/ink-*.png` + wl-copy clipboard
- `scribble` — synthetic-pen self-test (no keys needed), same pipeline
- `demo` — old-clicky agent mock (bezier flights + "right here!" bubbles)
- `<secs>` — plain buddy-follows-cursor for N seconds

### Layout

| file | role |
|---|---|
| `main.rs` | args, windows, draw func, state-machine tick |
| `state.rs` | Mode / Syn / Ctx + sprite consts |
| `layershell.rs` | gtk4-layer-shell FFI + click-through setup |
| `hypr.rs` | IPC: cursor_pos, focused_monitor |
| `ink.rs` | marker ink + comet-trail decay rendering |
| `session.rs` | session records + grim/wl-copy capture |
| `hotkey.rs` | evdev ctrl+alt watcher (all `/dev/input/event*`) |
| `flight.rs` | demo agent-pointing bezier math (OverlayWindow.swift port) |

### Ink trail (comet decay)

- Points sampled @16ms from `j/cursorpos`, each with birth time
- Per-point alpha: full for `TRAIL_HOLD_MS` (900), fade over `TRAIL_FADE_MS`
  (1200) -> line dissolves tail-first while drawing
- Rendered as overlapping-by-one-point bands of `TRAIL_BAND` (6) pts, band
  alpha = its oldest point -> smooth gradient, every segment painted once
- Sessions are isolated: pen-down wipes previous trail; chord tap w/o
  movement (<2 pts) captures nothing

### Session records (agent feed)

On pen-up (>=2 pts): console summary + full record appended to
`~/.local/state/heyclicky/sessions.jsonl`:
```json
{"id":"ink-<wallms>","kind":"ink_session","started_at_ms":..,"ended_at_ms":..,
 "duration_ms":..,"point_count":..,"bbox_logical":[x0,y0,x1,y1],
 "points":[{"x":..,"y":..,"t_ms":..}...],"screenshot":"~/Pictures/heyclicky/ink-*.png"}
```
- coords logical px (same space as `j/cursorpos`), `t_ms` since pen-down

### Next steps

1. Feed record + png to a real agent (local socket / CLI hook)
2. Agent-point mode (Fly/Point) driven by real agent output
3. Release build + omarchy packaging (bin script, .desktop, autostart)
