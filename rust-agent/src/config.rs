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

/// Zero-config default ring, ordered by live verification (2026-08-25):
/// 1. opencode zen free tier — keyless w/ desktop headers; mimo/big-pickle/
///    hy3 have VISION (screenshots are our core input), tools work
/// 2. OVHcloud AI Endpoints — fully anonymous (no key at all); Qwen2.5-VL-72B
///    reads screen text OCR-grade, but only ~2 RPM so it's a fallback
/// 3. LLM7.io — anonymous text-only (vision models need a pro-tier key)
/// 4. local ollama as last resort
impl Config {
    pub fn with_defaults() -> Self {
        Config {
            providers: vec![
                ProviderCfg {
                    id: "zen-free".into(),
                    base_url: "https://opencode.ai/zen/v1".into(),
                    // models go down randomly ("Endpoint is unavailable") — rotation is the point
                    models: vec![
                        "mimo-v2.5-free".into(),       // vision + clean reasoning split
                        "big-pickle".into(),           // vision (streams CoT into content!)
                        "hy3-free".into(),             // vision
                        "deepseek-v4-flash-free".into(), // text
                        "nemotron-3-ultra-free".into(),  // text (rejects images -> rotates)
                    ],
                    key_account: None,
                },
                ProviderCfg {
                    id: "ovh-anon".into(),
                    base_url: "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1".into(),
                    models: vec![
                        "Qwen2.5-VL-72B-Instruct".into(),   // vision, no auth
                        "Meta-Llama-3_3-70B-Instruct".into(),
                    ],
                    key_account: None,
                },
                ProviderCfg {
                    id: "llm7-anon".into(),
                    base_url: "https://api.llm7.io/v1".into(),
                    models: vec!["minimax-m2.7".into()], // anon tier is text-only
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
