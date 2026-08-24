# heyclicky — TODO

## Cursor redesign (rust side) — DONE

- [x] **Buddy shape: simple triangle, slightly curved bottom** — cairo path at
      render time (main.rs draw_func), runtime recolor, no asset
- [x] **Consume `cursor_color` from settings.json** — wired into the cairo
      draw + hot-reload (queue_draw on change)

## Omarchy plugin (zubin.heyclicky) — DONE-ish

- [x] Bar widget: white triangle (bar foreground, not cursor color) +
      pulsing green agent dot (visible on `agent_busy`/`transcribing`)
- [x] Settings panel: cursor color swatches, voice toggle, buddy gap slider
      — writes `~/.config/heyclicky/settings.json`, daemon hot-reloads
- [x] Live status in hero (model, voice, transcribing) from status.json
- [ ] **Agent jobs list** — placeholder section exists ("AGENT JOBS"); needs:
  - [ ] rust: agent job lifecycle (spawn/track/finish) + write jobs feed
        (proposal: `~/.local/state/heyclicky/jobs.jsonl`, same append style
        as sessions.jsonl)
  - [x] rust: `agent_busy` field in status.json (published false until the
        agent leg lands; QML already reads it, dot no longer demos off
        transcribing)
  - [ ] qml: parse jobs feed → list rows (Model.js gets parseJobs)

## Roadmap (already discussed)

- [ ] Agent leg — **registries landed** (`rust-agent/` → `squeaky-agent`):
  - [x] LLM registry: opencode zen free ring w/ model rotation on
        429/down/5xx, ollama fallback, keyed providers via keyring,
        streaming SSE + native tool-calling (`llm.rs`)
  - [x] Search registry: ddgs/searxng/brave/tavily/exa chain, key-aware
        auto order, ring failover + `web_extract` readability-lite
        (`search.rs`); one-shot tool loop: `squeaky-agent ask "…"`
  - [ ] sessions.jsonl watcher → turn a pen session into the `ask` flow
  - [ ] jobs feed writer (~/.local/state/heyclicky/jobs.jsonl) + status
        agent_busy flips true while a job runs
- [ ] Distribution kit: PKGBUILD (`heyclicky-bin`), installer script
      (voxtype pattern), systemd user unit, .desktop + icon
- [ ] Multi-monitor: per-output overlay surfaces + capture all displays
- [ ] Response bubble: word-wrap + streaming chars for agent replies
- [x] STT: whisper-rs in-process, Vulkan GPU (0.46s/2s audio), reuses
      voxtype GGML models, [BLANK_AUDIO]-style hallucination filter
- [x] Settings hot-reload + status publishing (file contract with plugin)
