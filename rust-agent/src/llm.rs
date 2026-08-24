//! LLM registry — OpenAI-compatible chat against a rotating provider ring.
//!
//! Zero config = opencode zen free tier (keyless, headers `x-opencode-client:
//! desktop` + UA `opencode`) with model rotation on 429/down/5xx, then local
//! ollama. Keyed providers from settings.json take precedence when their
//! keyring account resolves.
//!
//! Verified against the live endpoint (2026-08-25):
//! - SSE chunks: `data: {…}` lines, `: keep-alive` comment lines in between,
//!   `[DONE]` terminator; text arrives in `choices[0].delta.content`
//! - reasoning models also emit `delta.reasoning` and may send
//!   `content: null` — we surface content only
//! - native tool-calling works (`finish_reason: "tool_calls"`)

use crate::config::{keyring_lookup, ProviderCfg};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

// --- wire types -------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // "function"
    pub function: ToolFn,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ToolFn {
    pub name: String,
    /// JSON-encoded arguments string, per OpenAI wire format
    pub arguments: String,
}

/// One chat message. `content` is None for pure tool_call turns and for
/// assistant messages that only reasoned; tool responses carry tool_call_id.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Msg {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Msg {
    pub fn system(t: &str) -> Self { Msg { role: "system".into(), content: Some(t.into()), tool_calls: None, tool_call_id: None, name: None } }
    pub fn user(t: &str) -> Self { Msg { role: "user".into(), content: Some(t.into()), tool_calls: None, tool_call_id: None, name: None } }
    pub fn tool_result(call_id: &str, name: &str, out: &str) -> Self {
        Msg { role: "tool".into(), content: Some(out.into()), tool_calls: None, tool_call_id: Some(call_id.into()), name: Some(name.into()) }
    }
}

#[derive(Clone, Serialize)]
pub struct ToolDef {
    pub r#type: String, // "function"
    pub function: serde_json::Value, // JSON schema {name,description,parameters}
}

impl ToolDef {
    pub fn function(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        ToolDef {
            r#type: "function".into(),
            function: serde_json::json!({ "name": name, "description": description, "parameters": parameters }),
        }
    }
}

// --- registry ---------------------------------------------------------------

#[derive(Clone)]
struct Endpoint {
    provider_id: String,
    base_url: String,
    model: String,
    key: Option<String>,
}

pub struct LlmRegistry {
    http: reqwest::Client,
    ring: Mutex<Vec<Endpoint>>,
    start: AtomicUsize, // rotates past endpoints that failed recently
}

const MAX_ATTEMPTS: usize = 8; // ring spans zen(5) + ovh(2) + llm7(1) + ollama
const BACKOFF_MS: u64 = 500;

