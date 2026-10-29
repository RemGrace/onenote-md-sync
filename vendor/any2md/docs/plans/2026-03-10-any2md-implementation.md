# any2md Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust CLI tool that converts PDF files to Markdown using a trait-based plugin architecture with heuristic element classification.

**Architecture:** Trait-based converter system where each input format implements `Converter`. All converters produce a unified `Document` model. A single `MarkdownRenderer` converts `Document` to Markdown. First converter: PDF via lopdf.

**Tech Stack:** Rust, clap 4 (CLI), lopdf 0.34 (PDF), image 0.25, base64 0.22, anyhow 1

---

### Task 1: Project Scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`

**Step 1: Initialize Cargo project**

Run:
```bash
cargo init --name any2md
```

Expected: Creates `Cargo.toml` and `src/main.rs`

**Step 2: Configure Cargo.toml with dependencies**

Replace `Cargo.toml` with:

```toml
[package]
name = "any2md"
version = "0.1.0"
edition = "2021"
description = "CLI utility for converting files to Markdown"

[dependencies]
clap = { version = "4", features = ["derive"] }
lopdf = "0.34"
image = "0.25"
base64 = "0.22"
anyhow = "1"
thiserror = "1"
```

**Step 3: Create minimal lib.rs**

Create `src/lib.rs`:

```rust
pub mod model;
pub mod converter;
pub mod renderer;
```

**Step 4: Create stub modules so it compiles**

Create `src/model/mod.rs`:
```rust
pub mod document;
pub mod options;
```

Create `src/model/document.rs`:
```rust
// Will be implemented in Task 2
```

Create `src/model/options.rs`:
```rust
// Will be implemented in Task 3
```

Create `src/converter/mod.rs`:
```rust
pub mod pdf;
```

Create `src/converter/pdf/mod.rs`:
```rust
pub mod extractor;
pub mod classifier;
pub mod assembler;
```

Create `src/converter/pdf/extractor.rs`:
```rust
// Will be implemented in Task 6
```

Create `src/converter/pdf/classifier.rs`:
```rust
// Will be implemented in Task 7
```

Create `src/converter/pdf/assembler.rs`:
```rust
// Will be implemented in Task 8
```

Create `src/renderer/mod.rs`:
```rust
pub mod markdown;
```

Create `src/renderer/markdown.rs`:
```rust
// Will be implemented in Task 5
```

**Step 5: Update main.rs to use lib**

Replace `src/main.rs` with:

```rust
fn main() {
    println!("any2md - coming soon");
}
```

**Step 6: Verify it compiles**

Run: `cargo check`
Expected: Compiles with no errors (warnings OK)

**Step 7: Commit**

```bash
git init
git add Cargo.toml src/ docs/
git commit -m "feat: scaffold project structure with stub modules"
```

---

### Task 2: Document Model

**Files:**
- Modify: `src/model/document.rs`
- Create: `tests/model_tests.rs`

**Step 1: Write the failing test**

Create `tests/model_tests.rs`:

```rust
use any2md::model::document::*;

#[test]
fn test_document_creation() {
    let doc = Document {
        metadata: Metadata {
            title: Some("Test".to_string()),
            author: None,
            date: None,
        },
        pages: vec![Page {
            elements: vec![
                Element::Heading {
                    level: 1,
                    text: "Hello".to_string(),
                },
                Element::Paragraph {
                    text: RichText {
                        segments: vec![TextSegment {
                            text: "World".to_string(),
                            bold: false,
                            italic: false,
                            code: false,
                            link: None,
                        }],
                    },
                },
            ],
        }],
    };
    assert_eq!(doc.metadata.title, Some("Test".to_string()));
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].elements.len(), 2);
}

#[test]
fn test_rich_text_display() {
    let rt = RichText {
        segments: vec![
            TextSegment {
                text: "hello ".to_string(),
                bold: true,
                italic: false,
                code: false,
                link: None,
            },
            TextSegment {
                text: "world".to_string(),
                bold: false,
                italic: true,
                code: false,
                link: None,
            },
        ],
    };
    assert_eq!(rt.segments.len(), 2);
    assert!(rt.segments[0].bold);
    assert!(rt.segments[1].italic);
}

#[test]
fn test_all_element_variants() {
    let elements: Vec<Element> = vec![
        Element::Heading { level: 1, text: "H1".to_string() },
        Element::Paragraph { text: RichText { segments: vec![] } },
        Element::CodeBlock { language: Some("rust".to_string()), code: "let x = 1;".to_string() },
        Element::List {
            ordered: false,
            items: vec![ListItem {
                text: RichText { segments: vec![TextSegment {
                    text: "item".to_string(),
                    bold: false, italic: false, code: false, link: None,
                }] },
                children: vec![],
            }],
        },
        Element::Table {
            headers: vec!["A".to_string()],
            rows: vec![vec!["1".to_string()]],
        },
        Element::Image { data: vec![0u8], alt: Some("img".to_string()) },
        Element::HorizontalRule,
        Element::BlockQuote { text: RichText { segments: vec![] } },
    ];
    assert_eq!(elements.len(), 8);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --test model_tests`
Expected: FAIL — types not defined yet

**Step 3: Implement the Document model**

Replace `src/model/document.rs` with:

```rust
#[derive(Debug, Clone)]
pub struct Document {
    pub metadata: Metadata,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone)]
pub enum Element {
    Heading { level: u8, text: String },
    Paragraph { text: RichText },
    CodeBlock { language: Option<String>, code: String },
    List { ordered: bool, items: Vec<ListItem> },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    Image { data: Vec<u8>, alt: Option<String> },
    HorizontalRule,
    BlockQuote { text: RichText },
}

#[derive(Debug, Clone)]
pub struct RichText {
    pub segments: Vec<TextSegment>,
}

#[derive(Debug, Clone)]
pub struct TextSegment {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ListItem {
    pub text: RichText,
    pub children: Vec<ListItem>,
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --test model_tests`
Expected: 3 tests PASS

**Step 5: Commit**

