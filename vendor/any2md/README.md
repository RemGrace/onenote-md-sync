# any2md

CLI utility in Rust for converting various sources to Markdown. Supports PDF files, websites, images (OCR), and audio transcription.

## Installation

### Download Pre-built Binaries

Download the latest release from the [Releases page](https://github.com/RemGrace/any2md/releases/latest).

| Platform | File |
|----------|------|
| macOS (Apple Silicon & Intel) | `any2md-vX.Y.Z-macos-universal.tar.gz` |
| Linux x86_64 | `any2md-vX.Y.Z-linux-x86_64.tar.gz` |
| Windows x86_64 | `any2md-vX.Y.Z-windows-x86_64.zip` |

```bash
# macOS / Linux
tar xzf any2md-*.tar.gz
chmod +x any2md
sudo mv any2md /usr/local/bin/

# Windows (PowerShell)
Expand-Archive any2md-*.zip -DestinationPath .
# Move any2md.exe to a directory in your PATH
```

### Build from Source

#### Prerequisites

| Feature | Requirement |
|---------|-------------|
| PDF | None (built-in) |
| Image OCR (local) | [Tesseract](https://github.com/tesseract-ocr/tesseract) installed |
| Image OCR (cloud) | `OPENAI_API_KEY` environment variable |
| Audio (local) | Auto-downloads Whisper model on first use. Requires `cmake` at build time. |
| Audio (cloud) | `OPENAI_API_KEY` environment variable |
| Website | None (built-in) |

```bash
# macOS
brew install tesseract cmake

# Ubuntu/Debian
sudo apt install tesseract-ocr cmake

# Build
cargo build --release
```

The binary will be at `target/release/any2md`.

## Usage

```
any2md [OPTIONS] [INPUT]

Arguments:
  [INPUT]  Input file path (not required with --url or --audio --live)

Options:
  -o, --output <path>            Output file (default: <input_name>.md)
      --images <extract|inline>  Image mode (default: extract)
      --pages <single|split>     Page mode (default: single)
      --url <URL>                Convert a webpage to markdown
      --audio                    Audio transcription mode
      --live                     Live microphone recording (use with --audio)
      --engine <local|cloud>     Engine for OCR/transcription (default: local)
      --model <path>             Path to Whisper model file (default: auto-download)
      --debug                    Enable debug logging to console and file
      --log-file <path>          Path for debug log file (default: any2md.log)
  -h, --help                     Help
```

### PDF to Markdown

Convert PDF documents to structured Markdown with headings, tables, lists, code blocks, bold/italic formatting, and images.

```bash
# Basic conversion (output: document.md)
any2md document.pdf

# Custom output path
any2md document.pdf -o output/result.md

# Embed images as base64 data URIs instead of saving to files
any2md document.pdf --images inline

# Debug mode — see extraction details in stderr and any2md.log
any2md document.pdf --debug
```

**What it does:**
- Extracts text blocks with position, font, and size from PDF content streams
- Detects tables via column alignment analysis (text-edge detection algorithm)
- Classifies blocks as headings (by font size), code (by monospace font), lists (by markers), or paragraphs
- Merges consecutive headings, code blocks, and list items
- Extracts embedded images and saves them to `images/` directory
- Extracts document metadata (title, author, date) from PDF info dictionary

**Output structure:**
```markdown
# Document Title

**Author:** John Doe
**Date:** 2026-03-10

## Section Heading

Regular paragraph text with **bold** and *italic* formatting.

| Column 1 | Column 2 | Column 3 |
| --- | --- | --- |
| Data | Data | Data |

- List item one
- List item two

![image](images/img_1.png)
```

---

### Website to Markdown

Convert any webpage to clean Markdown using reader-mode content extraction.

```bash
# Basic — fetches page and extracts article content
any2md --url https://example.com/article -o article.md

# With inline images (base64 embedded)
any2md --url https://blog.com/post --images inline -o post.md

# Default output file is page.md when no -o specified
any2md --url https://docs.example.com/guide
```

**What it does:**
- Fetches the HTML page via HTTP GET (with timeouts and redirect limits)
- Finds the main content using reader-mode heuristics:
  - Tries `<article>`, `<main>`, `[role="main"]` first
  - Falls back to the `<div>` with the most text content
- Strips non-content elements: `<nav>`, `<footer>`, `<header>`, `<aside>`, `<script>`, `<style>`
- Converts HTML elements to Markdown:
  - `<h1>`-`<h6>` → headings
  - `<p>` → paragraphs with inline formatting (`<strong>` → bold, `<em>` → italic, `<code>` → code, `<a>` → links)
  - `<ul>`/`<ol>` → lists (with nesting support)
  - `<table>` → Markdown tables
  - `<pre><code>` → fenced code blocks (with language detection from CSS classes)
  - `<blockquote>` → blockquotes
  - `<img>` → downloaded and saved to `images/` directory
- Extracts metadata from `<title>`, `<meta name="author">`, `<meta name="date">`, `<time datetime>`

**Security notes:**
- URLs are validated before fetching — private IPs (10.x, 172.16.x, 192.168.x, 127.x, 169.254.x), localhost, and non-HTTP schemes are blocked
- HTML responses capped at 50MB, individual images at 10MB
- Maximum 5 redirects, 10s connection timeout, 30s total timeout

**Limitations:**
- No JavaScript rendering — single-page apps (SPAs) that require JS will return empty content
- No cookie/session handling — pages behind login won't work

---

### Image OCR to Markdown

Extract text from images using OCR (Optical Character Recognition).

```bash
# Local engine — requires tesseract installed on system
any2md photo.png -o text.md
any2md screenshot.jpg -o extracted.md

# Cloud engine — requires OPENAI_API_KEY env var
export OPENAI_API_KEY=sk-...
any2md scan.tiff --engine cloud -o text.md
```

**Supported formats:** `.png`, `.jpg`, `.jpeg`, `.tiff`, `.bmp`, `.webp`

**Local engine (default):**
- Calls the `tesseract` command-line tool (must be installed separately)
- Language: English by default (`eng`)
- If tesseract is not found, shows installation instructions:
  ```
  Error: Tesseract not found. Install it:
  brew install tesseract (macOS) or apt install tesseract-ocr (Linux)
  ```

**Cloud engine (`--engine cloud`):**
- Sends the image to OpenAI's GPT-4o vision model
- Requires `OPENAI_API_KEY` environment variable
- Maximum file size: 20MB
- Better accuracy on complex layouts, handwriting, and non-English text

**Output:** Plain paragraphs of extracted text (no structure detection in v1 — headings, tables, and lists are not detected from images).

---

### Audio to Markdown

Transcribe audio files or live microphone input to Markdown with timestamped speaker sections.

```bash
# Transcribe an audio file with local Whisper engine
any2md --audio recording.mp3 -o notes.md
any2md --audio lecture.wav -o lecture.md
any2md --audio podcast.m4a -o transcript.md

# Transcribe with OpenAI cloud engine
export OPENAI_API_KEY=sk-...
any2md --audio meeting.wav --engine cloud -o meeting.md

# Use a custom Whisper model
any2md --audio recording.mp3 --model ~/models/ggml-large.bin -o notes.md

# Live microphone recording
any2md --audio --live
# → Records until you press Enter
# → Transcribes and prints markdown to stdout
```

**Supported audio formats:** `.wav`, `.mp3`, `.m4a`, `.ogg`, `.webm`, `.flac`

**Local engine (default):**
- Uses [whisper.cpp](https://github.com/ggerganov/whisper.cpp) via `whisper-rs` bindings
- **First run:** Automatically downloads the Whisper `base` model (~148MB) to `~/.any2md/models/ggml-base.bin`
- **Language:** Auto-detected (supports 99 languages)
- **Custom model:** Override with `--model /path/to/ggml-model.bin` (supports any GGML Whisper model)
- No internet connection required after model download

**Cloud engine (`--engine cloud`):**
- Uses OpenAI Whisper API (`whisper-1` model)
- Requires `OPENAI_API_KEY` environment variable
- Faster processing, handles more formats

**Live mode (`--audio --live`):**
- Records from default system microphone
- Press **Enter** to stop recording (maximum: 1 hour)
- Transcribes the recording and prints Markdown to stdout
- Only works with local engine (not cloud)
- Requires a working audio input device

**Speaker detection:**
- Uses pause-based heuristic: a gap > 2 seconds between speech segments triggers a speaker change
- Alternates between "Speaker 1" and "Speaker 2"
- This is a simple heuristic, not real speaker diarization — it works best for two-person conversations

**Output structure:**
```markdown
## [00:00 - 00:45] Speaker 1
Hello, welcome to the meeting. Today we'll discuss the roadmap for the next quarter.

## [00:45 - 01:20] Speaker 2
Thanks. I think we should prioritize the mobile app first, since most of our users are on mobile.

## [01:20 - 02:05] Speaker 1
Good point. Let me pull up the metrics from last month.
```

## Supported Formats

| Format | Engine | Notes |
|--------|--------|-------|
| PDF | Built-in (`lopdf`) | 4-phase pipeline: extract, detect tables, classify, assemble |
| Website | `reqwest` + `scraper` | Reader-mode extraction, SSRF protection |
| Image OCR | Tesseract CLI / OpenAI Vision | Local or cloud via `--engine` flag |
| Audio | Whisper.cpp / OpenAI Whisper API | Local or cloud, file or live mic |

## Architecture

```
CLI (main.rs)
  ├── --url        → WebConverter::convert_url()
  ├── --audio      → AudioConverter::convert_file() / convert_live()
  ├── .png/.jpg/…  → ImageOcrConverter::convert_with_engine()
  └── .pdf         → PdfConverter::convert() via ConverterRegistry
                        ↓
                   Document (unified model)
                        ↓
                   MarkdownRenderer → .md file
```

### PDF Pipeline (4 phases)

1. **Extraction** — Parses PDF content streams via `lopdf`. Extracts text blocks with position, font, size. Extracts embedded images. Two-phase merge: fix_end_x pass corrects short-block widths, then gap-based merging assembles text lines.
2. **Table Detection** — Text-edge column detection (Nurminen/Tabula algorithm). Identifies grid-aligned tabular data before line assembly.
3. **Classification** — Heuristics classify blocks: code (monospace font), heading (large font size), list (bullet/number markers), paragraph (default). Bold/italic from font names.
4. **Assembly** — Merges consecutive headings, code blocks, list items. URL continuation detection. Tables interleaved at correct Y positions.

### Web Pipeline

1. **Fetch** — HTTP GET with timeout, redirect limits, SSRF validation
2. **Content extraction** — Reader mode: finds `<article>`, `<main>`, or largest `<div>`. Strips nav, footer, scripts.
3. **DOM walking** — Converts HTML elements to Document model with inline formatting (bold, italic, code, links)
4. **Image download** — Downloads images with size limits (10MB per image)

### Security

- **SSRF protection**: URL validation blocks private IPs, localhost, non-HTTP schemes
- **HTTP timeouts**: All network requests have connect and read timeouts
- **Response size limits**: HTML (50MB), images (10MB), OCR uploads (20MB)
- **Model integrity**: Whisper model download verified by file size range
- **No command injection**: Tesseract called via `std::process::Command` (not shell)
- **Recursion limits**: DOM walker capped at 100 levels depth

## Debug Logging

Pass `--debug` to enable detailed logging. Logs go to stderr (colored) and a log file (default: `any2md.log`).

```bash
any2md document.pdf --debug
any2md document.pdf --debug --log-file /tmp/debug.log
```

## Known Limitations

- **Split pages mode**: `--pages split` is accepted but not yet implemented
- **Password-protected PDFs**: Fails with a generic parse error
- **Audio speaker detection**: Simple pause-based heuristic (2 speakers max), not real diarization
- **Audio live mode**: Only supports local Whisper engine, not cloud
- **Website JS rendering**: Plain HTTP fetch only, no headless browser (SPAs won't work)
- **Image OCR structure**: Flat paragraphs only (v1), no heading/table detection from images
- **Large files**: Audio and images are read fully into memory for cloud upload

## Development

```bash
# Run all tests (130 tests)
cargo test

# Lint
cargo clippy -- -W clippy::all

# Format
cargo fmt

# Build release
cargo build --release
```

## Releasing

To publish a new release:

```bash
# Tag with a version
git tag v0.2.0
git push origin v0.2.0
```

This triggers the release workflow which builds binaries for all platforms and creates a GitHub Release with the artifacts.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `lopdf` | PDF parsing |
| `whisper-rs` | Local speech-to-text (whisper.cpp bindings) |
| `cpal` | Cross-platform audio capture |
| `symphonia` | Audio format decoding (MP3, OGG, FLAC, WAV, AAC) |
| `reqwest` | HTTP client (web fetch, cloud APIs) |
| `scraper` | HTML DOM parsing |
| `clap` | CLI argument parsing |
| `tracing` | Structured logging |
| `serde_json` | JSON parsing for cloud API responses |
| `base64` | Base64 encoding for inline images and cloud OCR |
| `dirs` | Home directory resolution for model storage |
