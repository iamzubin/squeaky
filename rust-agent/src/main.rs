//! squeaky-agent — the sidecar brain (LLM + search + screen direction).
//!
//! Runs separately from rust-buddy so the 16ms ink loop never blocks on
//! LLM/search calls (docs/spec.md). After each pen session lands in
//! sessions.jsonl, `handle-session` sends the annotated screenshot + bbox +
//! voice transcript to a vision model that can either guide the user on
//! screen (point_at/say -> director.jsonl -> buddy flies) or research via
//! web_search/web_extract — searching whenever its own knowledge is unsure.
//!
//! Modes:
//!   ask "<prompt>"          one-shot agent (tools incl. screen pointing)
//!   handle-session [id]     react to a pen session (default: latest)
//!   llm-test [prompt]       streaming chat through the provider ring
//!   search-test <q>         search chain end-to-end
//!   extract-test <url>      readability-lite extraction

mod config;
mod director;
mod llm;
mod search;

use anyhow::{anyhow, Result};
use base64::Engine;
use llm::{Msg, ToolDef};
use std::path::PathBuf;

const ASK_SYSTEM_PROMPT: &str = "\
You are squeaky, a desktop buddy's research agent on Omarchy/Linux. Answer \
concisely (a few sentences unless asked otherwise). You have tools:\n\
- web_search(query): find pages; use when facts may be stale or unknown\n\
- web_extract(url): read a page's text; prefer it over guessing from snippets\n\
- point_at(x,y,label): fly the user's cursor buddy to logical-screen x,y \
holding a short label bubble — use to give click/draw guidance\n\
- say(text): show a short status bubble near the buddy\n\
Cite sources inline as [n] matching your web_search result order.";

const SESSION_SYSTEM_PROMPT: &str = "\
You are squeaky, a cursor-buddy agent on Omarchy (Hyprland). The user just \
drew ink over their screen around something and released ctrl+alt. You are \
given: the annotated screenshot, the ink bbox in logical pixels (the same \
coordinate space as the capture), and any dictated transcript.\n\
Decide what they want:\n\
- Guidance about UI they circled: call point_at(x,y,\"click here\") or a \
short imperative label at the right spot, and/or say(text) for one-line help.\n\
- Facts you're unsure about or anything researchy: use web_search / \
web_extract before answering.\n\
Finish with a short final text answer (a few sentences max) — it shows in \
their panel. Use tools as often as needed; coordinates must be logical px.";

#[tokio::main]
async fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    let arg = std::env::args().nth(2).unwrap_or_default();
    match mode.as_str() {
        "ask" if !arg.is_empty() => ask(&arg).await,
        "handle-session" => handle_session(if arg.is_empty() { None } else { Some(arg) }).await,
        "watch" => watch().await,
        "llm-test" => {
            let p = if arg.is_empty() { "Say OK".to_string() } else { arg };
            llm_test(&p).await
        }
        "search-test" if !arg.is_empty() => search_test(&arg).await,
        "extract-test" if !arg.is_empty() => extract_test(&arg).await,
        _ => {
            println!("usage: squeaky-agent <mode>");
            println!("  ask \"<prompt>\"       one-shot agent w/ search + screen pointing");
            println!("  handle-session [id]  react to latest (or given) pen session");
            println!("  watch                daemon: auto-react to every pen release");
            println!("  llm-test [prompt]    streaming chat via the provider ring");
            println!("  search-test <query>  search chain end-to-end");
            println!("  extract-test <url>   readability-lite extraction");
            Ok(())
        }
    }
}

// --- tool loop ----------------------------------------------------------------

async fn run_tool_loop(
    llm: &llm::LlmRegistry,
    srch: &search::WebSearchRegistry,
    http: &reqwest::Client,
    tools: &[ToolDef],
    mut msgs: Vec<Msg>,
) -> Result<String> {
    for round in 0..8 {
        println!("AGENT: round {} …", round);
        let (reply, used) = llm.chat(&msgs, Some(tools), 4096).await?;
        println!("AGENT: endpoint {}", used);
        let calls = reply.tool_calls.clone().filter(|c| !c.is_empty());
        match calls {
            Some(calls) => {
                msgs.push(reply);
                for call in &calls {
                    let out = exec_tool(srch, http, &call.function.name, &call.function.arguments).await;
                    println!("TOOL {}({}) → {} chars", call.function.name, call.function.arguments, out.len());
                    msgs.push(Msg::tool_result(&call.id, &call.function.name, &out));
                }
            }
            None => {
                let text = reply.content.as_str().unwrap_or("(empty reply)").to_string();
                println!("{}", text);
                return Ok(text);
            }
        }
    }
    Err(anyhow!("agent gave up after 8 rounds (model kept calling tools)"))
}

