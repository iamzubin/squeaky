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

---

## Appendix: spec technical details (from spec.md)

### Architecture

```
rust-buddy (GTK, overlay, ink, STT+TTS) --sessions.jsonl--> rust-agent (sidecar, tokio)
      ^ status.json / jobs.jsonl / context.json <--------'               |
      | secret-tool (libsecret) only — no .env for dist.                  v
Panel.qml <--> settings.json (+ keyring)                          LLM (Rig) + Search + TTS -> site -> xdg-open
```

Sidecar avoids blocking the 16ms ink loop. It watches `sessions.jsonl`, runs the LLM loop on tokio, writes `jobs.jsonl`/`status.json`/`context.json`. Buddy only renders flight/bubble.

### LLM, search, TTS registries

- **LLM**: `rig-core 0.27` with `Client::builder().base_url()`. Zero-config `opencode-free` (`https://opencode.ai/zen/v1`, models `big-pickle`/`deepseek-v4-flash-free`/`mimo-v2.5-free`/`nemotron-3-super-free`, headers `x-opencode-client: desktop` + `User-Agent: opencode`), fallback `ollama` (`http://localhost:11434/v1`). Keyed providers take over when a key exists in keyring.
- **Search**: `ddgs` (DuckDuckGo, `reqwest`+`scraper` on `html.duckduckgo.com`) and `SearXNG` are no-key; `brave`/`tavily`/`exa` are keyed. Auto order `tavily > exa > brave > searxng > ddgs`, ring failover on 429.
- **TTS**: local `piper-rs`/`tts` + `rodio` by default, `elevenlabs` (`api.elevenlabs.io/v1/text-to-speech/<voice>`) when keyed.
- Retries: `backon` 0.4 or `reqwest-retry` (3 tries, 500ms, 429/5xx).

### squeaky-agent (LLM + search sidecar) — live-verified 2026-08-25

- **opencode zen** (`https://opencode.ai/zen/v1`): OpenAI-compatible,
  keyless with headers `x-opencode-client: desktop` + `User-Agent: opencode`.
  - SSE: `data: {…}` chunks separated by `: keep-alive` comment lines;
    `[DONE]` terminator. Text arrives in `choices[0].delta.content`;
    reasoning arrives separately in `delta.reasoning` (mimo/nemotron) —
    surface content only or you'll print thinking out loud.
  - `big-pickle` streams its chain-of-thought *into* `content` (no separate
    reasoning field) and can burn the whole token budget thinking; fine for
    tool turns, cap expectations on streamed answers.
  - Models go down randomly ("Model is unavailable"/"Endpoint is
    unavailable") → ring rotation across models/providers is mandatory.
    Current free ring: big-pickle, deepseek-v4-flash-free, mimo-v2.5-free,
    nemotron-3-ultra-free (+ local ollama fallback via GET /models).
  - Native tool-calling works keyless (`finish_reason: "tool_calls"`).
- **DDG scrape** (`html.duckduckgo.com/html/?q=`): needs a browser UA;
  result links are `//duckduckgo.com/l/?uddg=<percent-encoded>&rut=…` —
  unwrap `uddg=` (scraper decodes the `&amp;` entities already).
- **Director feed** (`~/.local/state/heyclicky/director.jsonl`): the
  sidecar→buddy command channel (ambient-agent state-file pattern). Events:
  `{"cmd":"fly","x":..,"y":..,"label":"click here"}` (bezier flight + label
  bubble), `{"cmd":"say","text":"…"}` (bubble at buddy, hold scales with
  length). Buddy polls mtime per tick; line-count cursor replays on
  truncation. `agent_busy` is RMW'd into status.json by the agent around
  each turn (races with buddy voice writes are possible but harmless v1).
- **Vision**: zen free models accept OpenAI `image_url` data URIs — verified
  mimo-v2.5-free / big-pickle / hy3-free; nemotron rejects images ("No
  endpoints found that support image input") and rotates past. OVH's
  anonymous Qwen2.5-VL-72B reads screen text OCR-grade.
- Keys live in keyring only: `secret-tool lookup service squeaky account
  <id>` (tavily/exa/brave backends activate when their key exists).

### File contracts (additive)



`~/.config/heyclicky/settings.json` adds `context_ttl_secs` (30..600, default 120), `providers[]`, `search{search_backend,extract_backend,searxng_url,count,keyless_fallback}`, `tts{backend,elevenlabs_voice_id,volume}`, `research{out_dir}`. Secrets are **not** in this file — they live in `secret-tool` (`service=squeaky`, `account=<id>`).

`~/.local/state/heyclicky/status.json` adds `agent_busy`, `context_active`, `context_expires_at_ms`. `context.json` keeps turns, `jobs.jsonl` keeps `{id,status,site,log_tail}`. See spec's `squeaky-agent` sketch for the `LlmRegistry`/`WebSearchRegistry`/`TtsRegistry` flow and `backon` retry loop.
