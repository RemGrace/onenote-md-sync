use std::net::IpAddr;
use std::time::Duration;

use scraper::{ElementRef, Html, Selector};
use tracing::{debug, info, warn};

use crate::error::ConvertError;
use crate::model::document::{Document, Element, ListItem, Metadata, Page, RichText, TextSegment};
use crate::model::options::{ConvertOptions, ImageMode};

/// Tags that should be stripped during content extraction (non-content chrome).
const STRIP_TAGS: &[&str] = &[
    "nav", "footer", "header", "aside", "script", "style", "noscript",
];

/// Selectors tried in order to locate the main content node.
const CONTENT_SELECTORS: &[&str] = &["article", "main", "[role=\"main\"]"];

/// Maximum HTML response size: 50 MB.
const MAX_HTML_SIZE: usize = 50 * 1024 * 1024;

/// Maximum image response size: 10 MB.
const MAX_IMAGE_SIZE: usize = 10 * 1024 * 1024;

/// Maximum recursion depth for DOM walking.
const MAX_RECURSION_DEPTH: usize = 100;

pub struct WebConverter;

// ── URL validation (SSRF protection) ────────────────────────────────────────

fn validate_url(url: &str) -> Result<(), ConvertError> {
    // Only allow http and https schemes.
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ConvertError::NetworkError(format!(
            "URL scheme not allowed: {url}"
        )));
    }

    // Extract host from URL.
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    let host_port = without_scheme.split('/').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        // IPv6 bracket notation: [::1]:8080
        host_port
            .strip_prefix('[')
            .and_then(|h| h.split(']').next())
            .unwrap_or(host_port)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };

    // Block localhost.
    if host.eq_ignore_ascii_case("localhost") {
        return Err(ConvertError::NetworkError(
            "Requests to localhost are blocked".to_string(),
        ));
    }

    // Check if host is an IP address and block private/loopback ranges.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            return Err(ConvertError::NetworkError(format!(
                "Requests to private/loopback IP {ip} are blocked"
            )));
        }
    }

    Ok(())
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 127.0.0.0/8
            octets[0] == 127
            // 10.0.0.0/8
            || octets[0] == 10
            // 172.16.0.0/12
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            // 192.168.0.0/16
            || (octets[0] == 192 && octets[1] == 168)
            // 169.254.0.0/16
            || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            // ::1
            v6.is_loopback()
            // fc00::/7
            || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

// ── Shared HTTP client builder ──────────────────────────────────────────────

fn build_http_client() -> Result<reqwest::blocking::Client, ConvertError> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| ConvertError::NetworkError(format!("Failed to build HTTP client: {e}")))
}

impl WebConverter {
    /// Fetch a URL and convert its HTML content into a [`Document`].
    pub fn convert_url(url: &str, options: &ConvertOptions) -> Result<Document, ConvertError> {
        info!("Fetching URL: {}", url);

        validate_url(url)?;

        let client = build_http_client()?;
        let resp = client
            .get(url)
            .send()
            .map_err(|e| ConvertError::NetworkError(format!("HTTP request failed: {e}")))?;

        let bytes = resp.bytes().map_err(|e| {
            ConvertError::NetworkError(format!("Failed to read response body: {e}"))
        })?;

        if bytes.len() > MAX_HTML_SIZE {
            return Err(ConvertError::NetworkError(format!(
                "Response too large: {} bytes (max {})",
                bytes.len(),
                MAX_HTML_SIZE
            )));
        }

        let html_text = String::from_utf8_lossy(&bytes).into_owned();

        debug!("Fetched {} bytes of HTML", html_text.len());

        Self::convert_html(&html_text, Some(url), options)
    }

