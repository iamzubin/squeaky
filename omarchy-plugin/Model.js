// heyclicky plugin helpers: settings.json parsing.
// (a jobs-feed parser lands here when the agent leg defines its format)
.pragma library

function defaultSettings() {
  return {
    voice_enabled: true,
    model: "ggml-base.en.bin",
    language: "en",
    buddy_gap: 8,
    cursor_color: "#A78BFA"
  }
}

// Parse settings.json text into a full config (defaults filled in).
function parseSettings(text) {
  var cfg = defaultSettings()
  try {
    var parsed = JSON.parse(text)
    if (typeof parsed.cursor_color === "string") cfg.cursor_color = parsed.cursor_color
    if (typeof parsed.voice_enabled === "boolean") cfg.voice_enabled = parsed.voice_enabled
    if (typeof parsed.buddy_gap === "number") cfg.buddy_gap = Math.max(0, Math.round(parsed.buddy_gap))
    if (typeof parsed.model === "string") cfg.model = parsed.model
    if (typeof parsed.language === "string") cfg.language = parsed.language
  } catch (e) { /* corrupt/empty file -> defaults */ }
  return cfg
}
