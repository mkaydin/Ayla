use crate::models::RenderedPage;
use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::json;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};
use url::Url;

const LIGHTPANDA_WS: &str = "ws://localhost:9222";

#[derive(Debug, Clone)]
pub struct Renderer {
    http_client: Client,
    ws_url: String,
    timeout_secs: u64,
}

#[derive(Debug, Clone)]
pub struct RenderResult {
    pub page: RenderedPage,
    pub html: String,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::ACCEPT,
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                        .parse()
                        .unwrap(),
                );
                headers.insert(
                    reqwest::header::ACCEPT_LANGUAGE,
                    "en-US,en;q=0.9".parse().unwrap(),
                );
                headers
            })
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            http_client,
            ws_url: LIGHTPANDA_WS.to_string(),
            timeout_secs: 30,
        })
    }

    pub fn with_ws_url(mut self, url: String) -> Self {
        self.ws_url = url;
        self
    }

    pub async fn health_check(&self) -> Result<bool> {
        let json_url = format!(
            "{}://{}:{}/json/version",
            if self.ws_url.starts_with("wss") {
                "https"
            } else {
                "http"
            },
            self.ws_host(),
            self.ws_port()
        );
        match self.http_client.get(&json_url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    pub async fn fetch_rendered(&self, url: &str) -> Result<RenderResult> {
        let parsed = Url::parse(url).context("Invalid URL for rendering")?;

        debug!("Connecting to Lightpanda CDP at {}", self.ws_url);
        let connect_result = connect_async(&self.ws_url).await;

        match connect_result {
            Ok((ws_stream, _)) => {
                let (mut write, mut read) = ws_stream.split();

                let navigate = json!({
                    "id": 1,
                    "method": "Page.navigate",
                    "params": { "url": parsed.as_str() }
                });
                if let Err(e) = write.send(Message::Text(navigate.to_string())).await {
                    warn!("Lightpanda navigate failed: {}, using HTTP fallback", e);
                    return self.fetch_plain(url).await;
                }

                tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                let evaluate_script = r#"
                    (() => {
                        function getText(sel) {
                            const el = document.querySelector(sel);
                            return el ? el.innerText.trim() : '';
                        }
                        const title = document.title || '';
                        let text = getText('article') ||
                                   getText('[role="main"]') ||
                                   getText('main') ||
                                   getText('.main-content') ||
                                   getText('#main-content') ||
                                   getText('.paper-detail') ||
                                   getText('.article-body') ||
                                   getText('.content') ||
                                   getText('#content');
                        if (!text && document.body) text = document.body.innerText;
                        const metaDesc = getText('meta[name="description"]');
                        if (metaDesc) text = metaDesc + '\n\n' + text;
                        return JSON.stringify({ title, text: text.substring(0, 60000) });
                    })()
                "#;

                let evaluate = json!({
                    "id": 2,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": evaluate_script,
                        "returnByValue": true,
                        "awaitPromise": true
                    }
                });
                if let Err(e) = write.send(Message::Text(evaluate.to_string())).await {
                    warn!("Lightpanda evaluate failed: {}, using HTTP fallback", e);
                    return self.fetch_plain(url).await;
                }

                let mut html_content = String::new();
                let mut page_title = String::new();
                let load_timeout =
                    tokio::time::sleep(std::time::Duration::from_secs(self.timeout_secs));
                tokio::pin!(load_timeout);

                loop {
                    tokio::select! {
                        msg = read.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&text) {
                                        if let Some(error) = resp.get("error") {
                                            debug!("Lightpanda CDP notice: {}", error);
                                            continue;
                                        }
                                        if let Some(value) = resp.get("result")
                                            .and_then(|r| r.get("result"))
                                            .and_then(|r| r.get("value"))
                                            .and_then(|v| v.as_str())
                                        {
                                            if let Ok(extracted) = serde_json::from_str::<serde_json::Value>(value) {
                                                page_title = extracted["title"].as_str().unwrap_or("").to_string();
                                                html_content = extracted["text"].as_str().unwrap_or("").to_string();
                                                break;
                                            }
                                        }
                                    }
                                }
                                Some(Ok(Message::Close(_))) => break,
                                Some(Err(_)) => break,
                                None => break,
                                _ => {}
                            }
                        }
                        _ = &mut load_timeout => {
                            warn!("Lightpanda render timeout for {}", url);
                            break;
                        }
                    }
                }

                let _ = write.send(Message::Close(None)).await;

                if html_content.is_empty() {
                    return self.fetch_plain(url).await;
                }

                Ok(RenderResult {
                    page: RenderedPage {
                        url: url.to_string(),
                        title: page_title,
                        text_content: html_content,
                        status_code: 200,
                    },
                    html: String::new(),
                })
            }
            Err(e) => {
                debug!("Lightpanda unavailable ({}), using HTTP fallback", e);
                self.fetch_plain(url).await
            }
        }
    }

    async fn fetch_plain(&self, url: &str) -> Result<RenderResult> {
        debug!("HTTP fetch: {}", url);
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .context("HTTP fetch failed")?;

        let status = response.status().as_u16();
        let html = response
            .text()
            .await
            .context("Failed to read response body")?;

        let title = extract_title_plain(&html);
        let text = extract_content_structured(&html);

        Ok(RenderResult {
            page: RenderedPage {
                url: url.to_string(),
                title,
                text_content: text,
                status_code: status,
            },
            html,
        })
    }

    fn ws_host(&self) -> &str {
        self.ws_url
            .trim_start_matches("ws://")
            .trim_start_matches("wss://")
            .split(':')
            .next()
            .unwrap_or("localhost")
    }

    fn ws_port(&self) -> &str {
        self.ws_url
            .trim_start_matches("ws://")
            .trim_start_matches("wss://")
            .split(':')
            .nth(1)
            .unwrap_or("9222")
            .split('/')
            .next()
            .unwrap_or("9222")
    }
}