fn screen_tools() -> Vec<ToolDef> {
    vec![
        ToolDef::function(
            "web_search",
            "Search the web. Returns numbered results (title | url | snippet).",
            serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
            }),
        ),
        ToolDef::function(
            "web_extract",
            "Fetch a web page and return its readable text.",
            serde_json::json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"],
            }),
        ),
        ToolDef::function(
            "point_at",
            "Fly the user's cursor buddy to logical-pixel x,y where it holds a \
             short label bubble (e.g. \"click here\"). Coordinates are logical px.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "integer" },
                    "label": { "type": "string", "maxLength": 60 },
                },
                "required": ["x", "y"],
            }),
        ),
        ToolDef::function(
            "say",
            "Show a short status bubble near the buddy without moving it.",
            serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string", "maxLength": 120 } },
                "required": ["text"],
            }),
        ),
    ]
}

async fn exec_tool(
    srch: &search::WebSearchRegistry,
    http: &reqwest::Client,
    name: &str,
    args_json: &str,
) -> String {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or(serde_json::json!({}));
    match name {
        "web_search" => {
            let q = args["query"].as_str().unwrap_or_default().to_string();
            match srch.search(&q).await {
                Ok((hits, backend)) => {
                    let mut out = format!("[via {}]\n", backend);
                    for (i, h) in hits.iter().enumerate() {
                        out.push_str(&format!("[{}] {} | {} | {}\n", i + 1, h.title, h.url, h.snippet));
                    }
                    out
                }
                Err(e) => format!("error: {}", e),
            }
        }
        "web_extract" => {
            let url = args["url"].as_str().unwrap_or_default().to_string();
            match search::web_extract(http, &url).await {
                Ok(txt) => txt,
                Err(e) => format!("error: {}", e),
            }
        }
        "point_at" => {
            let (Some(x), Some(y)) = (args["x"].as_i64(), args["y"].as_i64()) else {
                return "error: need integer x,y".into();
            };
            let label = args["label"].as_str().unwrap_or("look here");
            match director::fly(x as i32, y as i32, label) {
                Ok(_) => format!("ok: buddy flying to {},{} holding \"{}\"", x, y, label),
                Err(e) => format!("error: {}", e),
            }
        }
        "say" => {
            let text = args["text"].as_str().unwrap_or_default();
            match director::say(text) {
                Ok(_) => "ok".into(),
                Err(e) => format!("error: {}", e),
            }
        }
        other => format!("error: unknown tool '{}'", other),
    }
}

// --- pen-session flow -------------------------------------------------------------

/// Daemon mode: tail sessions.jsonl and run a turn per new ink_session.
/// Startup marks the current newest session as seen (no replay of history).
async fn watch() -> Result<()> {
    let last = load_session(None).map(|(s, _)| s["id"].as_str().unwrap_or("").to_string());
    let mut last_id = match &last {
        Ok(id) if !id.is_empty() => {
            println!("WATCH: tailing sessions.jsonl (current head {})", id);
            id.clone()
        }
        _ => {
            println!("WATCH: tailing sessions.jsonl (no sessions yet)");
            String::new()
        }
    };
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let Ok((session, _)) = load_session(None) else { continue };
        let id = session["id"].as_str().unwrap_or_default().to_string();
        if id.is_empty() || id == last_id {
            continue;
        }
        println!("WATCH: new pen session {} — reacting", id);
        last_id = id.clone();
        if let Err(e) = handle_session(Some(id)).await {
            eprintln!("WATCH: turn failed: {}", e);
        }
    }
}

/// Latest (or requested) ink_session record + its transcript events.
fn load_session(id: Option<&str>) -> Result<(serde_json::Value, Vec<String>)> {
    use std::collections::HashMap;
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let p = PathBuf::from(home).join(".local/state/heyclicky/sessions.jsonl");
    let txt = std::fs::read_to_string(&p).map_err(|_| anyhow!("no sessions.jsonl yet — draw something first"))?;
    let mut sessions: Vec<serde_json::Value> = Vec::new();
    let mut transcripts: HashMap<String, Vec<String>> = HashMap::new();
    for line in txt.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match v["kind"].as_str() {
            Some("ink_session") => sessions.push(v),
            Some("transcript") => transcripts
                .entry(v["session"].as_str().unwrap_or("").to_string())
                .or_default()
                .push(v["text"].as_str().unwrap_or_default().to_string()),
            _ => {}
        }
    }
    let session = match id {
        Some(id) => sessions.into_iter().find(|s| s["id"].as_str() == Some(id))
            .ok_or_else(|| anyhow!("session {} not found", id))?,
        None => sessions.pop().ok_or_else(|| anyhow!("no ink sessions recorded yet"))?,
    };
    let t = transcripts.remove(session["id"].as_str().unwrap_or("")).unwrap_or_default();
    Ok((session, t))
}

