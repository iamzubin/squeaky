# squeaky spec

HeyClicky for Omarchy — cursor buddy, draw-to-highlight, and research agent that works with no keys.

## Goal

Hold `ctrl+alt` to draw ink that follows the cursor. Release captures a screenshot + voice transcript and sends it to an agent. The buddy signals what context is kept, runs research jobs in the background, and opens a local site when done.

## Features (from `product.md`)

- **Cursor follower** — buddy stays visible after a highlight to show context is alive. Configurable TTL (30s–10m, default 2m). Panel has `Clear context` and a countdown. On expiry context is cleared and the buddy parks off-screen.
- **Omarchy panel**
  1. Cursor color swatches
  2. `Clear context`
  3. `Open settings` (gear)
  4. Shortcuts line (`ctrl+alt` drag, `esc` clear)
  5. Agent jobs tab — running/finished jobs, `Open site` / `Show log`
  6. Long-running output — local site under `~/.local/share/squeaky/sites/<job>/` + streamed log
- **Agent mode** — user asks for research, sidecar spins a job that can use tools (search, extract), retries on 429/5xx, streams progress to the panel, writes `index.html` and `xdg-open`s it.

## Architecture

```
rust-buddy (GTK4 + layer-shell, 60fps ink, whisper-rs STT)
   -- sessions.jsonl -->
rust-agent (sidecar, tokio)  -- jobs.jsonl / status.json / context.json -->
panel (Panel.qml)
```

Sidecar is a separate binary (`squeaky-agent`) so the 16ms ink loop never blocks on LLM/search/TTS. It tails `sessions.jsonl` and drives `flight.rs`/`bubble` via `jobs.jsonl`. All config is `settings.json` + keyring; no `.env` needed for distribution.

## Main decisions

- **LLM:** `opencode-free` (`https://opencode.ai/zen/v1`, keyless, models `big-pickle`/`deepseek-v4-flash-free`/`mimo-v2.5-free`/`nemotron-3-super-free`) is the default. Local `ollama` (`http://localhost:11434/v1`) is the next fallback. Any key added in the panel (OpenAI/Anthropic/Zen/custom `base_url`) takes precedence. Free tier rotates models on 429.
- **Search:** free by default via `DuckDuckGo` (html scrape, no key) and `SearXNG` if you set a URL in the panel. Keyed `Brave`/`Tavily`/`Exa` are used only when you store a key. Auto order `tavily > exa > brave > searxng > ddgs`, failover to next on error. `search_only` providers refuse `web_extract` with a hint.
- **TTS:** local voice (`piper-rs` or `tts` + `rodio`) by default, `ElevenLabs` (`api.elevenlabs.io/v1/text-to-speech/<voice>`) when you store a key. Matches `farzaa/clicky`'s ElevenLabs path but adds a free fallback.
- **Secrets:** every key is entered in the panel and stored in the system keyring (`secret-tool` / `service=squeaky`, `account=<id>` — same pattern as `hass`). `settings.json` holds only non-secret choices (`searxng_url`, `elevenlabs_voice_id`, etc).

## How it flows

1. Pen-up → `session.rs` appends `sessions.jsonl` (bbox + points + screenshot) + voice transcript.
2. Sidecar appends a turn to `context.json`, picks the LLM/search/TTS registries, calls the agent with image+bbox+transcript+history.
3. Agent streams `web_search`/`web_extract` → writes `jobs.jsonl` progress → renders `index.html` → `xdg-open` + `tts.speak`.
4. Panel watches `status.json`/`jobs.jsonl` and shows `agent_busy`, context countdown, and jobs. Buddy flies/points via `flight.rs` when the agent returns coordinates.

## Files (additive)

- `~/.config/heyclicky/settings.json` — `cursor_color`, `buddy_gap`, `voice_enabled`, `context_ttl_secs`, `providers[]`, `search{search_backend,extract_backend,searxng_url}`, `tts{backend,voice_id}`, `research{out_dir}`. No secrets.
- `~/.local/state/heyclicky/status.json` — `listening`, `transcribing`, `agent_busy`, `context_active`, `context_expires_at_ms`.
- `~/.local/state/heyclicky/context.json` / `sessions.jsonl` / `jobs.jsonl` — history, ink sessions, job log + site path.
- `~/.local/share/squeaky/sites/<job>/index.html` — research output.

## Libraries

| Need | Choice | Why |
|---|---|---|
| LLM, multi-provider + custom `base_url` (opencode) | `rig-core 0.27` | unified `Agent` + tools + streaming |
| Async | `tokio` | required by `rig` |
| Retries | `backon` + `reqwest-retry` | exponential + jitter, 429/5xx only |
| Search | `reqwest` + `scraper` (5 providers + ring) | one dep, hermes pattern |
| TTS | `piper-rs`/`tts` + `rodio` (local) / `reqwest` (ElevenLabs) | free fallback + clicky parity |
| Watch | `notify` | file tail, not polling |

Full Rust sketches and exact JSON shapes are in `docs/NOTES.md` appendix.