```bash
git add src/model/document.rs tests/model_tests.rs
git commit -m "feat: implement Document intermediate model with all element types"
```

---

### Task 3: ConvertOptions and Error Types

**Files:**
- Modify: `src/model/options.rs`
- Create: `src/error.rs`
- Modify: `src/lib.rs`

**Step 1: Write the failing test**

Add to `tests/model_tests.rs`:

```rust
use any2md::model::options::*;
use std::path::PathBuf;

#[test]
fn test_convert_options_defaults() {
    let opts = ConvertOptions::default();
    assert!(matches!(opts.image_mode, ImageMode::Extract));
    assert!(matches!(opts.page_mode, PageMode::SingleFile));
    assert_eq!(opts.image_output_dir, PathBuf::from("images"));
}

#[test]
fn test_image_mode_variants() {
    let extract = ImageMode::Extract;
    let inline = ImageMode::Inline;
    assert!(matches!(extract, ImageMode::Extract));
    assert!(matches!(inline, ImageMode::Inline));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --test model_tests`
Expected: FAIL — `ConvertOptions` not defined

**Step 3: Implement options and error types**

Replace `src/model/options.rs` with:

```rust
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub image_mode: ImageMode,
    pub page_mode: PageMode,
    pub image_output_dir: PathBuf,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            image_mode: ImageMode::Extract,
            page_mode: PageMode::SingleFile,
            image_output_dir: PathBuf::from("images"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ImageMode {
    Extract,
    Inline,
}

#[derive(Debug, Clone)]
pub enum PageMode {
    SingleFile,
    SplitPages,
}
```

Create `src/error.rs`:

```rust
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConvertError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Corrupted file: {0}")]
    CorruptedFile(String),

    #[error("Image extraction failed: {0}")]
    ImageExtractionFailed(String),

    #[error(transparent)]
    IoError(#[from] std::io::Error),
}
```

Update `src/lib.rs` to:

```rust
pub mod model;
pub mod converter;
pub mod renderer;
pub mod error;
```

**Step 4: Run test to verify it passes**

Run: `cargo test --test model_tests`
Expected: 5 tests PASS

**Step 5: Commit**

```bash
git add src/model/options.rs src/error.rs src/lib.rs tests/model_tests.rs
git commit -m "feat: add ConvertOptions, ImageMode, PageMode, and ConvertError"
```

---

### Task 4: Converter Trait and Registry

**Files:**
- Modify: `src/converter/mod.rs`
- Create: `tests/converter_tests.rs`

**Step 1: Write the failing test**

Create `tests/converter_tests.rs`:

```rust
use any2md::converter::{Converter, ConverterRegistry};
use any2md::model::document::*;
use any2md::model::options::ConvertOptions;
use any2md::error::ConvertError;
use std::path::Path;

struct MockConverter;

impl Converter for MockConverter {
    fn name(&self) -> &str {
        "mock"
    }

    fn supported_extensions(&self) -> &[&str] {
        &["mock", "mck"]
    }

    fn convert(&self, _input: &Path, _options: &ConvertOptions) -> Result<Document, ConvertError> {
        Ok(Document {
            metadata: Metadata {
                title: Some("Mock".to_string()),
                author: None,
                date: None,
            },
            pages: vec![],
        })
    }
}

#[test]
fn test_registry_find_by_extension() {
    let mut registry = ConverterRegistry::new();
    registry.register(Box::new(MockConverter));
    assert!(registry.find_by_extension("mock").is_some());
    assert!(registry.find_by_extension("mck").is_some());
    assert!(registry.find_by_extension("unknown").is_none());
}

#[test]
fn test_converter_trait() {
    let converter = MockConverter;
    assert_eq!(converter.name(), "mock");
    assert_eq!(converter.supported_extensions(), &["mock", "mck"]);

    let opts = ConvertOptions::default();
    let doc = converter.convert(Path::new("test.mock"), &opts).unwrap();
    assert_eq!(doc.metadata.title, Some("Mock".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --test converter_tests`
Expected: FAIL — `Converter` trait and `ConverterRegistry` not defined

**Step 3: Implement Converter trait and registry**

Replace `src/converter/mod.rs` with:

```rust
pub mod pdf;

use std::path::Path;
use crate::model::document::Document;
use crate::model::options::ConvertOptions;
use crate::error::ConvertError;

pub trait Converter {
    fn name(&self) -> &str;
    fn supported_extensions(&self) -> &[&str];
    fn convert(&self, input: &Path, options: &ConvertOptions) -> Result<Document, ConvertError>;
}

pub struct ConverterRegistry {
    converters: Vec<Box<dyn Converter>>,
}

impl ConverterRegistry {
    pub fn new() -> Self {
        Self {
            converters: Vec::new(),
        }
    }

    pub fn register(&mut self, converter: Box<dyn Converter>) {
        self.converters.push(converter);
    }

    pub fn find_by_extension(&self, ext: &str) -> Option<&dyn Converter> {
        let ext_lower = ext.to_lowercase();
        self.converters
            .iter()
            .find(|c| c.supported_extensions().iter().any(|e| e.to_lowercase() == ext_lower))
            .map(|c| c.as_ref())
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --test converter_tests`
Expected: 2 tests PASS

**Step 5: Commit**

```bash
git add src/converter/mod.rs tests/converter_tests.rs
git commit -m "feat: implement Converter trait and ConverterRegistry"
```

---

### Task 5: Markdown Renderer

**Files:**
- Modify: `src/renderer/markdown.rs`
- Modify: `src/renderer/mod.rs`
- Create: `tests/renderer_tests.rs`

**Step 1: Write the failing tests**

Create `tests/renderer_tests.rs`:

```rust
use any2md::model::document::*;
use any2md::model::options::*;
use any2md::renderer::markdown::MarkdownRenderer;

fn plain_text(s: &str) -> RichText {
    RichText {
        segments: vec![TextSegment {
            text: s.to_string(),
            bold: false,
            italic: false,
            code: false,
            link: None,
        }],
    }
}

#[test]
fn test_render_heading() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![
                Element::Heading { level: 1, text: "Title".to_string() },
                Element::Heading { level: 2, text: "Subtitle".to_string() },
                Element::Heading { level: 3, text: "Section".to_string() },
            ],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("# Title"));
    assert!(result.contains("## Subtitle"));
    assert!(result.contains("### Section"));
}

#[test]
fn test_render_paragraph_plain() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::Paragraph { text: plain_text("Hello world") }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("Hello world"));
}

#[test]
fn test_render_rich_text_formatting() {
    let rt = RichText {
        segments: vec![
            TextSegment { text: "bold".to_string(), bold: true, italic: false, code: false, link: None },
            TextSegment { text: " and ".to_string(), bold: false, italic: false, code: false, link: None },
            TextSegment { text: "italic".to_string(), bold: false, italic: true, code: false, link: None },
            TextSegment { text: " and ".to_string(), bold: false, italic: false, code: false, link: None },
            TextSegment { text: "code".to_string(), bold: false, italic: false, code: true, link: None },
            TextSegment { text: " and ".to_string(), bold: false, italic: false, code: false, link: None },
            TextSegment { text: "link".to_string(), bold: false, italic: false, code: false, link: Some("https://example.com".to_string()) },
        ],
    };
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::Paragraph { text: rt }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("**bold**"));
    assert!(result.contains("*italic*"));
    assert!(result.contains("`code`"));
    assert!(result.contains("[link](https://example.com)"));
}

#[test]
fn test_render_code_block() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::CodeBlock {
                language: Some("rust".to_string()),
                code: "fn main() {}".to_string(),
            }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("```rust\nfn main() {}\n```"));
}

#[test]
fn test_render_code_block_no_language() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::CodeBlock {
                language: None,
                code: "some code".to_string(),
            }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("```\nsome code\n```"));
}

#[test]
fn test_render_unordered_list() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::List {
                ordered: false,
                items: vec![
                    ListItem { text: plain_text("First"), children: vec![] },
                    ListItem { text: plain_text("Second"), children: vec![] },
                ],
            }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("- First"));
    assert!(result.contains("- Second"));
}

#[test]
fn test_render_ordered_list() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::List {
                ordered: true,
                items: vec![
                    ListItem { text: plain_text("First"), children: vec![] },
                    ListItem { text: plain_text("Second"), children: vec![] },
                ],
            }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("1. First"));
    assert!(result.contains("2. Second"));
}

#[test]
fn test_render_nested_list() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::List {
                ordered: false,
                items: vec![
                    ListItem {
                        text: plain_text("Parent"),
                        children: vec![
                            ListItem { text: plain_text("Child"), children: vec![] },
                        ],
                    },
                ],
            }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("- Parent"));
    assert!(result.contains("  - Child"));
}

#[test]
fn test_render_table() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::Table {
                headers: vec!["Name".to_string(), "Age".to_string()],
                rows: vec![
                    vec!["Alice".to_string(), "30".to_string()],
                    vec!["Bob".to_string(), "25".to_string()],
                ],
            }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("| Name | Age |"));
    assert!(result.contains("| --- | --- |"));
    assert!(result.contains("| Alice | 30 |"));
    assert!(result.contains("| Bob | 25 |"));
}

#[test]
fn test_render_horizontal_rule() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::HorizontalRule],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("---"));
}

#[test]
fn test_render_blockquote() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::BlockQuote { text: plain_text("A quote") }],
        }],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("> A quote"));
}

#[test]
fn test_render_image_inline() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![Page {
            elements: vec![Element::Image {
                data: vec![0x89, 0x50, 0x4E, 0x47],
                alt: Some("test image".to_string()),
            }],
        }],
    };
    let opts = ConvertOptions {
        image_mode: ImageMode::Inline,
        ..ConvertOptions::default()
    };
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("![test image](data:image/png;base64,"));
}

#[test]
fn test_render_multiple_pages_single_file() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![
            Page { elements: vec![Element::Paragraph { text: plain_text("Page 1") }] },
            Page { elements: vec![Element::Paragraph { text: plain_text("Page 2") }] },
        ],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("Page 1"));
    assert!(result.contains("<!-- page 2 -->"));
    assert!(result.contains("Page 2"));
}

#[test]
fn test_render_metadata_header() {
    let doc = Document {
        metadata: Metadata {
            title: Some("My Doc".to_string()),
            author: Some("Author".to_string()),
            date: Some("2026-01-01".to_string()),
        },
        pages: vec![],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("# My Doc"));
    assert!(result.contains("Author"));
    assert!(result.contains("2026-01-01"));
}

#[test]
fn test_render_empty_page_skipped() {
    let doc = Document {
        metadata: Metadata { title: None, author: None, date: None },
        pages: vec![
            Page { elements: vec![] },
            Page { elements: vec![Element::Paragraph { text: plain_text("Content") }] },
        ],
    };
    let opts = ConvertOptions::default();
    let result = MarkdownRenderer::render(&doc, &opts);
    assert!(result.contains("Content"));
    assert!(!result.contains("<!-- page 1 -->"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --test renderer_tests`
Expected: FAIL — `MarkdownRenderer` not defined

**Step 3: Implement MarkdownRenderer**

Update `src/renderer/mod.rs` to:

```rust
pub mod markdown;
```

Replace `src/renderer/markdown.rs` with:

```rust
use crate::model::document::*;
use crate::model::options::{ConvertOptions, ImageMode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

pub struct MarkdownRenderer;

impl MarkdownRenderer {
    pub fn render(doc: &Document, opts: &ConvertOptions) -> String {
        let mut out = String::new();

        Self::render_metadata(&doc.metadata, &mut out);

        let non_empty_pages: Vec<(usize, &Page)> = doc
            .pages
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.elements.is_empty())
            .collect();

        for (i, (page_idx, page)) in non_empty_pages.iter().enumerate() {
            if i > 0 {
                out.push_str(&format!("\n<!-- page {} -->\n\n---\n\n", page_idx + 1));
            }
            for (j, element) in page.elements.iter().enumerate() {
                if j > 0 {
                    out.push('\n');
                }
                Self::render_element(element, opts, &mut out);
            }
        }

        out
    }

    fn render_metadata(meta: &Metadata, out: &mut String) {
        if let Some(title) = &meta.title {
            out.push_str(&format!("# {}\n\n", title));
        }
        let mut meta_parts = Vec::new();
        if let Some(author) = &meta.author {
            meta_parts.push(format!("**Author:** {}", author));
        }
        if let Some(date) = &meta.date {
            meta_parts.push(format!("**Date:** {}", date));
        }
        if !meta_parts.is_empty() {
            out.push_str(&meta_parts.join(" | "));
            out.push_str("\n\n");
        }
    }

    fn render_element(el: &Element, opts: &ConvertOptions, out: &mut String) {
        match el {
            Element::Heading { level, text } => {
                let hashes = "#".repeat(*level as usize);
                out.push_str(&format!("{} {}\n", hashes, text));
            }
            Element::Paragraph { text } => {
                out.push_str(&Self::render_rich_text(text));
                out.push('\n');
            }
            Element::CodeBlock { language, code } => {
                let lang = language.as_deref().unwrap_or("");
                out.push_str(&format!("```{}\n{}\n```\n", lang, code));
            }
            Element::List { ordered, items } => {
                Self::render_list_items(items, *ordered, 0, out);
            }
            Element::Table { headers, rows } => {
                Self::render_table(headers, rows, out);
            }
            Element::Image { data, alt } => {
                Self::render_image(data, alt.as_deref(), opts, out);
            }
            Element::HorizontalRule => {
                out.push_str("---\n");
            }
            Element::BlockQuote { text } => {
                let rendered = Self::render_rich_text(text);
                for line in rendered.lines() {
                    out.push_str(&format!("> {}\n", line));
                }
            }
        }
    }

    fn render_rich_text(rt: &RichText) -> String {
        let mut s = String::new();
        for seg in &rt.segments {
            let mut text = seg.text.clone();
            if seg.bold {
                text = format!("**{}**", text);
            }
            if seg.italic {
                text = format!("*{}*", text);
            }
            if seg.code {
                text = format!("`{}`", text);
            }
            if let Some(url) = &seg.link {
                text = format!("[{}]({})", text, url);
            }
            s.push_str(&text);
        }
        s
    }

    fn render_list_items(items: &[ListItem], ordered: bool, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);
        for (i, item) in items.iter().enumerate() {
            let marker = if ordered {
                format!("{}. ", i + 1)
            } else {
                "- ".to_string()
            };
            out.push_str(&format!("{}{}{}\n", indent, marker, Self::render_rich_text(&item.text)));
            if !item.children.is_empty() {
                Self::render_list_items(&item.children, ordered, depth + 1, out);
            }
        }
    }

    fn render_table(headers: &[String], rows: &[Vec<String>], out: &mut String) {
        out.push_str("| ");
        out.push_str(&headers.join(" | "));
        out.push_str(" |\n");

        out.push_str("| ");
        out.push_str(&headers.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
        out.push_str(" |\n");

        for row in rows {
            out.push_str("| ");
            out.push_str(&row.join(" | "));
            out.push_str(" |\n");
        }
    }

    fn render_image(data: &[u8], alt: Option<&str>, opts: &ConvertOptions, out: &mut String) {
        let alt_text = alt.unwrap_or("image");
        match opts.image_mode {
            ImageMode::Inline => {
                let encoded = BASE64.encode(data);
                out.push_str(&format!("![{}](data:image/png;base64,{})\n", alt_text, encoded));
            }
            ImageMode::Extract => {
                out.push_str(&format!("![{}]({})\n", alt_text, "image_placeholder"));
            }
        }
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --test renderer_tests`
Expected: All 15 tests PASS

**Step 5: Commit**

```bash
git add src/renderer/ tests/renderer_tests.rs
git commit -m "feat: implement MarkdownRenderer with full element support"
```

---

### Task 6: PDF Extractor (RawElement extraction via lopdf)

**Files:**
- Modify: `src/converter/pdf/extractor.rs`
- Modify: `src/converter/pdf/mod.rs`

**Step 1: Write the failing test**