    /// Convert an HTML string into a [`Document`]. Exposed for testability.
    pub fn convert_html(
        html_text: &str,
        base_url: Option<&str>,
        options: &ConvertOptions,
    ) -> Result<Document, ConvertError> {
        let document = Html::parse_document(html_text);

        let metadata = Self::extract_metadata(&document);
        debug!("Extracted metadata: title={:?}", metadata.title);

        let content_root = Self::find_content_root(&document);

        let mut elements = Vec::new();
        match content_root {
            Some(root) => {
                debug!("Found content root element: {}", root.value().name());
                Self::walk_element(root, &mut elements, base_url, options, 0);
            }
            None => {
                debug!("No content root found, walking entire body");
                let body_sel = Selector::parse("body").unwrap();
                if let Some(body) = document.select(&body_sel).next() {
                    Self::walk_element(body, &mut elements, base_url, options, 0);
                }
            }
        }

        info!("Converted {} elements from HTML", elements.len());

        Ok(Document {
            metadata,
            pages: vec![Page { elements }],
        })
    }

    // ── Metadata extraction ─────────────────────────────────────────────

    fn extract_metadata(document: &Html) -> Metadata {
        let title = Selector::parse("title")
            .ok()
            .and_then(|sel| document.select(&sel).next())
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty());

        let author = Self::meta_content(document, "author");

        let date = Self::meta_content(document, "date")
            .or_else(|| Self::meta_content(document, "article:published_time"))
            .or_else(|| {
                Selector::parse("time[datetime]")
                    .ok()
                    .and_then(|sel| document.select(&sel).next())
                    .and_then(|el| el.value().attr("datetime").map(String::from))
            });