/// Attach the screenshot as a data URI when present and reasonably sized.
fn screenshot_data_uri(session: &serde_json::Value) -> Option<String> {
    let path = session["screenshot"].as_str()?;
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 6_000_000 {
        println!("SESSION: screenshot {} too large to attach ({} bytes)", path, meta.len());
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Read-modify-write agent_busy into the shared status.json (buddy owns the
/// rest of the fields).
fn set_agent_busy(busy: bool) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let p = PathBuf::from(home).join(".local/state/heyclicky/status.json");
    let mut v: serde_json::Value = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::json!({}));
    v["agent_busy"] = serde_json::Value::Bool(busy);
    if let Ok(json) = serde_json::to_string(&v) {
        let _ = std::fs::write(&p, json + "\n");
    }
}

async fn handle_session(id: Option<String>) -> Result<()> {
    let (session, transcripts) = load_session(id.as_deref())?;
    let sid = session["id"].as_str().unwrap_or("?").to_string();
    let bbox = session["bbox_logical"].clone();
    let shot_uri = screenshot_data_uri(&session);

    set_agent_busy(true);
    let result = run_session_turn(&session, &bbox, &transcripts, shot_uri).await;
    set_agent_busy(false);
    let reply = result?;

    // durable record of what the agent did with this session
    config::append_session_event(&serde_json::json!({
        "kind": "agent_reply",
        "session": sid,
        "text": reply,
    }));
    Ok(())
}

async fn run_session_turn(
    session: &serde_json::Value,
    bbox: &serde_json::Value,
    transcripts: &[String],
    shot_uri: Option<String>,
) -> Result<String> {
    let cfg = config::load();
    let llm = llm::LlmRegistry::new(&cfg.providers).await?;
    let srch = search::WebSearchRegistry::new(&cfg.search).await?;
    let http = reqwest::Client::builder().user_agent(search::BROWSER_UA).build()?;

    let sid = session["id"].as_str().unwrap_or("?");
    let transcript_line = if transcripts.is_empty() {
        "(no voice transcript)".to_string()
    } else {
        format!("Voice transcript: {:?}", transcripts.join(" / "))
    };
    let text = format!(
        "Ink session {}. The user circled region bbox_logical={} then released. {}. \
What do they want? Guide them on screen if that helps; search if unsure.",
        sid, bbox, transcript_line
    );

    let user_msg = match &shot_uri {
        Some(uri) => Msg::user_vision(&text, uri),
        None => Msg::user(&format!("{}\n(screenshot unavailable)", text)),
    };

    director::say("thinking…")?;
    run_tool_loop(&llm, &srch, &http, &screen_tools(), vec![Msg::system(SESSION_SYSTEM_PROMPT), user_msg]).await
}

// --- simple modes ---------------------------------------------------------------

async fn ask(prompt: &str) -> Result<()> {
    let cfg = config::load();
    let llm = llm::LlmRegistry::new(&cfg.providers).await?;
    let srch = search::WebSearchRegistry::new(&cfg.search).await?;
    let http = reqwest::Client::builder().user_agent(search::BROWSER_UA).build()?;
    run_tool_loop(
        &llm, &srch, &http,
        &screen_tools(),
        vec![Msg::system(ASK_SYSTEM_PROMPT), Msg::user(prompt)],
    ).await?;
    Ok(())
}

async fn llm_test(prompt: &str) -> Result<()> {
    let cfg = config::load();
    let llm = llm::LlmRegistry::new(&cfg.providers).await?;
    print!("STREAM: ");
    let full = llm
        .chat_stream(&[Msg::user(prompt)], 1024, |d| {
            print!("{}", d);
            use std::io::Write;
            let _ = std::io::stdout().flush();
        })
        .await?;
    println!("\nSTREAM: done ({} chars)", full.len());
    Ok(())
}

async fn search_test(q: &str) -> Result<()> {
    let cfg = config::load();
    let srch = search::WebSearchRegistry::new(&cfg.search).await?;
    let (hits, backend) = srch.search(q).await?;
    println!("SEARCH: {} hits via {}", hits.len(), backend);
    for (i, h) in hits.iter().enumerate() {
        println!("  [{}] {}\n      {}\n      {}", i + 1, h.title, h.url, h.snippet);
    }
    Ok(())
}

async fn extract_test(url: &str) -> Result<()> {
    let http = reqwest::Client::builder().user_agent(search::BROWSER_UA).build()?;
    let txt = search::web_extract(&http, url).await?;
    println!("EXTRACT: {} chars\n{}", txt.len(), txt);
    Ok(())
}