Add to `src/converter/pdf/extractor.rs` (at the bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_text_block_creation() {
        let block = RawTextBlock {
            text: "Hello".to_string(),
            x: 72.0,
            y: 700.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
        };
        assert_eq!(block.text, "Hello");
        assert_eq!(block.font_size, 12.0);
    }

    #[test]
    fn test_raw_image_creation() {
        let img = RawImage {
            data: vec![0x89, 0x50],
            width: 100,
            height: 50,
        };
        assert_eq!(img.width, 100);
        assert_eq!(img.data.len(), 2);
    }

    #[test]
    fn test_extract_nonexistent_file() {
        let result = PdfExtractor::extract(Path::new("nonexistent.pdf"));
        assert!(result.is_err());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test pdf::extractor`
Expected: FAIL — types not defined

**Step 3: Implement PdfExtractor with RawElement types**

Replace `src/converter/pdf/extractor.rs` with:

```rust
use std::path::Path;
use crate::error::ConvertError;

#[derive(Debug, Clone)]
pub struct RawTextBlock {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
    pub font_name: String,
}

#[derive(Debug, Clone)]
pub struct RawImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub enum RawElement {
    Text(RawTextBlock),
    Image(RawImage),
}

pub struct RawPage {
    pub elements: Vec<RawElement>,
}

pub struct PdfExtractor;

impl PdfExtractor {
    pub fn extract(path: &Path) -> Result<Vec<RawPage>, ConvertError> {
        if !path.exists() {
            return Err(ConvertError::FileNotFound(path.to_path_buf()));
        }

        let doc = lopdf::Document::load(path).map_err(|e| {
            ConvertError::CorruptedFile(format!("Failed to parse PDF: {}", e))
        })?;

        let mut pages = Vec::new();
        let page_count = doc.get_pages().len();

        for page_num in 1..=page_count as u32 {
            let raw_page = Self::extract_page(&doc, page_num)?;
            pages.push(raw_page);
        }

        Ok(pages)
    }

    fn extract_page(doc: &lopdf::Document, page_num: u32) -> Result<RawPage, ConvertError> {
        let mut elements = Vec::new();

        if let Ok(content) = doc.extract_text(&[page_num]) {
            let lines: Vec<&str> = content.lines().collect();
            let mut y_pos = 800.0;
            for line in lines {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    elements.push(RawElement::Text(RawTextBlock {
                        text: trimmed.to_string(),
                        x: 72.0,
                        y: y_pos,
                        font_size: 12.0,
                        font_name: "Unknown".to_string(),
                    }));
                    y_pos -= 14.0;
                }
            }
        }

        Ok(RawPage { elements })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_text_block_creation() {
        let block = RawTextBlock {
            text: "Hello".to_string(),
            x: 72.0,
            y: 700.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
        };
        assert_eq!(block.text, "Hello");
        assert_eq!(block.font_size, 12.0);
    }

    #[test]
    fn test_raw_image_creation() {
        let img = RawImage {
            data: vec![0x89, 0x50],
            width: 100,
            height: 50,
        };
        assert_eq!(img.width, 100);
        assert_eq!(img.data.len(), 2);
    }

    #[test]
    fn test_extract_nonexistent_file() {
        let result = PdfExtractor::extract(Path::new("nonexistent.pdf"));
        assert!(result.is_err());
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test pdf::extractor`
Expected: 3 tests PASS

**Step 5: Commit**

```bash
git add src/converter/pdf/extractor.rs
git commit -m "feat: implement PdfExtractor with lopdf text extraction"
```

---

### Task 7: PDF Classifier (Heuristics)

**Files:**
- Modify: `src/converter/pdf/classifier.rs`

**Step 1: Write the failing tests**

Replace `src/converter/pdf/classifier.rs` with tests at bottom:

```rust
use crate::converter::pdf::extractor::{RawTextBlock, RawElement, RawPage};
use crate::model::document::Element;

#[derive(Debug, Clone, PartialEq)]
pub enum BlockType {
    Heading(u8),
    CodeBlock,
    ListItem,
    Paragraph,
}

pub struct Classifier;

impl Classifier {
    pub fn classify(pages: &[RawPage]) -> Vec<Vec<(RawTextBlock, BlockType)>> {
        let avg_font_size = Self::average_font_size(pages);

        pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter_map(|el| {
                        if let RawElement::Text(block) = el {
                            let block_type = Self::classify_block(block, avg_font_size);
                            Some((block.clone(), block_type))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn classify_block(block: &RawTextBlock, avg_font_size: f64) -> BlockType {
        if Self::is_code(block) {
            return BlockType::CodeBlock;
        }
        if Self::is_heading(block, avg_font_size) {
            let level = Self::heading_level(block.font_size, avg_font_size);
            return BlockType::Heading(level);
        }
        if Self::is_list_item(block) {
            return BlockType::ListItem;
        }
        BlockType::Paragraph
    }

    fn is_code(block: &RawTextBlock) -> bool {
        let mono_fonts = ["courier", "consolas", "menlo", "monaco", "monospace", "source code", "fira code"];
        let font_lower = block.font_name.to_lowercase();
        mono_fonts.iter().any(|f| font_lower.contains(f))
    }

    fn is_heading(block: &RawTextBlock, avg_font_size: f64) -> bool {
        block.font_size > avg_font_size * 1.2
    }

    fn heading_level(font_size: f64, avg_font_size: f64) -> u8 {
        let ratio = font_size / avg_font_size;
        if ratio > 2.0 {
            1
        } else if ratio > 1.5 {
            2
        } else {
            3
        }
    }

    fn is_list_item(block: &RawTextBlock) -> bool {
        let text = block.text.trim();
        text.starts_with("• ")
            || text.starts_with("- ")
            || text.starts_with("* ")
            || Self::starts_with_ordered_marker(text)
    }

    fn starts_with_ordered_marker(text: &str) -> bool {
        let mut chars = text.chars();
        let first = chars.next();
        match first {
            Some(c) if c.is_ascii_digit() => {
                for ch in chars {
                    if ch == '.' || ch == ')' {
                        return true;
                    }
                    if !ch.is_ascii_digit() {
                        return false;
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn is_bold(font_name: &str) -> bool {
        font_name.to_lowercase().contains("bold")
    }

    fn is_italic(font_name: &str) -> bool {
        let lower = font_name.to_lowercase();
        lower.contains("italic") || lower.contains("oblique")
    }

    fn average_font_size(pages: &[RawPage]) -> f64 {
        let mut total = 0.0;
        let mut count = 0;
        for page in pages {
            for el in &page.elements {
                if let RawElement::Text(block) = el {
                    total += block.font_size;
                    count += 1;
                }
            }
        }
        if count == 0 { 12.0 } else { total / count as f64 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_block(text: &str, font_size: f64, font_name: &str) -> RawTextBlock {
        RawTextBlock {
            text: text.to_string(),
            x: 72.0,
            y: 700.0,
            font_size,
            font_name: font_name.to_string(),
        }
    }

    fn make_page(blocks: Vec<RawTextBlock>) -> RawPage {
        RawPage {
            elements: blocks.into_iter().map(RawElement::Text).collect(),
        }
    }

    #[test]
    fn test_classify_heading_by_font_size() {
        let pages = vec![make_page(vec![
            make_block("Title", 24.0, "Helvetica-Bold"),
            make_block("Normal text", 12.0, "Helvetica"),
            make_block("Normal text 2", 12.0, "Helvetica"),
        ])];
        let result = Classifier::classify(&pages);
        assert_eq!(result[0][0].1, BlockType::Heading(1));
        assert_eq!(result[0][1].1, BlockType::Paragraph);
    }

    #[test]
    fn test_classify_code_by_font() {
        let pages = vec![make_page(vec![
            make_block("let x = 1;", 12.0, "Courier"),
            make_block("normal text", 12.0, "Helvetica"),
        ])];
        let result = Classifier::classify(&pages);
        assert_eq!(result[0][0].1, BlockType::CodeBlock);
        assert_eq!(result[0][1].1, BlockType::Paragraph);
    }

    #[test]
    fn test_classify_list_items() {
        let pages = vec![make_page(vec![
            make_block("• item one", 12.0, "Helvetica"),
            make_block("- item two", 12.0, "Helvetica"),
            make_block("1. item three", 12.0, "Helvetica"),
            make_block("normal text", 12.0, "Helvetica"),
        ])];
        let result = Classifier::classify(&pages);
        assert_eq!(result[0][0].1, BlockType::ListItem);
        assert_eq!(result[0][1].1, BlockType::ListItem);
        assert_eq!(result[0][2].1, BlockType::ListItem);
        assert_eq!(result[0][3].1, BlockType::Paragraph);
    }

    #[test]
    fn test_code_has_priority_over_heading() {
        let pages = vec![make_page(vec![
            make_block("fn main()", 24.0, "Courier"),
            make_block("normal", 12.0, "Helvetica"),
        ])];
        let result = Classifier::classify(&pages);
        assert_eq!(result[0][0].1, BlockType::CodeBlock);
    }

    #[test]
    fn test_heading_levels() {
        let pages = vec![make_page(vec![
            make_block("Big title", 30.0, "Helvetica"),
            make_block("Medium title", 20.0, "Helvetica"),
            make_block("Small title", 16.0, "Helvetica"),
            make_block("Normal", 12.0, "Helvetica"),
            make_block("Normal 2", 12.0, "Helvetica"),
            make_block("Normal 3", 12.0, "Helvetica"),
        ])];
        let result = Classifier::classify(&pages);
        assert_eq!(result[0][0].1, BlockType::Heading(1));
        assert_eq!(result[0][1].1, BlockType::Heading(2));
        assert_eq!(result[0][2].1, BlockType::Heading(3));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test pdf::classifier`
Expected: FAIL initially, then PASS once you write the full file above (tests and impl together)

**Step 3: The implementation is included above. Run tests.**

Run: `cargo test pdf::classifier`
Expected: 5 tests PASS

**Step 4: Commit**

```bash
git add src/converter/pdf/classifier.rs
git commit -m "feat: implement PDF classifier with heuristic detection"
```

---

### Task 8: PDF Assembler (Classified blocks to Document)

**Files:**
- Modify: `src/converter/pdf/assembler.rs`

**Step 1: Write the failing tests and implementation**

Replace `src/converter/pdf/assembler.rs` with:

```rust
use crate::converter::pdf::classifier::BlockType;
use crate::converter::pdf::extractor::RawTextBlock;
use crate::model::document::*;

pub struct Assembler;

impl Assembler {
    pub fn assemble(classified_pages: Vec<Vec<(RawTextBlock, BlockType)>>, metadata: Metadata) -> Document {
        let pages = classified_pages
            .into_iter()
            .map(|blocks| Self::assemble_page(blocks))
            .collect();

        Document { metadata, pages }
    }

    fn assemble_page(blocks: Vec<(RawTextBlock, BlockType)>) -> Page {
        let mut elements = Vec::new();
        let mut i = 0;

        while i < blocks.len() {
            let (ref block, ref block_type) = blocks[i];

            match block_type {
                BlockType::Heading(level) => {
                    elements.push(Element::Heading {
                        level: *level,
                        text: block.text.clone(),
                    });
                    i += 1;
                }
                BlockType::CodeBlock => {
                    let mut code_lines = vec![block.text.clone()];
                    i += 1;
                    while i < blocks.len() {
                        if let BlockType::CodeBlock = blocks[i].1 {
                            code_lines.push(blocks[i].0.text.clone());
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    elements.push(Element::CodeBlock {
                        language: None,
                        code: code_lines.join("\n"),
                    });
                }
                BlockType::ListItem => {
                    let mut items = Vec::new();
                    while i < blocks.len() {
                        if let BlockType::ListItem = blocks[i].1 {
                            let text = Self::strip_list_marker(&blocks[i].0.text);
                            items.push(ListItem {
                                text: Self::plain_rich_text(&text),
                                children: vec![],
                            });
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    let ordered = Self::detect_ordered(&blocks.get(i.saturating_sub(items.len()))
                        .map(|(b, _)| b.text.as_str()).unwrap_or(""));
                    elements.push(Element::List { ordered, items });
                }
                BlockType::Paragraph => {
                    elements.push(Element::Paragraph {
                        text: Self::plain_rich_text(&block.text),
                    });
                    i += 1;
                }
            }
        }

        Page { elements }
    }

    fn strip_list_marker(text: &str) -> String {
        let text = text.trim();
        if text.starts_with("• ") || text.starts_with("- ") || text.starts_with("* ") {
            return text[2..].to_string();
        }
        if let Some(pos) = text.find(". ") {
            let prefix = &text[..pos];
            if prefix.chars().all(|c| c.is_ascii_digit()) {
                return text[pos + 2..].to_string();
            }
        }
        if let Some(pos) = text.find(") ") {
            let prefix = &text[..pos];
            if prefix.chars().all(|c| c.is_ascii_digit()) {
                return text[pos + 2..].to_string();
            }
        }
        text.to_string()
    }

    fn detect_ordered(text: &str) -> bool {
        let trimmed = text.trim();
        trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
    }

    fn plain_rich_text(text: &str) -> RichText {
        RichText {
            segments: vec![TextSegment {
                text: text.to_string(),
                bold: false,
                italic: false,
                code: false,
                link: None,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::pdf::extractor::RawTextBlock;

    fn make_block(text: &str) -> RawTextBlock {
        RawTextBlock {
            text: text.to_string(),
            x: 72.0,
            y: 700.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
        }
    }

    fn empty_metadata() -> Metadata {
        Metadata { title: None, author: None, date: None }
    }

    #[test]
    fn test_assemble_headings() {
        let blocks = vec![
            (make_block("Title"), BlockType::Heading(1)),
            (make_block("Subtitle"), BlockType::Heading(2)),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata());
        assert_eq!(doc.pages.len(), 1);
        assert!(matches!(&doc.pages[0].elements[0], Element::Heading { level: 1, text } if text == "Title"));
        assert!(matches!(&doc.pages[0].elements[1], Element::Heading { level: 2, text } if text == "Subtitle"));
    }

    #[test]
    fn test_assemble_consecutive_code_blocks_merged() {
        let blocks = vec![
            (make_block("fn main() {"), BlockType::CodeBlock),
            (make_block("    println!(\"hi\");"), BlockType::CodeBlock),
            (make_block("}"), BlockType::CodeBlock),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata());
        assert_eq!(doc.pages[0].elements.len(), 1);
        if let Element::CodeBlock { code, .. } = &doc.pages[0].elements[0] {
            assert!(code.contains("fn main()"));
            assert!(code.contains("println!"));
            assert!(code.contains("}"));
        } else {
            panic!("Expected CodeBlock");
        }
    }

    #[test]
    fn test_assemble_list_items() {
        let blocks = vec![
            (make_block("- First"), BlockType::ListItem),
            (make_block("- Second"), BlockType::ListItem),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata());
        if let Element::List { ordered, items } = &doc.pages[0].elements[0] {
            assert!(!ordered);
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].text.segments[0].text, "First");
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn test_assemble_ordered_list() {
        let blocks = vec![
            (make_block("1. First"), BlockType::ListItem),
            (make_block("2. Second"), BlockType::ListItem),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata());
        if let Element::List { ordered, items } = &doc.pages[0].elements[0] {
            assert!(ordered);
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected ordered List");
        }
    }

    #[test]
    fn test_assemble_paragraphs() {
        let blocks = vec![
            (make_block("Some text"), BlockType::Paragraph),
            (make_block("More text"), BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata());
        assert_eq!(doc.pages[0].elements.len(), 2);
        assert!(matches!(&doc.pages[0].elements[0], Element::Paragraph { .. }));
    }

    #[test]
    fn test_assemble_mixed_content() {
        let blocks = vec![
            (make_block("Title"), BlockType::Heading(1)),
            (make_block("Intro text"), BlockType::Paragraph),
            (make_block("let x = 1;"), BlockType::CodeBlock),
            (make_block("- item"), BlockType::ListItem),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata());
        assert_eq!(doc.pages[0].elements.len(), 4);
    }

    #[test]
    fn test_strip_list_markers() {
        assert_eq!(Assembler::strip_list_marker("- item"), "item");
        assert_eq!(Assembler::strip_list_marker("• item"), "item");
        assert_eq!(Assembler::strip_list_marker("* item"), "item");
        assert_eq!(Assembler::strip_list_marker("1. item"), "item");
        assert_eq!(Assembler::strip_list_marker("10. item"), "item");
    }
}
```

**Step 2: Run tests**

Run: `cargo test pdf::assembler`
Expected: 7 tests PASS

**Step 3: Commit**

```bash
git add src/converter/pdf/assembler.rs
git commit -m "feat: implement PDF assembler converting classified blocks to Document"
```

---

### Task 9: PDF Converter (Glue: Extractor → Classifier → Assembler)

**Files:**
- Modify: `src/converter/pdf/mod.rs`

**Step 1: Write the failing test**

Replace `src/converter/pdf/mod.rs` with:

```rust
pub mod extractor;
pub mod classifier;
pub mod assembler;

use std::path::Path;
use crate::converter::Converter;
use crate::model::document::{Document, Metadata};
use crate::model::options::ConvertOptions;
use crate::error::ConvertError;
use extractor::PdfExtractor;
use classifier::Classifier;
use assembler::Assembler;

pub struct PdfConverter;

impl Converter for PdfConverter {
    fn name(&self) -> &str {
        "pdf"
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pdf"]
    }

    fn convert(&self, input: &Path, _options: &ConvertOptions) -> Result<Document, ConvertError> {
        let raw_pages = PdfExtractor::extract(input)?;
        let classified = Classifier::classify(&raw_pages);
        let metadata = Metadata {
            title: None,
            author: None,
            date: None,
        };
        Ok(Assembler::assemble(classified, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdf_converter_name() {
        let conv = PdfConverter;
        assert_eq!(conv.name(), "pdf");
    }

    #[test]
    fn test_pdf_converter_extensions() {
        let conv = PdfConverter;
        assert_eq!(conv.supported_extensions(), &["pdf"]);
    }

    #[test]
    fn test_pdf_converter_file_not_found() {
        let conv = PdfConverter;
        let opts = ConvertOptions::default();
        let result = conv.convert(Path::new("nonexistent.pdf"), &opts);
        assert!(result.is_err());
    }
}
```

**Step 2: Run tests**

Run: `cargo test pdf::tests`
Expected: 3 tests PASS

**Step 3: Commit**

```bash
git add src/converter/pdf/mod.rs
git commit -m "feat: implement PdfConverter gluing extractor, classifier, and assembler"
```

---

### Task 10: CLI with clap

**Files:**
- Modify: `src/main.rs`

**Step 1: Implement CLI**

Replace `src/main.rs` with:

```rust
use std::path::{Path, PathBuf};
use std::process;
use clap::Parser;
use any2md::converter::{Converter, ConverterRegistry};
use any2md::converter::pdf::PdfConverter;
use any2md::model::options::{ConvertOptions, ImageMode, PageMode};
use any2md::renderer::markdown::MarkdownRenderer;

#[derive(Parser)]
#[command(name = "any2md", about = "Convert files to Markdown")]
struct Cli {
    /// Input file path
    input: PathBuf,

    /// Output file path (default: <input_name>.md)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Image mode: extract or inline
    #[arg(long, default_value = "extract")]
    images: String,

    /// Page mode: single or split
    #[arg(long, default_value = "single")]
    pages: String,
}

fn main() {
    let cli = Cli::parse();

    let image_mode = match cli.images.as_str() {
        "extract" => ImageMode::Extract,
        "inline" => ImageMode::Inline,
        other => {
            eprintln!("Error: unknown image mode '{}'. Use 'extract' or 'inline'.", other);
            process::exit(1);
        }
    };

    let page_mode = match cli.pages.as_str() {
        "single" => PageMode::SingleFile,
        "split" => PageMode::SplitPages,
        other => {
            eprintln!("Error: unknown page mode '{}'. Use 'single' or 'split'.", other);
            process::exit(1);
        }
    };

    let output_path = cli.output.unwrap_or_else(|| {
        let stem = cli.input.file_stem().unwrap_or_default();
        PathBuf::from(format!("{}.md", stem.to_string_lossy()))
    });

    let image_output_dir = output_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("images");

    let options = ConvertOptions {
        image_mode,
        page_mode,
        image_output_dir,
    };

    let mut registry = ConverterRegistry::new();
    registry.register(Box::new(PdfConverter));

    let ext = cli
        .input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let converter = match registry.find_by_extension(ext) {
        Some(c) => c,
        None => {
            eprintln!("Error: unsupported format '.{}'", ext);
            process::exit(1);
        }
    };

    eprintln!("Converting {}...", cli.input.display());

    let doc = match converter.convert(&cli.input, &options) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    };

    let markdown = MarkdownRenderer::render(&doc, &options);

    match std::fs::write(&output_path, &markdown) {
        Ok(_) => eprintln!("Written to {}", output_path.display()),
        Err(e) => {
            eprintln!("Error writing output: {}", e);
            process::exit(1);
        }
    }
}
```

**Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully

**Step 3: Test CLI help**

Run: `cargo run -- --help`
Expected: Shows usage with input, --output, --images, --pages options

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: implement CLI with clap argument parsing"
```

---

### Task 11: Integration Test with Test PDF

**Files:**
- Create: `tests/integration_test.rs`
- Create: `tests/fixtures/` (directory)

**Step 1: Create a simple integration test**

Create `tests/integration_test.rs`:

```rust
use std::path::Path;
use any2md::converter::Converter;
use any2md::converter::pdf::PdfConverter;
use any2md::model::options::ConvertOptions;
use any2md::renderer::markdown::MarkdownRenderer;

#[test]
fn test_pdf_converter_nonexistent_file_returns_error() {
    let converter = PdfConverter;
    let opts = ConvertOptions::default();
    let result = converter.convert(Path::new("tests/fixtures/nonexistent.pdf"), &opts);
    assert!(result.is_err());
}

#[test]
fn test_full_pipeline_with_document() {
    use any2md::model::document::*;
    use any2md::model::options::*;

    let doc = Document {
        metadata: Metadata {
            title: Some("Integration Test".to_string()),
            author: Some("Test Author".to_string()),
            date: None,
        },
        pages: vec![
            Page {
                elements: vec![
                    Element::Heading { level: 1, text: "Chapter 1".to_string() },
                    Element::Paragraph {
                        text: RichText {
                            segments: vec![
                                TextSegment {
                                    text: "This is ".to_string(),
                                    bold: false, italic: false, code: false, link: None,
                                },
                                TextSegment {
                                    text: "bold".to_string(),
                                    bold: true, italic: false, code: false, link: None,
                                },
                                TextSegment {
                                    text: " text.".to_string(),
                                    bold: false, italic: false, code: false, link: None,
                                },
                            ],
                        },
                    },
                    Element::CodeBlock {
                        language: Some("rust".to_string()),
                        code: "fn hello() {\n    println!(\"Hello!\");\n}".to_string(),
                    },
                    Element::List {
                        ordered: false,
                        items: vec![
                            ListItem {
                                text: RichText { segments: vec![TextSegment { text: "Item A".to_string(), bold: false, italic: false, code: false, link: None }] },
                                children: vec![],
                            },
                            ListItem {
                                text: RichText { segments: vec![TextSegment { text: "Item B".to_string(), bold: false, italic: false, code: false, link: None }] },
                                children: vec![],
                            },
                        ],
                    },
                ],
            },
            Page {
                elements: vec![
                    Element::Table {
                        headers: vec!["Col1".to_string(), "Col2".to_string()],
                        rows: vec![vec!["A".to_string(), "B".to_string()]],
                    },
                    Element::HorizontalRule,
                    Element::BlockQuote {
                        text: RichText { segments: vec![TextSegment { text: "A wise quote".to_string(), bold: false, italic: false, code: false, link: None }] },
                    },
                ],
            },
        ],
    };

    let opts = ConvertOptions::default();
    let md = MarkdownRenderer::render(&doc, &opts);

    assert!(md.contains("# Integration Test"));
    assert!(md.contains("**Author:** Test Author"));
    assert!(md.contains("# Chapter 1"));
    assert!(md.contains("**bold**"));
    assert!(md.contains("```rust"));
    assert!(md.contains("- Item A"));
    assert!(md.contains("| Col1 | Col2 |"));
    assert!(md.contains("> A wise quote"));
    assert!(md.contains("<!-- page 2 -->"));
}
```

**Step 2: Run tests**

Run: `cargo test --test integration_test`
Expected: 2 tests PASS

**Step 3: Run all tests**

Run: `cargo test`
Expected: All tests PASS

**Step 4: Commit**

```bash
mkdir -p tests/fixtures
git add tests/integration_test.rs tests/fixtures/
git commit -m "feat: add integration tests for full pipeline"
```

---

### Task 12: Final Verification and Cleanup

**Step 1: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No errors (fix any that appear)

**Step 2: Run fmt**

Run: `cargo fmt`
Expected: Code formatted

**Step 3: Run all tests one final time**

Run: `cargo test`
Expected: All tests PASS

**Step 4: Final commit**

```bash
git add -A
git commit -m "chore: clippy fixes and formatting"
```

---

**Plan complete and saved to `docs/plans/2026-03-10-any2md-implementation.md`. Two execution options:**

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

**Which approach?**
