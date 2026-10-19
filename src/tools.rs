//! Small, explicitly read-only tools exposed to the model.
//!
//! Tool results are untrusted input. Callers must pass them back as tool data,
//! never as instructions to execute.

use crate::config::Config;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

const FILE_LIMIT: usize = 24 * 1024;
const TOOL_RESULT_LIMIT: usize = 12 * 1024;
const SEARCH_RESULT_COUNT: usize = 8;
const SEARCH_SCAN_FILE_LIMIT: usize = 10_000;
const SEARCH_SCAN_BYTE_LIMIT: usize = 16 * 1024 * 1024;

/// OpenAI-compatible function definitions for the currently enabled tools.
pub fn definitions(config: &Config) -> Vec<Value> {
    let mut out = Vec::new();
    if config.local_tools {
        out.extend([
            json!({"type":"function","function":{"name":"search_files","description":"Search the already indexed local files for a phrase or keyword. Returns matching file paths and short excerpts.","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}}}),
            json!({"type":"function","function":{"name":"read_file","description":"Read a small, non-hidden text file within configured search roots. Use a path returned by search_files or shown in the user's request.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}}}),
        ]);
    }
    if config.web_tools {
        out.extend([
            json!({"type":"function","function":{"name":"web_search","description":"Search the public web for current information and return a few snippets with source links.","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}}}),
            json!({"type":"function","function":{"name":"fetch_url","description":"Fetch a public HTTP or HTTPS page and return its readable text. Private and local network destinations are blocked.","parameters":{"type":"object","properties":{"url":{"type":"string"}},"required":["url"],"additionalProperties":false}}}),
        ]);
    }
    out
}

/// Execute one named, read-only tool. Unknown or disabled tools fail closed.
pub async fn call(
    name: &str,
    arguments: &Value,
    config: &Config,
    indexed_files: &[PathBuf],
) -> Result<String, String> {
    match name {
        "search_files" if config.local_tools => {
            let query = required_string(arguments, "query")?;
            Ok(search_files(query, indexed_files, &config.search_roots))
        }
        "read_file" if config.local_tools => {
            let path = required_string(arguments, "path")?;
            read_file(path, indexed_files, &config.search_roots)
        }
        "web_search" if config.web_tools => {
            let query = required_string(arguments, "query")?;
            web_search(query).await
        }
        "fetch_url" if config.web_tools => {
            let url = required_string(arguments, "url")?;
            fetch_url(url).await
        }
        _ => Err("Tool is unavailable or disabled.".to_string()),
    }
}

fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 2048)
        .ok_or_else(|| format!("Invalid `{key}` argument."))
}

fn within_roots(path: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path).map_err(|_| "File is unavailable.".to_string())?;
    // Both sides are canonical (on Windows that means the same `\\?\` verbatim
    // form), so a component-wise prefix check is reliable. Only the part below
    // the root is policed, so an explicitly configured hidden root still works.
    let relative = roots
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .find_map(|root| strip_root(&canonical, &root))
        .ok_or_else(|| "File is outside the configured search roots.".to_string())?;
    if is_forbidden_path(&relative) {
        return Err("Hidden, ignored, or sensitive files cannot be read.".to_string());
    }
    Ok(canonical)
}

/// `path` relative to `root` if it lies inside it. Windows paths compare
/// case-insensitively, like the filesystem.
fn strip_root(path: &Path, root: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let mut path_parts = path.components();
        for root_part in root.components() {
            let part = path_parts.next()?;
            let lower =
                |c: std::path::Component<'_>| c.as_os_str().to_string_lossy().to_lowercase();
            if lower(part) != lower(root_part) {
                return None;
            }
        }
        Some(path_parts.as_path().to_path_buf())
    }
    #[cfg(not(windows))]
    {
        path.strip_prefix(root).ok().map(Path::to_path_buf)
    }
}

fn resolve_requested(
    path: &str,
    roots: &[PathBuf],
    indexed_files: &[PathBuf],
) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    let mut variants = Vec::new();
    if candidate.is_absolute() {
        variants.push(candidate);
    } else {
        variants.extend(roots.iter().map(|root| root.join(&candidate)));
        // Indexers may provide a displayed relative path with a root prefix.
        variants.extend(
            indexed_files
                .iter()
                .filter(|p| p.ends_with(&candidate))
                .cloned(),
        );
    }
    for variant in variants {
        if let Ok(canonical) = within_roots(&variant, roots)
            && (indexed_files.is_empty()
                || indexed_files
                    .iter()
                    .any(|p| fs::canonicalize(p).ok().as_deref() == Some(canonical.as_path())))
        {
            return Ok(canonical);
        }
    }
    Err("File is not in the indexed files or configured search roots.".to_string())
}

fn is_forbidden_path(path: &Path) -> bool {
    use std::path::Component;
    for component in path.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        let s = part.to_string_lossy().to_lowercase();
        if s.starts_with('.')
            || matches!(
                s.as_str(),
                "node_modules" | "target" | "vendor" | "proc" | "sys" | "dev"
            )
            // Windows keeps browser profiles and app tokens under AppData.
            || cfg!(windows) && matches!(s.as_str(), "appdata" | "$recycle.bin")
        {
            return true;
        }
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    name.contains("credential")
        || name.contains("secret")
        || name.contains("password")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name == "id_rsa"
        || name == "id_ed25519"
        || matches!(ext.as_str(), "pem" | "key" | "p12" | "pfx" | "keystore")
}

fn read_file(path: &str, indexed_files: &[PathBuf], roots: &[PathBuf]) -> Result<String, String> {
    let canonical = resolve_requested(path, roots, indexed_files)?;
    let metadata = fs::metadata(&canonical).map_err(|_| "File is unavailable.".to_string())?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT as u64 {
        return Err("File is not a regular file or is larger than 24 KiB.".to_string());
    }
    let mut file = fs::File::open(&canonical).map_err(|_| "File could not be read.".to_string())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "File could not be read.".to_string())?;
    if bytes.len() > FILE_LIMIT {
        return Err("File is larger than 24 KiB.".to_string());
    }
    if bytes.contains(&0) {
        return Err("Binary files cannot be read.".to_string());
    }
    let text = String::from_utf8(bytes).map_err(|_| "File is not valid UTF-8 text.".to_string())?;
    Ok(format!(
        "File: {}\n{}",
        canonical.display(),
        truncate(&text, FILE_LIMIT)
    ))
}

fn search_files(query: &str, files: &[PathBuf], roots: &[PathBuf]) -> String {
    let terms: Vec<String> = query.split_whitespace().map(|s| s.to_lowercase()).collect();
    if terms.is_empty() {
        return "No search terms provided.".to_string();
    }
    let mut results = Vec::new();
    let mut scanned_files = 0usize;
    let mut scanned_bytes = 0usize;
    for path in files {
        if scanned_files >= SEARCH_SCAN_FILE_LIMIT || scanned_bytes >= SEARCH_SCAN_BYTE_LIMIT {
            break;
        }
        let Ok(canonical) = within_roots(path, roots) else {
            continue;
        };
        let Ok(meta) = fs::metadata(&canonical) else {
            continue;
        };
        if !meta.is_file()
            || meta.len() > 2 * 1024 * 1024
            || scanned_bytes.saturating_add(meta.len() as usize) > SEARCH_SCAN_BYTE_LIMIT
        {
            continue;
        }
        scanned_files += 1;
        let name = canonical.file_name().unwrap_or_default().to_string_lossy();
        if terms.iter().all(|term| name.to_lowercase().contains(term)) {
            results.push(format!("{}\n  [filename match]", canonical.display()));
            if results.len() >= SEARCH_RESULT_COUNT {
                break;
            }
            continue;
        }
        let Ok(mut file) = fs::File::open(&canonical) else {
            continue;
        };
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        if file
            .by_ref()
            .take((2 * 1024 * 1024 + 1) as u64)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > 2 * 1024 * 1024
        {
            continue;
        }
        scanned_bytes += bytes.len();
        if bytes.contains(&0) {
            continue;
        }
        let Ok(content) = String::from_utf8(bytes) else {
            continue;
        };
        let lower = content.to_lowercase();
        if !terms.iter().all(|term| lower.contains(term)) {
            continue;
        }
        let snippet = content
            .lines()
            .find(|line| terms.iter().all(|term| line.to_lowercase().contains(term)))
            .unwrap_or("");
        results.push(format!(
            "{}\n  {}",
            canonical.display(),
            truncate(snippet.trim(), 300)
        ));
        if results.len() >= SEARCH_RESULT_COUNT {
            break;
        }
    }
    if results.is_empty() {
        "No matching indexed files found.".to_string()
    } else {
        format!("Matching indexed files:\n{}", results.join("\n"))
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || a == 0
        || a >= 224
        || a == 100 && (64..=127).contains(&b)
        || a == 169 && b == 254
        || a == 192 && (b == 0 || b == 168 || b == 88 && c == 99)
        || a == 198 && (b == 18 || b == 19 || b == 51 && c == 100)
        || a == 203 && b == 0 && c == 113)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let [a, b, ..] = ip.segments();
    let global_unicast = a & 0xe000 == 0x2000;
    let special_2001 = a == 0x2001 && b & 0xfe00 == 0;
    let documentation_2001 = a == 0x2001 && b == 0x0db8;
    let docs_3fff = a == 0x3fff && b & 0xf000 == 0;
    !ip.is_loopback()
        && !ip.is_unspecified()
        && global_unicast
        && !special_2001
        && !documentation_2001
        && a != 0x2002
        && !docs_3fff
}

async fn validate_public_url(url: &Url) -> Result<Vec<std::net::SocketAddr>, String> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Only public HTTP(S) URLs without embedded credentials are allowed.".to_string(),
        );
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host.".to_string())?;
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
    {
        return Err("Local network URLs are blocked.".to_string());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !public_ip(ip) {
            return Err("Private or reserved network destinations are blocked.".to_string());
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| "URL has no port.".to_string())?;
        Ok(vec![std::net::SocketAddr::new(ip, port)])
    } else {
        let port = url
            .port_or_known_default()
            .ok_or_else(|| "URL has no port.".to_string())?;
        let resolved = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| "Could not resolve URL host.".to_string())?;
        let addresses: Vec<_> = resolved.map(|a| a.ip()).collect();
        if addresses.is_empty() || addresses.iter().any(|ip| !public_ip(*ip)) {
            return Err("Private or reserved network destinations are blocked.".to_string());
        }
        Ok(addresses
            .into_iter()
            .map(|ip| std::net::SocketAddr::new(ip, port))
            .collect())
    }
}

