# heyclicky — TODO

## Cursor redesign (rust side)

- [ ] **Buddy shape: simple triangle, slightly curved bottom** — like the
      reference (rounded triangle, tip up-left). Replaces the arrow PNG.
  - Draw with cairo at render time: shape = f(size, color) → runtime
    recolor for free, no asset, crisp at any scale
  - HOT_X/HOT_Y derived from shape geometry
- [ ] **Consume `cursor_color` from settings.json** — rust Settings struct
  currently ignores it (serde skips unknown fields, so the panel's writes
  round-trip safely); wire it into the cairo draw + hot-reload

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
  - [ ] rust: `agent_busy` field in status.json (dot goes live for agents,
        currently demos off transcribing)
  - [ ] qml: parse jobs feed → list rows (Model.js gets parseJobs)

## Roadmap (already discussed)

- [ ] Agent leg: transcript + annotated screenshot + bbox → LLM → streamed
      reply → buddy bubble + flight (`flight.rs` is ready, needs real input)
      — this is what creates agent jobs
- [ ] Distribution kit: PKGBUILD (`heyclicky-bin`), installer script
      (voxtype pattern), systemd user unit, .desktop + icon
- [ ] Multi-monitor: per-output overlay surfaces + capture all displays
- [ ] Response bubble: word-wrap + streaming chars for agent replies
- [x] STT: whisper-rs in-process, Vulkan GPU (0.46s/2s audio), reuses
      voxtype GGML models, [BLANK_AUDIO]-style hallucination filter
- [x] Settings hot-reload + status publishing (file contract with plugin)
