//! squeaky-agent — the sidecar brain (LLM + search registries).
//!
//! Runs separately from rust-buddy so the 16ms ink loop never blocks on
//! LLM/search calls (docs/spec.md). This binary currently exposes the
//! registries + a one-shot tool-using `ask` loop; the sessions.jsonl watcher
//! / jobs feed wiring is the next leg.
//!
//! Modes:
//!   ask "<prompt>"     one-shot agent: LLM + web_search/web_extract tools
//!   llm-test [prompt]  streaming chat through the provider ring (no tools)
//!   search-test <q>    run the search chain, print hits + winning backend
//!   extract-test <url> fetch + readability-lite extraction

mod config;
mod llm;
mod search;

use anyhow::Result;
use llm::{Msg, ToolDef};

const SYSTEM_PROMPT: &str = "\
You are squeaky, a desktop buddy's research agent. Answer concisely (a few \
sentences unless asked otherwise). You have tools:\n\
- web_search(query): find pages; use when facts may be stale or unknown\n\
- web_extract(url): read a page's text; prefer it over guessing from snippets\n\
Cite sources inline as [n] matching your web_search result order.";

#[tokio::main]
async fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    let arg = std::env::args().nth(2).unwrap_or_default();
    match mode.as_str() {
        "ask" if !arg.is_empty() => ask(&arg).await,
        "llm-test" => {
            let p = if arg.is_empty() { "Say OK".to_string() } else { arg };
            llm_test(&p).await
        }
        "search-test" if !arg.is_empty() => search_test(&arg).await,
        "extract-test" if !arg.is_empty() => extract_test(&arg).await,
        _ => {
            println!("usage: squeaky-agent <mode>");
            println!("  ask \"<prompt>\"      one-shot agent w/ web_search + web_extract");
            println!("  llm-test [prompt]   streaming chat via the provider ring");
            println!("  search-test <query> search chain end-to-end");
            println!("  extract-test <url>  readability-lite extraction");
            Ok(())
        }
    }
}

// --- one-shot agent loop ------------------------------------------------------

async fn ask(prompt: &str) -> Result<()> {
    let cfg = config::load();
    let llm = llm::LlmRegistry::new(&cfg.providers).await?;
    let srch = search::WebSearchRegistry::new(&cfg.search).await?;
    let http = reqwest::Client::builder()
        .user_agent(search::BROWSER_UA)
        .build()?;

    let tools = vec![
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
    ];

    let mut msgs = vec![Msg::system(SYSTEM_PROMPT), Msg::user(prompt)];
    for round in 0..8 {
        println!("AGENT: round {} …", round);
        let (reply, used) = llm.chat(&msgs, Some(&tools), 4096).await?;
        println!("AGENT: endpoint {}", used);
        match &reply.tool_calls.clone() {
            Some(calls) if !calls.is_empty() => {
                msgs.push(reply);
                for call in calls {
                    let out = exec_tool(&srch, &http, &call.function.name, &call.function.arguments).await;
                    println!("TOOL {}({}) → {} chars", call.function.name, call.function.arguments, out.len());
                    msgs.push(Msg::tool_result(&call.id, &call.function.name, &out));
                }
            }
            _ => {
                println!("{}", reply.content.as_deref().unwrap_or("(empty reply)"));
                return Ok(());
            }
        }
    }
    println!("AGENT: gave up after 8 rounds (model kept calling tools)");
    Ok(())
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
        other => format!("error: unknown tool '{}'", other),
    }
}

// --- test commands ------------------------------------------------------------

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
