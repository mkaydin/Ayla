use crate::models::RenderedPage;
use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::json;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

const LIGHTPANDA_WS: &str = "ws://localhost:9222";

#[derive(Debug, Clone)]
pub struct Renderer {
    http_client: Client,
    ws_url: String,
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
        // The HTTP gateway validates redirects and pins DNS. Never let CDP
        // navigate arbitrary source URLs or execute their scripts on the host.
        let plain = self.fetch_plain(url).await?;
        if !plain.html.to_ascii_lowercase().contains("<html") {
            return Ok(plain);
        }
        match self.render_guarded_html(&plain.html).await {
            Ok((title, text)) if !text.trim().is_empty() => Ok(RenderResult {
                page: RenderedPage {
                    url: url.to_string(),
                    title,
                    text_content: text,
                    status_code: plain.page.status_code,
                },
                html: plain.html,
            }),
            Ok(_) => Ok(plain),
            Err(error) => {
                debug!("Guarded CDP unavailable ({error}); using bounded HTTP extraction");
                Ok(plain)
            }
        }
    }

    async fn render_guarded_html(&self, html: &str) -> Result<(String, String)> {
        let (mut socket, _) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connect_async(&self.ws_url),
        )
        .await??;
        let mut sequence = 0u64;
        // Independent browser contexts prevent concurrent jobs from replacing each other's pages.
        let result = cdp_call(
            &mut socket,
            &mut sequence,
            None,
            "Browser.createBrowserContext",
            json!({"disposeOnDetach": true}),
        )
        .await?;
        let context = result["browserContextId"]
            .as_str()
            .context("CDP has no isolated browser contexts")?
            .to_string();
        let rendering = async {
            let target = cdp_call(&mut socket, &mut sequence, None, "Target.createTarget", json!({"url": "about:blank", "browserContextId": context})).await?;
            let target = target["targetId"].as_str().context("CDP has no target")?;
            let attached = cdp_call(&mut socket, &mut sequence, None, "Target.attachToTarget", json!({"targetId": target, "flatten": true})).await?;
            let session = attached["sessionId"].as_str().context("CDP has no session")?;
            cdp_call(&mut socket, &mut sequence, Some(session), "Network.enable", json!({})).await?;
            cdp_call(&mut socket, &mut sequence, Some(session), "Network.setBlockedURLs", json!({"urls": ["*"]})).await?;
            cdp_call(&mut socket, &mut sequence, Some(session), "Emulation.setScriptExecutionDisabled", json!({"value": true})).await?;
            let frame = cdp_call(&mut socket, &mut sequence, Some(session), "Page.getFrameTree", json!({})).await?;
            let frame = frame["frameTree"]["frame"]["id"].as_str().context("CDP has no frame")?;
            cdp_call(&mut socket, &mut sequence, Some(session), "Page.setDocumentContent", json!({"frameId": frame, "html": html})).await?;
            let extracted = cdp_call(&mut socket, &mut sequence, Some(session), "Runtime.evaluate", json!({
                "expression": "JSON.stringify({title: document.title, text: (document.querySelector('article, main, [role=main]') || document.body).innerText.substring(0, 60000)})",
                "returnByValue": true,
            })).await?;
            let value: serde_json::Value = serde_json::from_str(extracted["result"]["value"].as_str().context("CDP has no extracted value")?)?;
            Ok((value["title"].as_str().unwrap_or_default().to_string(), value["text"].as_str().unwrap_or_default().to_string()))
        }.await;
        // Disposal is awaited on success/failure. Connection closure also releases ownership
        // on cancellation; disposeOnDetach asks the server to enforce that lifetime.
        let _ = cdp_call(
            &mut socket,
            &mut sequence,
            None,
            "Browser.disposeBrowserContext",
            json!({"browserContextId": context}),
        )
        .await;
        let _ = socket.close(None).await;
        rendering
    }

    async fn fetch_plain(&self, url: &str) -> Result<RenderResult> {
        debug!("HTTP fetch: {}", url);
        let response =
            crate::public_fetch::get_public(url, std::time::Duration::from_secs(30)).await?;
        let status = response.status().as_u16();
        response.error_for_status_ref()?;
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > 768 * 1024 {
                anyhow::bail!("Research page exceeds the content limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        let html = String::from_utf8_lossy(&bytes).into_owned();
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

async fn cdp_call(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    sequence: &mut u64,
    session: Option<&str>,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    *sequence += 1;
    let id = *sequence;
    let mut command = json!({"id": id, "method": method, "params": params});
    if let Some(session) = session {
        command["sessionId"] = json!(session);
    }
    socket.send(Message::Text(command.to_string())).await?;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(message) = socket.next().await {
            if let Message::Text(text) = message? {
                if text.len() > 2 * 1024 * 1024 {
                    anyhow::bail!("CDP response exceeds the limit");
                }
                let response: serde_json::Value = serde_json::from_str(&text)?;
                if response["id"].as_u64() == Some(id) {
                    if let Some(error) = response.get("error") {
                        anyhow::bail!("CDP {method}: {error}");
                    }
                    return Ok(response["result"].clone());
                }
            }
        }
        anyhow::bail!("CDP closed before replying")
    })
    .await?
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
                let remaining: String = chars[i..].iter().take(10).collect();
                if remaining.to_lowercase().starts_with("</script>") {
                    in_script = false;
                    in_tag = true;
                }
            }
        } else if in_style {
            if chars[i] == '<' && i + 7 < chars.len() {
                let remaining: String = chars[i..].iter().take(10).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_scripts_are_removed_with_bounded_scanning() {
        let html = format!("<html><title>Fixture</title><script>{}</script><p>Public evidence survives the page extraction filter.</p></html>", "<x".repeat(100_000));
        let text = extract_content_structured(&html);
        assert!(text.contains("Public evidence"));
        assert!(!text.contains("<x"));
    }
}