fn extract_title_plain(html: &str) -> String {
    if let Some(start) = html.to_lowercase().find("<title>") {
        if let Some(end) = html.to_lowercase().find("</title>") {
            return html[start + 7..end].to_string();
        }
    }
    String::new()
}

fn extract_content_structured(html: &str) -> String {
    let desc = extract_meta_description(html);

    let body_text = strip_html_tags_keep_structure(html);

    let cleaned = clean_extracted_text(&body_text, html);

    if !desc.is_empty() {
        format!("{}\n\n{}", desc, cleaned)
    } else {
        cleaned
    }
}

fn extract_meta_description(html: &str) -> String {
    let lower = html.to_lowercase();
    for needle in &[
        r#"name="description""#,
        r#"name="citation_abstract""#,
        r#"name="dc.description""#,
    ] {
        if let Some(pos) = lower.find(needle) {
            let snippet = &html[pos..];
            if let Some(content_start) = snippet.find("content=\"") {
                let after = &snippet[content_start + 9..];
                if let Some(content_end) = after.find('"') {
                    let desc = &after[..content_end];
                    if desc.len() > 30 {
                        return unescape_html(desc);
                    }
                }
            }
        }
    }
    String::new()
}

fn clean_extracted_text(body: &str, _html: &str) -> String {
    let skip_lines: &[&str] = &[
        "javascript",
        "cookie",
        "accept",
        "subscribe",
        "sign in",
        "log in",
        "login",
        "register",
        "menu",
        "navigation",
        "skip to",
        "this site needs javascript",
        "please enable javascript",
        "access denied",
        "you don't have permission",
        "click here",
        "read more",
        "search",
        "press enter",
    ];

    let mut lines: Vec<String> = body
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| {
            let lower_line = l.to_lowercase();
            if l.len() < 20 && !l.contains("abstract") && !l.contains("title") {
                return false;
            }
            for skip in skip_lines {
                if lower_line.contains(skip) {
                    return false;
                }
            }
            if l.chars().filter(|c| !c.is_alphanumeric()).count() as f64 / l.len().max(1) as f64
                > 0.5
            {
                return false;
            }
            true
        })
        .collect();

    let total = lines.len();
    lines.truncate(total.min(300));

    lines.join("\n")
}

fn strip_html_tags_keep_structure(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut in_nav = false;

    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if !in_tag && !in_script && !in_style {
            if chars[i] == '<' {
                if i + 1 < chars.len() && (chars[i + 1] == '/' || chars[i + 1].is_alphabetic()) {
                    let remaining: String = chars[i..].iter().take(20).collect();
                    let lower_rem = remaining.to_lowercase();

                    if lower_rem.starts_with("</nav>") || lower_rem.starts_with("</header>") {
                        in_nav = false;
                        in_tag = true;
                    } else if lower_rem.starts_with("<nav") || lower_rem.starts_with("<header") {
                        in_nav = true;
                        in_tag = true;
                    } else if lower_rem.starts_with("<script") {
                        in_script = true;
                        in_tag = true;
                    } else if lower_rem.starts_with("<style") {
                        in_style = true;
                        in_tag = true;
                    } else {
                        in_tag = true;
                    }
                } else {
                    in_tag = true;
                }
            } else if !in_nav {
                result.push(chars[i]);
            }
        } else if in_tag {
            if chars[i] == '>' {
                in_tag = false;
            }
        } else if in_script {
            if chars[i] == '<' && i + 8 < chars.len() {
                let remaining: String = chars[i..].iter().collect();
                if remaining.to_lowercase().starts_with("</script>") {
                    in_script = false;
                    in_tag = true;
                }
            }
        } else if in_style {
            if chars[i] == '<' && i + 7 < chars.len() {
                let remaining: String = chars[i..].iter().collect();
                if remaining.to_lowercase().starts_with("</style>") {
                    in_style = false;
                    in_tag = true;
                }
            }
        }
        i += 1;
    }

    result
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn unescape_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}