        Metadata {
            title,
            author,
            date,
        }
    }

    fn meta_content(document: &Html, name: &str) -> Option<String> {
        // Try name attribute first, then property attribute (for Open Graph tags).
        for attr in &["name", "property"] {
            let selector_str = format!("meta[{attr}=\"{name}\"]");
            let sel = match Selector::parse(&selector_str) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if let Some(el) = document.select(&sel).next() {
                if let Some(content) = el.value().attr("content") {
                    let trimmed = content.trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
            }
        }
        None
    }

    // ── Content root detection ("reader mode") ──────────────────────────

    fn find_content_root(document: &Html) -> Option<ElementRef<'_>> {
        // Try well-known semantic selectors first.
        for sel_str in CONTENT_SELECTORS {
            if let Ok(sel) = Selector::parse(sel_str) {
                if let Some(el) = document.select(&sel).next() {
                    return Some(el);
                }
            }
        }

        // Fallback: pick the <div> with the most text content.
        debug!("Falling back to largest-div heuristic for content root");
        let div_sel = Selector::parse("div").unwrap();
        document
            .select(&div_sel)
            .max_by_key(|el| el.text().collect::<String>().len())
    }

    // ── DOM walking ─────────────────────────────────────────────────────

    fn walk_element(
        element: ElementRef<'_>,
        elements: &mut Vec<Element>,
        base_url: Option<&str>,
        options: &ConvertOptions,
        depth: usize,
    ) {
        if depth >= MAX_RECURSION_DEPTH {
            return;
        }

        for child in element.children() {
            if let Some(el) = ElementRef::wrap(child) {
                let tag = el.value().name();

                // Skip stripped tags.
                if STRIP_TAGS.contains(&tag) {
                    continue;
                }

                match tag {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let level = tag[1..].parse::<u8>().unwrap_or(1);
                        let text = el.text().collect::<String>().trim().to_string();
                        if !text.is_empty() {
                            elements.push(Element::Heading { level, text });
                        }
                    }
                    "p" => {
                        let rich = Self::parse_inline(el, base_url);
                        if !rich.segments.is_empty() {
                            elements.push(Element::Paragraph { text: rich });
                        }
                    }
                    "ul" => {
                        let items = Self::parse_list_items(el, base_url);
                        if !items.is_empty() {
                            elements.push(Element::List {
                                ordered: false,
                                items,
                            });
                        }
                    }
                    "ol" => {
                        let items = Self::parse_list_items(el, base_url);
                        if !items.is_empty() {
                            elements.push(Element::List {
                                ordered: true,
                                items,
                            });
                        }
                    }
                    "table" => {
                        if let Some(table_el) = Self::parse_table(el) {
                            elements.push(table_el);
                        }
                    }
                    "pre" => {
                        let (language, code) = Self::parse_code_block(el);
                        if !code.is_empty() {
                            elements.push(Element::CodeBlock { language, code });
                        }
                    }
                    "blockquote" => {
                        let rich = Self::parse_inline(el, base_url);
                        if !rich.segments.is_empty() {
                            elements.push(Element::BlockQuote { text: rich });
                        }
                    }
                    "hr" => {
                        elements.push(Element::HorizontalRule);
                    }
                    "img" => {
                        if let Some(img_el) = Self::parse_image(el, base_url, options) {
                            elements.push(img_el);
                        }
                    }
                    // For wrapper divs, sections, etc., recurse deeper.
                    _ => {
                        Self::walk_element(el, elements, base_url, options, depth + 1);
                    }
                }
            }
        }
    }

    // ── Inline text parsing ─────────────────────────────────────────────

    fn parse_inline(element: ElementRef<'_>, base_url: Option<&str>) -> RichText {
        let mut segments = Vec::new();
        Self::collect_inline_segments(
            element,
            &mut segments,
            false,
            false,
            false,
            None,
            base_url,
            0,
        );

        // Trim leading/trailing whitespace from the overall RichText.
        if let Some(first) = segments.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = segments.last_mut() {
            last.text = last.text.trim_end().to_string();
        }

        // Remove empty segments.
        segments.retain(|s| !s.text.is_empty());

        RichText { segments }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_inline_segments(
        element: ElementRef<'_>,
        segments: &mut Vec<TextSegment>,
        bold: bool,
        italic: bool,
        code: bool,
        link: Option<&str>,
        base_url: Option<&str>,
        depth: usize,
    ) {
        if depth >= MAX_RECURSION_DEPTH {
            return;
        }

        for child in element.children() {
            match child.value() {
                scraper::node::Node::Text(text) => {
                    let t = text.to_string();
                    if !t.is_empty() {
                        segments.push(TextSegment {
                            text: t,
                            bold,
                            italic,
                            code,
                            link: link.map(String::from),
                        });
                    }
                }
                scraper::node::Node::Element(_) => {
                    if let Some(el) = ElementRef::wrap(child) {
                        let tag = el.value().name();
                        let (b, i, c) = match tag {
                            "strong" | "b" => (true, italic, code),
                            "em" | "i" => (bold, true, code),
                            "code" => (bold, italic, true),
                            _ => (bold, italic, code),
                        };
                        let l = if tag == "a" {
                            el.value()
                                .attr("href")
                                .map(|href| Self::resolve_url(href, base_url))
                        } else {
                            link.map(String::from)
                        };
                        Self::collect_inline_segments(
                            el,
                            segments,
                            b,
                            i,
                            c,
                            l.as_deref(),
                            base_url,
                            depth + 1,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // ── List parsing ────────────────────────────────────────────────────

    fn parse_list_items(list_element: ElementRef<'_>, base_url: Option<&str>) -> Vec<ListItem> {
        let mut items = Vec::new();

        // Only direct child <li> elements.
        for child in list_element.children() {
            if let Some(li) = ElementRef::wrap(child) {
                if li.value().name() != "li" {
                    continue;
                }
                let text = Self::parse_inline(li, base_url);

                // Look for nested <ul>/<ol> that are direct children of this <li>.
                let mut children = Vec::new();
                for li_child in li.children() {
                    if let Some(nested) = ElementRef::wrap(li_child) {
                        let nested_tag = nested.value().name();
                        if nested_tag == "ul" || nested_tag == "ol" {
                            children.extend(Self::parse_list_items(nested, base_url));
                        }
                    }
                }

                items.push(ListItem { text, children });
            }
        }

        items
    }

    // ── Table parsing ───────────────────────────────────────────────────

    fn parse_table(table_element: ElementRef<'_>) -> Option<Element> {
        let th_sel = Selector::parse("th").unwrap();
        let tr_sel = Selector::parse("tr").unwrap();
        let td_sel = Selector::parse("td").unwrap();

        let headers: Vec<String> = table_element
            .select(&th_sel)
            .map(|th| th.text().collect::<String>().trim().to_string())
            .collect();

        let mut rows: Vec<Vec<String>> = Vec::new();
        for tr in table_element.select(&tr_sel) {
            let cells: Vec<String> = tr
                .select(&td_sel)
                .map(|td| td.text().collect::<String>().trim().to_string())
                .collect();
            if !cells.is_empty() {
                rows.push(cells);
            }
        }

        if headers.is_empty() && rows.is_empty() {
            return None;
        }

        Some(Element::Table { headers, rows })
    }

    // ── Code block parsing ──────────────────────────────────────────────

    fn parse_code_block(pre_element: ElementRef<'_>) -> (Option<String>, String) {
        let code_sel = Selector::parse("code").unwrap();
        let (language, code_text) = if let Some(code_el) = pre_element.select(&code_sel).next() {
            let lang = code_el.value().attr("class").and_then(|cls| {
                cls.split_whitespace()
                    .find(|c| c.starts_with("language-") || c.starts_with("lang-"))
                    .map(|c| {
                        c.trim_start_matches("language-")
                            .trim_start_matches("lang-")
                            .to_string()
                    })
            });
            let text = code_el.text().collect::<String>();
            (lang, text)
        } else {
            (None, pre_element.text().collect::<String>())
        };

        (language, code_text.trim().to_string())
    }

    // ── Image parsing ───────────────────────────────────────────────────

    fn parse_image(
        img_element: ElementRef<'_>,
        base_url: Option<&str>,
        options: &ConvertOptions,
    ) -> Option<Element> {
        let src = img_element.value().attr("src")?;
        let alt = img_element
            .value()
            .attr("alt")
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty());

        let resolved = Self::resolve_url(src, base_url);

        match options.image_mode {
            ImageMode::Extract | ImageMode::Inline => {
                debug!("Downloading image: {}", resolved);

                if let Err(e) = validate_url(&resolved) {
                    warn!("Image URL validation failed for {}: {}", resolved, e);
                    return None;
                }

                let client = match build_http_client() {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("Failed to build HTTP client for image {}: {}", resolved, e);
                        return None;
                    }
                };

                match client.get(&resolved).send() {
                    Ok(resp) => match resp.bytes() {
                        Ok(bytes) => {
                            if bytes.len() > MAX_IMAGE_SIZE {
                                warn!(
                                    "Image too large ({} bytes, max {}): {}",
                                    bytes.len(),
                                    MAX_IMAGE_SIZE,
                                    resolved
                                );
                                return None;
                            }
                            Some(Element::Image {
                                data: bytes.to_vec(),
                                alt,
                            })
                        }
                        Err(e) => {
                            warn!("Failed to read image bytes from {}: {}", resolved, e);
                            None
                        }
                    },
                    Err(e) => {
                        warn!("Failed to download image {}: {}", resolved, e);
                        None
                    }
                }
            }
        }
    }

    // ── URL helpers ─────────────────────────────────────────────────────

    fn resolve_url(href: &str, base_url: Option<&str>) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            return href.to_string();
        }
        if href.starts_with("//") {
            return format!("https:{href}");
        }

        if let Some(base) = base_url {
            if href.starts_with('/') {
                // Absolute path — combine with origin.
                if let Some(origin) = Self::extract_origin(base) {
                    return format!("{origin}{href}");
                }
            } else {
                // Relative path — combine with base directory.
                // If base has no path component (e.g. "https://example.com"),
                // treat it as "https://example.com/" so relative paths resolve correctly.
                let base_with_slash = if !base
                    .strip_prefix("https://")
                    .or_else(|| base.strip_prefix("http://"))
                    .unwrap_or("")
                    .contains('/')
                {
                    format!("{base}/")
                } else {
                    base.to_string()
                };
                let base_dir = base_with_slash
                    .rfind('/')
                    .map(|i| &base_with_slash[..=i])
                    .unwrap_or(&base_with_slash);
                return format!("{base_dir}{href}");
            }
        }

        href.to_string()
    }

    fn extract_origin(url: &str) -> Option<String> {
        // e.g. "https://example.com/path/page" -> "https://example.com"
        let without_scheme = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let scheme = if url.starts_with("https") {
            "https"
        } else {
            "http"
        };
        let host = without_scheme.split('/').next()?;
        Some(format!("{scheme}://{host}"))
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::options::{ConvertOptions, ImageMode, PageMode};
    use std::path::PathBuf;

    fn default_options() -> ConvertOptions {
        ConvertOptions {
            image_mode: ImageMode::Inline,
            page_mode: PageMode::SingleFile,
            image_output_dir: PathBuf::from("images"),
        }
    }

    #[test]
    fn test_metadata_title() {
        let html = "<html><head><title>My Page Title</title></head><body></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        assert_eq!(doc.metadata.title.as_deref(), Some("My Page Title"));
    }

    #[test]
    fn test_metadata_author() {
        let html =
            r#"<html><head><meta name="author" content="Jane Doe"></head><body></body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        assert_eq!(doc.metadata.author.as_deref(), Some("Jane Doe"));
    }

    #[test]
    fn test_metadata_date_from_meta() {
        let html =
            r#"<html><head><meta name="date" content="2025-01-15"></head><body></body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        assert_eq!(doc.metadata.date.as_deref(), Some("2025-01-15"));
    }

    #[test]
    fn test_metadata_date_from_time() {
        let html = r#"<html><body><time datetime="2025-06-01">June 1</time></body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        assert_eq!(doc.metadata.date.as_deref(), Some("2025-06-01"));
    }

    #[test]
    fn test_heading_extraction() {
        let html = "<html><body><h1>Title</h1><h2>Subtitle</h2><h3>Section</h3></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        assert!(matches!(&elements[0], Element::Heading { level: 1, text } if text == "Title"));
        assert!(matches!(&elements[1], Element::Heading { level: 2, text } if text == "Subtitle"));
        assert!(matches!(&elements[2], Element::Heading { level: 3, text } if text == "Section"));
    }

    #[test]
    fn test_paragraph_plain_text() {
        let html = "<html><body><p>Hello world</p></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::Paragraph { text } => {
                assert_eq!(text.segments.len(), 1);
                assert_eq!(text.segments[0].text, "Hello world");
                assert!(!text.segments[0].bold);
                assert!(!text.segments[0].italic);
            }
            other => panic!("Expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_paragraph_inline_formatting() {
        let html =
            "<html><body><p>Normal <strong>bold</strong> <em>italic</em> <code>code</code></p></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::Paragraph { text } => {
                let bold_seg = text.segments.iter().find(|s| s.text.trim() == "bold");
                assert!(bold_seg.is_some());
                assert!(bold_seg.unwrap().bold);

                let italic_seg = text.segments.iter().find(|s| s.text.trim() == "italic");
                assert!(italic_seg.is_some());
                assert!(italic_seg.unwrap().italic);

                let code_seg = text.segments.iter().find(|s| s.text.trim() == "code");
                assert!(code_seg.is_some());
                assert!(code_seg.unwrap().code);
            }
            other => panic!("Expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_link_extraction() {
        let html =
            r#"<html><body><p><a href="https://example.com">Click here</a></p></body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::Paragraph { text } => {
                assert_eq!(text.segments[0].text, "Click here");
                assert_eq!(
                    text.segments[0].link.as_deref(),
                    Some("https://example.com")
                );
            }
            other => panic!("Expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_unordered_list() {
        let html = "<html><body><ul><li>One</li><li>Two</li></ul></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
            }
            other => panic!("Expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_ordered_list() {
        let html = "<html><body><ol><li>First</li><li>Second</li></ol></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::List { ordered, items } => {
                assert!(ordered);
                assert_eq!(items.len(), 2);
            }
            other => panic!("Expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_table() {
        let html = r#"<html><body>
            <table>
                <tr><th>Name</th><th>Age</th></tr>
                <tr><td>Alice</td><td>30</td></tr>
                <tr><td>Bob</td><td>25</td></tr>
            </table>
        </body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::Table { headers, rows } => {
                assert_eq!(headers, &["Name", "Age"]);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], &["Alice", "30"]);
            }
            other => panic!("Expected Table, got {other:?}"),
        }
    }

    #[test]
    fn test_code_block() {
        let html = r#"<html><body><pre><code class="language-rust">fn main() {}</code></pre></body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::CodeBlock { language, code } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(code, "fn main() {}");
            }
            other => panic!("Expected CodeBlock, got {other:?}"),
        }
    }

    #[test]
    fn test_blockquote() {
        let html = "<html><body><blockquote>A wise quote</blockquote></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::BlockQuote { text } => {
                assert_eq!(text.segments[0].text.trim(), "A wise quote");
            }
            other => panic!("Expected BlockQuote, got {other:?}"),
        }
    }

    #[test]
    fn test_horizontal_rule() {
        let html = "<html><body><hr></body></html>";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        assert!(matches!(&elements[0], Element::HorizontalRule));
    }

    #[test]
    fn test_content_root_article() {
        let html = r#"<html><body>
            <nav>Navigation</nav>
            <article>
                <h1>Article Title</h1>
                <p>Article body</p>
            </article>
            <footer>Footer</footer>
        </body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        // nav and footer should be stripped; only article content should remain.
        assert!(elements.iter().all(|e| match e {
            Element::Heading { text, .. } => text != "Navigation" && text != "Footer",
            _ => true,
        }));

        assert!(elements
            .iter()
            .any(|e| matches!(e, Element::Heading { text, .. } if text == "Article Title")));
    }

    #[test]
    fn test_strips_script_and_style() {
        let html = r#"<html><body>
            <script>alert('hi');</script>
            <style>body { color: red; }</style>
            <p>Visible content</p>
        </body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        assert_eq!(elements.len(), 1);
        assert!(matches!(&elements[0], Element::Paragraph { .. }));
    }

    #[test]
    fn test_resolve_url_absolute() {
        assert_eq!(
            WebConverter::resolve_url("https://example.com/img.png", None),
            "https://example.com/img.png"
        );
    }

    #[test]
    fn test_resolve_url_relative_path() {
        let resolved =
            WebConverter::resolve_url("/images/photo.jpg", Some("https://example.com/page"));
        assert_eq!(resolved, "https://example.com/images/photo.jpg");
    }

    #[test]
    fn test_resolve_url_protocol_relative() {
        assert_eq!(
            WebConverter::resolve_url("//cdn.example.com/a.js", None),
            "https://cdn.example.com/a.js"
        );
    }

    #[test]
    fn test_empty_html_no_panic() {
        let html = "";
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        assert!(doc.pages[0].elements.is_empty());
    }

    // ── New tests for SSRF validation ───────────────────────────────────

    #[test]
    fn test_validate_url_blocks_localhost() {
        assert!(validate_url("http://localhost/secret").is_err());
    }

    #[test]
    fn test_validate_url_blocks_private_ip() {
        assert!(validate_url("http://127.0.0.1/secret").is_err());
        assert!(validate_url("http://10.0.0.1/secret").is_err());
        assert!(validate_url("http://172.16.0.1/secret").is_err());
        assert!(validate_url("http://192.168.1.1/secret").is_err());
        assert!(validate_url("http://169.254.1.1/secret").is_err());
    }

    #[test]
    fn test_validate_url_blocks_bad_scheme() {
        assert!(validate_url("ftp://example.com/file").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_url_allows_public() {
        assert!(validate_url("https://example.com/page").is_ok());
        assert!(validate_url("http://example.com/page").is_ok());
    }

    // ── Test for relative URL resolution with no path ───────────────────

    #[test]
    fn test_resolve_url_relative_no_path() {
        let resolved = WebConverter::resolve_url("image.png", Some("https://example.com"));
        assert_eq!(resolved, "https://example.com/image.png");
    }

    // ── Test for nested list direct children only ───────────────────────

    #[test]
    fn test_nested_list_no_double_counting() {
        let html = r#"<html><body>
            <ul>
                <li>Parent
                    <ul>
                        <li>Child
                            <ul><li>Grandchild</li></ul>
                        </li>
                    </ul>
                </li>
            </ul>
        </body></html>"#;
        let opts = default_options();
        let doc = WebConverter::convert_html(html, None, &opts).unwrap();
        let elements = &doc.pages[0].elements;

        match &elements[0] {
            Element::List { items, .. } => {
                // Top-level list has 1 item ("Parent").
                assert_eq!(items.len(), 1);
                // "Parent" has 1 direct nested child ("Child").
                assert_eq!(items[0].children.len(), 1);
                // "Child" has 1 direct nested child ("Grandchild").
                assert_eq!(items[0].children[0].children.len(), 1);
            }
            other => panic!("Expected List, got {other:?}"),
        }
    }
}
