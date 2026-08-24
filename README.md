# squeaky – heyclicky for omarchy

Cursor buddy + draw-to-highlight for your agent on Omarchy (Hyprland / Wayland).

Hold `ctrl+alt`, red ink trails your cursor (decaying comet). Release → screenshot with ink → clipboard + `~/Pictures/heyclicky/` + session record in `~/.local/state/heyclicky/sessions.jsonl`. Voice (whisper-rs, Vulkan) can dictate while you draw — transcript attaches to the same session.

## Structure

```
squeaky/
  rust-buddy/        # GTK4 + gtk4-layer-shell daemon (Rust)
    src/            # state, hotkey (evdev), ink, hypr IPC, session, stt (whisper-rs), settings
    assets/         # embedded cursor.png
    Cargo.toml
  omarchy-plugin/   # Quickshell bar widget (zubin.heyclicky)
    manifest.json
    Panel.qml       # waybar button (triangle, matches bar foreground) + cursor color, gap, agent jobs
    Model.js
  docs/
    NOTES.md        # Hyprland 0.56 quirks, IPC, coordinate spaces
    product.md      # product spec
    todo.md         # roadmap
```

## Build

```bash
# deps (Arch/Omarchy): gtk4 gtk4-layer-shell grim wl-clipboard pipewire
# plus Vulkan headers for whisper GPU:
sudo pacman -S vulkan-headers

cd rust-buddy
cargo build --release
./target/release/rust-buddy            # hotkey mode (default)
./target/release/rust-buddy demo       # bezier demo
./target/release/rust-buddy stt-test /path/to.wav
```

STT reuses voxtype's GGML models from `~/.local/share/voxtype/models/` or `~/.local/share/heyclicky/models/` (no download if voxtype installed). Config hot-reloaded from `~/.config/heyclicky/settings.json`.

## Omarchy plugin

The waybar button appears only while `rust-buddy` is running:

```bash
mkdir -p ~/.config/omarchy/plugins/zubin.heyclicky
cp -r omarchy-plugin/* ~/.config/omarchy/plugins/zubin.heyclicky/
omarchy restart shell
# button sits in the bar, panel: cursor color, gap, agent jobs
```

Color, gap & voice toggle write `settings.json`; daemon hot-reloads. Visible is gated via `pgrep -x rust-buddy`.

## Omarchy specifics

See `docs/NOTES.md` for Hyprland 0.56 Lua dispatch, IPC socket quirks (`j/cursorpos` no newline + shutdown), and coordinate spaces.

License: MIT
