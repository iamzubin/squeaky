//! Web search + extract registry — free by default, keyed when available.
//!
//! Auto order (spec): tavily > exa > brave > searxng > ddgs — keyed backends
//! join the front only when their keyring account resolves. Failures rotate
//! the ring (persisted for the process lifetime). DDG needs no key at all,
//! so search works out of the box.
//!
//! Verified live (2026-08-25): html.duckduckgo.com scrape returns results;
//! links come back as //duckduckgo.com/l/?uddg=<percent-encoded>&rut=…

use crate::config::{keyring_lookup, SearchCfg};
use anyhow::{anyhow, Result};
use scraper::{Html, Selector};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct WebSearchRegistry {
    http: reqwest::Client,
    chain: Mutex<Vec<Backend>>,
    start: AtomicUsize,
    count: u32,
    fixed: String,
    fallback: bool,
}

#[derive(Clone)]
enum Backend {
    Ddgs,
    Searxng(String), // instance base url
    Brave(String),   // api key
    Tavily(String),
    Exa(String),
}

impl Backend {
    fn name(&self) -> &'static str {
        match self {
            Backend::Ddgs => "ddgs",
            Backend::Searxng(_) => "searxng",
            Backend::Brave(_) => "brave",
            Backend::Tavily(_) => "tavily",
            Backend::Exa(_) => "exa",
        }
    }
}