impl LlmRegistry {
    /// Build the ring: keyed providers first (stable), models flattened,
    /// ollama's model list resolved live from GET /models.
    pub async fn new(providers: &[ProviderCfg]) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        let mut ring: Vec<Endpoint> = Vec::new();
        for p in providers {
            let key = p.key_account.as_deref().and_then(keyring_lookup);
            if p.key_account.is_some() && key.is_none() {
                println!("LLM: {} has no keyring entry — skipped", p.id);
                continue;
            }
            let mut models = p.models.clone();
            if models.is_empty() && p.base_url.contains("localhost") {
                // ollama fallback: ask it what it has
                match http.get(format!("{}/models", p.base_url)).send().await {
                    Ok(r) => {
                        let v: serde_json::Value = r.json().await?;
                        for m in v["data"].as_array().cloned().unwrap_or_default() {
                            if let Some(id) = m["id"].as_str() { models.push(id.to_string()); }
                        }
                    }
                    Err(_) => {}
                }
                if models.is_empty() {
                    println!("LLM: {} unreachable / no models — skipped", p.id);
                    continue;
                }
            }
            for m in &models {
                ring.push(Endpoint {
                    provider_id: p.id.clone(),
                    base_url: p.base_url.clone(),
                    model: m.clone(),
                    key: key.clone(),
                });
            }
        }
        if ring.is_empty() {
            return Err(anyhow!("no usable LLM endpoints (keyed providers missing keys, ollama down)"));
        }
        for e in &ring {
            println!("LLM: endpoint {}:{}{}", e.provider_id, e.model, if e.key.is_some() { " (keyed)" } else { "" });
        }
        Ok(LlmRegistry { http, ring: Mutex::new(ring), start: AtomicUsize::new(0) })
    }

    async fn ordered(&self) -> Vec<Endpoint> {
        let ring = self.ring.lock().unwrap().clone();
        let s = self.start.load(Ordering::Relaxed) % ring.len().max(1);
        let (a, b) = ring.split_at(s);
        b.iter().chain(a.iter()).cloned().collect()
    }

    fn note_failure(&self) {
        self.start.fetch_add(1, Ordering::Relaxed);
    }

    fn request(&self, ep: &Endpoint, body: &serde_json::Value, stream: bool) -> reqwest::RequestBuilder {
        let mut rb = self
            .http
            .post(format!("{}/chat/completions", ep.base_url.trim_end_matches('/')))
            .header("Content-Type", "application/json")
            .header("x-opencode-client", "desktop") // zen wants this; harmless elsewhere
            .header("User-Agent", "opencode");
        if let Some(k) = &ep.key {
            rb = rb.bearer_auth(k);
        }
        if stream {
            rb = rb.header("Accept", "text/event-stream");
        }
        rb.json(body)
    }

    fn build_body(model: &str, msgs: &[Msg], tools: Option<&[ToolDef]>, max_tokens: u32, stream: bool) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": model,
            "messages": msgs,
            "max_tokens": max_tokens,
            "stream": stream,
        });
        if let Some(t) = tools {
            body["tools"] = serde_json::to_value(t).unwrap_or(serde_json::json!([]));
        }
        body
    }

    /// Non-streaming chat turn. Rotates through the ring on failure.
    /// Returns (assistant message, provider:model used).
    pub async fn chat(&self, msgs: &[Msg], tools: Option<&[ToolDef]>, max_tokens: u32) -> Result<(Msg, String)> {
        let eps = self.ordered().await;
        let attempts = eps.len().min(MAX_ATTEMPTS);
        let mut last_err = String::new();
        for ep in eps.into_iter().take(attempts) {
            let body = Self::build_body(&ep.model, msgs, tools, max_tokens, false);
            match self.request(&ep, &body, false).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let txt = resp.text().await.unwrap_or_default();
                    if !status.is_success() {
                        last_err = format!("{}:{} HTTP {} {}", ep.provider_id, ep.model, status, truncate(&txt, 200));
                        println!("LLM: {} — rotating", last_err);
                        self.note_failure();
                        tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
                        continue;
                    }
                    match serde_json::from_str::<serde_json::Value>(&txt) {
                        Ok(v) => match parse_choice(&v) {
                            Ok(m) => return Ok((m, format!("{}:{}", ep.provider_id, ep.model))),
                            Err(e) => {
                                last_err = format!("{}:{} bad payload: {}", ep.provider_id, ep.model, e);
                            }
                        },
                        Err(e) => last_err = format!("{}:{} non-json reply ({})", ep.provider_id, ep.model, e),
                    }
                }
                Err(e) => {
                    last_err = format!("{}:{} network: {}", ep.provider_id, ep.model, e);
                    println!("LLM: {} — rotating", last_err);
                    self.note_failure();
                    tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
                    continue;
                }
            }
            println!("LLM: {} — rotating", last_err);
            self.note_failure();
            tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
        }
        Err(anyhow!("all LLM endpoints failed; last: {}", last_err))
    }

    /// Streaming chat (no tools): calls `on_delta` per text chunk, returns the
    /// full text. Used by llm-test and later by the buddy bubble feed.
    pub async fn chat_stream(&self, msgs: &[Msg], max_tokens: u32, mut on_delta: impl FnMut(&str)) -> Result<String> {
        let eps = self.ordered().await;
        let attempts = eps.len().min(MAX_ATTEMPTS);
        let mut last_err = String::new();
        for ep in eps.into_iter().take(attempts) {
            let body = Self::build_body(&ep.model, msgs, None, max_tokens, true);
            let resp = self.request(&ep, &body, true).send().await;
            let resp = match resp {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    last_err = format!("{}:{} HTTP {}", ep.provider_id, ep.model, r.status());
                    println!("LLM: {} — rotating", last_err);
                    self.note_failure();
                    tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
                    continue;
                }
                Err(e) => {
                    last_err = format!("{}:{} network: {}", ep.provider_id, ep.model, e);
                    println!("LLM: {} — rotating", last_err);
                    self.note_failure();
                    tokio::time::sleep(Duration::from_millis(BACKOFF_MS)).await;
                    continue;
                }
            };
            // SSE line loop
            use futures_util::StreamExt;
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut full = String::new();
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.context("stream read error")?;
                buf.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(pos) = buf.find('\n') {
                    let line: String = buf.drain(..=pos).collect();
                    let line = line.trim_end();
                    if let Some(data) = line.strip_prefix("data:") {
                        let data = data.trim();
                        if data == "[DONE]" {
                            return Ok(full);
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                            if let Some(piece) = v["choices"][0]["delta"]["content"].as_str() {
                                if !piece.is_empty() {
                                    on_delta(piece);
                                    full.push_str(piece);
                                }
                            }
                        }
                    }
                    // ": keep-alive" comment lines are ignored
                }
            }
            return Ok(full); // stream ended without [DONE] — still fine
        }
        Err(anyhow!("all LLM endpoints failed (stream); last: {}", last_err))
    }
}

fn parse_choice(v: &serde_json::Value) -> Result<Msg> {
    let msg = &v["choices"][0]["message"];
    if msg.is_null() {
        // zen error shape: {"error":{"type":..,"message":..}}
        let emsg = v["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(anyhow!("{}", emsg));
    }
    let content = msg["content"].as_str().map(|s| s.to_string());
    let tool_calls: Option<Vec<ToolCall>> = msg["tool_calls"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|t| serde_json::from_value(t.clone()).ok()).collect());
    if content.is_none() && tool_calls.is_none() {
        return Err(anyhow!("message had neither content nor tool_calls"));
    }
    Ok(Msg { role: "assistant".into(), content, tool_calls, tool_call_id: None, name: None })
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("{}…", &s[..n]) }
}
