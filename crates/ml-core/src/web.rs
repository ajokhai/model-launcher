//! Internet access for chat models: a web search (DuckDuckGo, no account or key) and a page reader
//! that turns HTML into plain text. Models reach these through the `web` MCP server that
//! llama-server is started with, which calls /api/web/* here. Settings › Internet access turns it off.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

const AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
/// Enough of a page to answer from, small enough to fit an 8K context alongside the chat.
const PAGE_CHARS: usize = 6000;

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(AGENT).timeout(Duration::from_secs(20)).build()?)
}

/// Top results for a query, as [{title, url, snippet}].
pub async fn search(query: &str, max: usize) -> Result<Vec<Value>> {
    let query = query.trim();
    if query.is_empty() {
        bail!("empty search");
    }
    let html = client()?
        .post("https://html.duckduckgo.com/html/")
        .form(&[("q", query)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut out = Vec::new();
    for chunk in html.split("class=\"result__a\"").skip(1) {
        let Some(href) = attr(chunk, "href") else { continue };
        let url = real_url(&href);
        if url.contains("duckduckgo.com/y.js") || !url.starts_with("http") {
            continue; // ads
        }
        let title = text_of(chunk.split_once('>').map(|x| x.1).unwrap_or("").split("</a>").next().unwrap_or(""));
        let snippet = chunk
            .split_once("class=\"result__snippet\"")
            .and_then(|(_, rest)| rest.split_once('>'))
            .map(|(_, rest)| text_of(rest.split("</a>").next().unwrap_or("")))
            .unwrap_or_default();
        out.push(json!({ "title": title, "url": url, "snippet": snippet }));
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
}

/// A page's title and readable text.
pub async fn fetch(url: &str) -> Result<Value> {
    let parsed = reqwest::Url::parse(url).map_err(|_| anyhow!("not a valid link: {url}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("only http and https links can be opened");
    }
    // Pages only: nothing on this Mac or the local network.
    let host = parsed.host_str().unwrap_or("").to_lowercase();
    if host == "localhost" || host.ends_with(".local") || host.parse::<std::net::IpAddr>().is_ok_and(private_ip) {
        bail!("local addresses can't be opened");
    }
    let resp = client()?.get(parsed).send().await?.error_for_status()?;
    let final_url = resp.url().to_string();
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body = resp.text().await?;
    let (title, text) = if ctype.contains("html") || body.trim_start().starts_with('<') {
        let title = body.split_once("<title").and_then(|(_, r)| r.split_once('>')).map(|(_, r)| text_of(r.split("</title>").next().unwrap_or(""))).unwrap_or_default();
        (title, readable(&body))
    } else {
        (String::new(), body)
    };
    let truncated = text.chars().count() > PAGE_CHARS;
    let text: String = text.chars().take(PAGE_CHARS).collect();
    Ok(json!({ "url": final_url, "title": title, "text": text, "truncated": truncated }))
}

fn private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified(),
        std::net::IpAddr::V6(v) => v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00,
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let end = tag[start..].find('"')? + start;
    Some(decode_entities(&tag[start..end]))
}

/// DuckDuckGo sometimes wraps links as //duckduckgo.com/l/?uddg=<encoded>.
fn real_url(href: &str) -> String {
    if let Some(q) = href.split_once("uddg=").map(|x| x.1) {
        let enc = q.split('&').next().unwrap_or(q);
        return percent_decode(enc);
    }
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }
    href.to_string()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Strips tags from a fragment and tidies whitespace.
fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The visible text of a page: scripts, styles and page furniture removed, one line per block.
fn readable(html: &str) -> String {
    let lower = html.to_lowercase();
    // Start at the main content when the page marks it, else at <body>.
    let body_start = ["<main", "<article", "<body"].iter().find_map(|t| lower.find(t)).unwrap_or(0);
    let (html, lower) = (&html[body_start..], &lower[body_start..]);
    let mut out = String::new();
    let mut i = 0;
    const SKIP: [&str; 9] = ["script", "style", "noscript", "svg", "nav", "footer", "template", "aside", "form"];
    const BLOCK: [&str; 14] = ["p", "div", "br", "li", "h1", "h2", "h3", "h4", "tr", "section", "article", "pre", "blockquote", "table"];
    while i < html.len() {
        let Some(off) = html[i..].find('<') else {
            out.push_str(&html[i..]);
            break;
        };
        out.push_str(&html[i..i + off]);
        i += off;
        let end = match html[i..].find('>') {
            Some(e) => i + e + 1,
            None => break,
        };
        let tag = &lower[i + 1..end - 1];
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if !tag.starts_with('/') && SKIP.contains(&name.as_str()) {
            let close = format!("</{name}");
            i = lower[end..].find(&close).map(|p| end + p).and_then(|p| lower[p..].find('>').map(|q| p + q + 1)).unwrap_or(html.len());
            continue;
        }
        if BLOCK.contains(&name.as_str()) {
            out.push('\n');
        }
        i = end;
    }
    decode_entities(&out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(10)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markup() {
        assert_eq!(text_of("<b>llama.cpp&#x27;s</b> &amp; MCP"), "llama.cpp's & MCP");
        assert_eq!(real_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=x"), "https://example.com/a");
        let page = readable("<html><head><style>x{}</style></head><body><nav>menu</nav><h1>Title</h1><p>Hello <i>there</i></p><script>bad()</script></body></html>");
        assert_eq!(page, "Title\nHello there");
    }

    #[tokio::test]
    #[ignore = "uses the network"]
    async fn live() {
        let r = search("llama.cpp github", 3).await.unwrap();
        println!("{r:#?}");
        assert!(!r.is_empty());
        let p = fetch(r[0]["url"].as_str().unwrap()).await.unwrap();
        println!("{}", &p["text"].as_str().unwrap()[..400.min(p["text"].as_str().unwrap().len())]);
        assert!(fetch("http://127.0.0.1:11500/api/status").await.is_err());
    }
}