impl WebSearchRegistry {
    /// Build the backend chain from config + keyring presence.
    pub async fn new(cfg: &SearchCfg) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(BROWSER_UA)
            .connect_timeout(Duration::from_secs(10))
            .timeout(TIMEOUT)
            .build()?;
        let mut chain = Vec::new();
        if let Some(k) = keyring_lookup("tavily") { chain.push(Backend::Tavily(k)); }
        if let Some(k) = keyring_lookup("exa") { chain.push(Backend::Exa(k)); }
        if let Some(k) = keyring_lookup("brave") { chain.push(Backend::Brave(k)); }
        if !cfg.searxng_url.is_empty() {
            chain.push(Backend::Searxng(cfg.searxng_url.trim_end_matches('/').to_string()));
        }
        chain.push(Backend::Ddgs);
        let names: Vec<&str> = chain.iter().map(|b| b.name()).collect();
        println!("SEARCH: backend chain {}", names.join(" > "));
        Ok(WebSearchRegistry {
            http,
            chain: Mutex::new(chain),
            start: AtomicUsize::new(0),
            count: cfg.count.max(1),
            fixed: cfg.search_backend.clone(),
            fallback: cfg.keyless_fallback,
        })
    }

    /// One web_search call. Fixed backend first (if set), then ring order.
    pub async fn search(&self, query: &str) -> Result<(Vec<SearchHit>, &'static str)> {
        let q = query.trim().to_string();
        if q.is_empty() {
            return Err(anyhow!("empty query"));
        }
        let chain = self.chain.lock().unwrap().clone();
        // fixed backend wins when configured; rest of the chain follows on failure
        let ordered: Vec<Backend> = match self.fixed.as_str() {
            "auto" | "" => {
                let s = self.start.load(Ordering::Relaxed) % chain.len().max(1);
                chain[s..].iter().chain(chain[..s].iter()).cloned().collect()
            }
            name => {
                let mut v: Vec<Backend> = chain.iter().filter(|b| b.name() == name).cloned().collect();
                if self.fallback {
                    v.extend(chain.iter().filter(|b| b.name() != name).cloned());
                }
                if v.is_empty() {
                    return Err(anyhow!("backend '{}' not available (no key / no url?)", name));
                }
                v
            }
        };
        let mut last_err = String::new();
        for b in &ordered {
            match self.run(b, &q).await {
                Ok(hits) if !hits.is_empty() => return Ok((hits, b.name())),
                Ok(_) => last_err = format!("{} returned 0 hits", b.name()),
                Err(e) => last_err = format!("{} failed: {}", b.name(), e),
            }
            println!("SEARCH: {} — falling through", last_err);
            self.start.fetch_add(1, Ordering::Relaxed); // demote the flaky one
        }
        Err(anyhow!("all search backends failed; last: {}", last_err))
    }

    async fn run(&self, b: &Backend, q: &str) -> Result<Vec<SearchHit>> {
        match b {
            Backend::Ddgs => self.ddgs(q).await,
            Backend::Searxng(base) => self.searxng(base, q).await,
            Backend::Brave(key) => self.brave(key, q).await,
            Backend::Tavily(key) => self.tavily(key, q).await,
            Backend::Exa(key) => self.exa(key, q).await,
        }
    }

    async fn ddgs(&self, q: &str) -> Result<Vec<SearchHit>> {
        let html = self
            .http
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", q)])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let doc = Html::parse_document(&html);
        let sel_body = Selector::parse(".result__body").unwrap();
        let sel_a = Selector::parse("a.result__a").unwrap();
        let sel_snip = Selector::parse("a.result__snippet").unwrap();
        let mut hits = Vec::new();
        for body in doc.select(&sel_body) {
            let Some(a) = body.select(&sel_a).next() else { continue };
            let title = clean(a.text().collect::<String>());
            let raw = a.value().attr("href").unwrap_or_default().to_string();
            let url = decode_ddg_href(&raw);
            let snippet = body
                .select(&sel_snip)
                .next()
                .map(|s| clean(s.text().collect::<String>()))
                .unwrap_or_default();
            if !title.is_empty() && url.starts_with("http") {
                hits.push(SearchHit { title, url, snippet });
            }
            if hits.len() >= self.count as usize { break; }
        }
        Ok(hits)
    }

    async fn searxng(&self, base: &str, q: &str) -> Result<Vec<SearchHit>> {
        let v: serde_json::Value = self
            .http
            .get(format!("{}/search", base))
            .query(&[("q", q), ("format", "json")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v["results"].as_array().cloned().unwrap_or_default().iter().take(self.count as usize).map(|r| SearchHit {
            title: r["title"].as_str().unwrap_or_default().into(),
            url: r["url"].as_str().unwrap_or_default().into(),
            snippet: r["content"].as_str().unwrap_or_default().into(),
        }).collect())
    }

    async fn brave(&self, key: &str, q: &str) -> Result<Vec<SearchHit>> {
        let v: serde_json::Value = self
            .http
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("X-Subscription-Token", key)
            .query(&[("q", q.to_string()), ("count", self.count.to_string())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v["web"]["results"].as_array().cloned().unwrap_or_default().iter().take(self.count as usize).map(|r| SearchHit {
            title: r["title"].as_str().unwrap_or_default().into(),
            url: r["url"].as_str().unwrap_or_default().into(),
            snippet: r["description"].as_str().unwrap_or_default().into(),
        }).collect())
    }

    async fn tavily(&self, key: &str, q: &str) -> Result<Vec<SearchHit>> {
        let v: serde_json::Value = self
            .http
            .post("https://api.tavily.com/search")
            .bearer_auth(key)
            .json(&serde_json::json!({ "query": q, "max_results": self.count }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v["results"].as_array().cloned().unwrap_or_default().iter().take(self.count as usize).map(|r| SearchHit {
            title: r["title"].as_str().unwrap_or_default().into(),
            url: r["url"].as_str().unwrap_or_default().into(),
            snippet: r["content"].as_str().unwrap_or_default().into(),
        }).collect())
    }

    async fn exa(&self, key: &str, q: &str) -> Result<Vec<SearchHit>> {
        let v: serde_json::Value = self
            .http
            .post("https://api.exa.ai/search")
            .header("x-api-key", key)
            .json(&serde_json::json!({
                "query": q,
                "numResults": self.count,
                "contents": { "text": { "maxCharacters": 400 } },
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v["results"].as_array().cloned().unwrap_or_default().iter().take(self.count as usize).map(|r| SearchHit {
            title: r["title"].as_str().unwrap_or_default().into(),
            url: r["url"].as_str().unwrap_or_default().into(),
            snippet: r["text"].as_str().unwrap_or_default().into(),
        }).collect())
    }
}

// --- web_extract ------------------------------------------------------------

/// Fetch a page and pull readable text out of it (readability-lite):
/// drop script/style/nav chrome, prefer <main>/<article>/<body>.
pub async fn web_extract(http: &reqwest::Client, url: &str) -> Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow!("not an http(s) url: {}", url));
    }
    let resp = http.get(url).timeout(Duration::from_secs(25)).send().await?.error_for_status()?;
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let html = resp.text().await?;
    if ctype.contains("application/json") {
        return Ok(format!("[json] {}", truncate(&html, 6000)));
    }
    let doc = Html::parse_document(&html);
    // pick the densest of main/article/body
    let mut best = String::new();
    for scope in ["main", "article", "[role=main]", "body"] {
        let sel = match Selector::parse(scope) { Ok(s) => s, Err(_) => continue };
        if let Some(el) = doc.select(&sel).next() {
            let mut out = String::new();
            // text nodes whose ancestry avoids chrome (script/style/nav/…)
            'nodes: for node in el.descendants() {
                let Some(t) = node.value().as_text() else { continue };
                let mut anc = node.parent();
                while let Some(a) = anc {
                    if let Some(ael) = a.value().as_element() {
                        if SKIP_TAGS.contains(&ael.name()) {
                            continue 'nodes;
                        }
                        if ael.name() == "br" {
                            out.push(' ');
                            break;
                        }
                    }
                    anc = a.parent();
                }
                out.push_str(t);
            }
            let t2 = collapse(&out);
            if t2.len() > best.len() { best = t2; }
        }
    }
    if best.is_empty() {
        return Err(anyhow!("no readable text at {}", url));
    }
    Ok(truncate(&best, 8000))
}

const SKIP_TAGS: &[&str] = &["script", "style", "noscript", "svg", "nav", "footer", "header", "aside", "form"];

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        let sp = ch.is_whitespace();
        if sp && prev_space { continue; }
        out.push(if sp { ' ' } else { ch });
        prev_space = sp;
    }
    out.trim().to_string()
}

fn clean(s: String) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// DDG wraps results in //duckduckgo.com/l/?uddg=<encoded>&rut=… — unwrap it.
fn decode_ddg_href(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("//duckduckgo.com/l/") {
        if let Some(start) = rest.find("uddg=") {
            let enc = &rest[start + 5..];
            let enc = enc.split('&').next().unwrap_or(enc);
            return percent_encoding::percent_decode_str(enc).decode_utf8_lossy().into_owned();
        }
    }
    if raw.starts_with("//") { format!("https:{}", raw) } else { raw.to_string() }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("{}…", &s[..n]) }
}