fn web_client(url: &Url, resolved: &[std::net::SocketAddr]) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Spotlight/0.1 (read-only web tool)");
    if let Some(host) = url.host_str().filter(|h| h.parse::<IpAddr>().is_err()) {
        builder = builder.resolve_to_addrs(host, resolved);
    }
    builder
        .build()
        .map_err(|e| format!("Could not create web client: {e}"))
}

async fn get_bounded(mut url: Url) -> Result<(Url, String, String), String> {
    for _ in 0..4 {
        let resolved = validate_public_url(&url).await?;
        // Pin the HTTP client to the checked DNS answers to prevent a DNS
        // rebinding between validation and connection.
        let client = web_client(&url, &resolved)?;
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;
        if response.status().is_redirection() {
            let Some(loc) = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            else {
                return Err("Redirect had no valid destination.".to_string());
            };
            url = url
                .join(loc)
                .map_err(|_| "Invalid redirect destination.".to_string())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("Web server returned HTTP {}.", response.status()));
        }
        let final_url = url;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if !(content_type.is_empty()
            || content_type.contains("text/")
            || content_type.contains("html")
            || content_type.contains("json")
            || content_type.contains("xml"))
        {
            return Err("This tool only reads text pages.".to_string());
        }
        if response.content_length().is_some_and(|n| n > 1024 * 1024) {
            return Err("Web page exceeds the 1 MiB limit.".to_string());
        }
        let response = response;
        let mut bytes = Vec::new();
        let mut body_stream = response.bytes_stream();
        while let Some(chunk) = body_stream.next().await {
            let chunk = chunk.map_err(|e| format!("Could not read response: {e}"))?;
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err("Web page exceeds the 1 MiB limit.".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        return Ok((final_url, content_type, body));
    }
    Err("Too many redirects.".to_string())
}

async fn web_search(query: &str) -> Result<String, String> {
    let mut url = Url::parse("https://html.duckduckgo.com/html/").map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("q", query);
    let (_, _, html) = get_bounded(url).await?;
    let document = scraper::Html::parse_document(&html);
    let selector = scraper::Selector::parse(".result")
        .map_err(|_| "Could not parse search results.".to_string())?;
    let title_sel = scraper::Selector::parse(".result__title a").unwrap();
    let snippet_sel = scraper::Selector::parse(".result__snippet").unwrap();
    let mut rows = Vec::new();
    for result in document.select(&selector).take(6) {
        let title = result
            .select(&title_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let href = result
            .select(&title_sel)
            .next()
            .and_then(|e| e.value().attr("href"))
            .unwrap_or("");
        let snippet = result
            .select(&snippet_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if !title.is_empty() && !href.is_empty() {
            rows.push(format!("{title}\n{href}\n{}", truncate(&snippet, 500)));
        }
    }
    if rows.is_empty() {
        Ok("No results were returned by the public search page.".to_string())
    } else {
        Ok(truncate(&rows.join("\n\n"), TOOL_RESULT_LIMIT))
    }
}

async fn fetch_url(raw: &str) -> Result<String, String> {
    let url = Url::parse(raw).map_err(|_| "Invalid URL.".to_string())?;
    let (final_url, content_type, body) = get_bounded(url).await?;
    let text = if content_type.contains("html") || body.to_ascii_lowercase().contains("<html") {
        let body = regex::Regex::new(r"(?is)<script\b[^>]*>.*?</script\s*>")
            .unwrap()
            .replace_all(&body, " ");
        let body = regex::Regex::new(r"(?is)<style\b[^>]*>.*?</style\s*>")
            .unwrap()
            .replace_all(&body, " ");
        let body = regex::Regex::new(r"(?is)<noscript\b[^>]*>.*?</noscript\s*>")
            .unwrap()
            .replace_all(&body, " ");
        let document = scraper::Html::parse_document(&body);
        let selector = scraper::Selector::parse("body").unwrap();
        document
            .select(&selector)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default()
    } else {
        body
    };
    Ok(format!(
        "Source: {final_url}\n{}",
        truncate(text.trim(), TOOL_RESULT_LIMIT)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_and_local_destinations() {
        assert!(!public_v4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!public_v4(Ipv4Addr::new(10, 1, 2, 3)));
        assert!(!public_v4(Ipv4Addr::new(169, 254, 1, 2)));
        for reserved in [
            [192, 0, 2, 1],
            [192, 88, 99, 1],
            [198, 51, 100, 2],
            [203, 0, 113, 3],
        ] {
            assert!(!public_v4(Ipv4Addr::from(reserved)));
        }
        assert!(public_v4(Ipv4Addr::new(1, 1, 1, 1)));
        assert!(!public_v6(Ipv6Addr::LOCALHOST));
        assert!(!public_v6("2001:db8::1".parse().unwrap()));
        assert!(!public_v6("2001:20::1".parse().unwrap()));
        assert!(!public_v6("2002::1".parse().unwrap()));
        assert!(!public_v6("3fff::1".parse().unwrap()));
        assert!(public_v6("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn rejects_hidden_and_secret_paths() {
        assert!(is_forbidden_path(Path::new("/tmp/project/.env")));
        assert!(is_forbidden_path(Path::new(
            "/tmp/project/client-secret.txt"
        )));
        assert!(!is_forbidden_path(Path::new("/tmp/project/src/main.rs")));
    }

    #[test]
    fn root_prefix_is_component_wise() {
        let root = std::env::temp_dir().join("spotlight-root");
        let inside = root.join("docs").join("a.txt");
        let sibling = std::env::temp_dir()
            .join("spotlight-root-other")
            .join("a.txt");
        assert_eq!(
            strip_root(&inside, &root).unwrap(),
            Path::new("docs").join("a.txt")
        );
        assert!(strip_root(&sibling, &root).is_none());
        #[cfg(windows)]
        assert!(strip_root(Path::new(r"C:\USERS\Me\x.txt"), Path::new(r"c:\users\me")).is_some());
    }

    #[test]
    fn file_read_is_limited_to_indexed_safe_files_inside_roots() {
        let root = tempfile::Builder::new()
            .prefix("spotlight-project-")
            .tempdir()
            .unwrap();
        let safe = root.path().join("notes.txt");
        let secret = root.path().join(".env");
        std::fs::write(&safe, "hello from the project").unwrap();
        std::fs::write(&secret, "SECRET=do-not-read").unwrap();
        let roots = vec![root.path().to_path_buf()];
        let indexed = vec![safe.clone(), secret.clone()];
        assert!(
            read_file(safe.to_str().unwrap(), &indexed, &roots)
                .unwrap()
                .contains("hello from the project")
        );
        assert!(read_file(secret.to_str().unwrap(), &indexed, &roots).is_err());
        assert!(read_file("missing.txt", &indexed, &roots).is_err());
    }
}
