//! Config + secrets — the agent-side read of the file contract.
//!
//! - Reads ~/.config/heyclicky/settings.json (additive fields `providers[]`
//!   + `search{}`; missing file or fields -> zero-config defaults, and the
//!   buddy's fields are simply ignored).
//! - API keys never live in the file: looked up from the system keyring via
//!   `secret-tool lookup service squeaky account <id>` (same pattern as hass).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProviderCfg {
    /// display id, e.g. "zen", "ollama", "openai"
    pub id: String,
    /// OpenAI-compatible base url (…/v1)
    pub base_url: String,
    /// models to rotate through on failure
    #[serde(default)]
    pub models: Vec<String>,
    /// keyring account holding this provider's API key; None = keyless
    #[serde(default)]
    pub key_account: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SearchCfg {
    /// "auto" (key-aware chain) or a fixed backend:
    /// ddgs | searxng | brave | tavily | exa
    pub search_backend: String,
    /// "auto" = direct fetch + readability-lite extraction
    pub extract_backend: String,
    #[serde(default)]
    pub searxng_url: String,
    pub count: u32,
    /// fall through the chain when the chosen backend fails
    #[serde(default = "default_true")]
    pub keyless_fallback: bool,
}

fn default_true() -> bool {
    true
}

impl Default for SearchCfg {
    fn default() -> Self {
        SearchCfg {
            search_backend: "auto".into(),
            extract_backend: "auto".into(),
            searxng_url: String::new(),
            count: 5,
            keyless_fallback: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    /// LLM providers in preference order; keyed ones take precedence at runtime.
    #[serde(default)]
    pub providers: Vec<ProviderCfg>,
    #[serde(default)]
    pub search: SearchCfg,
}

/// Zero-config default ring: opencode zen free tier (rotates on 429/down),
/// then local ollama as fallback. Matches docs/spec.md.
impl Config {
    pub fn with_defaults() -> Self {
        Config {
            providers: vec![
                ProviderCfg {
                    id: "zen-free".into(),
                    base_url: "https://opencode.ai/zen/v1".into(),
                    // big-pickle was down upstream on 2026-08-25 — rotation is the point
                    models: vec![
                        "big-pickle".into(),
                        "deepseek-v4-flash-free".into(),
                        "mimo-v2.5-free".into(),
                        "nemotron-3-ultra-free".into(),
                    ],
                    key_account: None,
                },
                ProviderCfg {
                    id: "ollama".into(),
                    base_url: "http://localhost:11434/v1".into(),
                    models: vec![], // resolved at runtime from GET /models
                    key_account: None,
                },
            ],
            search: SearchCfg::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config/heyclicky/settings.json")
}

/// Load settings.json over the defaults. Unknown/missing fields are fine —
/// this must never hard-fail on the buddy's own keys.
pub fn load() -> Config {
    let mut cfg = Config::with_defaults();
    match std::fs::read_to_string(config_path()) {
        Ok(txt) => match serde_json::from_str::<Config>(&txt) {
            Ok(file) => {
                if !file.providers.is_empty() {
                    cfg.providers = file.providers;
                }
                cfg.search = file.search;
            }
            Err(e) => eprintln!("CONFIG: bad json in {:?} ({}), using defaults", config_path(), e),
        },
        Err(_) => {} // no settings file yet -> pure defaults
    }
    cfg
}

/// Fetch an API key from the keyring. None when unset (keyless mode).
pub fn keyring_lookup(account: &str) -> Option<String> {
    let out = Command::new("secret-tool")
        .args(["lookup", "service", "squeaky", "account", account])
        .output()
        .ok()?;
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if key.is_empty() { None } else { Some(key) }
}
