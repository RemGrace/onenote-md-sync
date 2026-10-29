# New Converters Design — Audio, Website, Image OCR

**Date:** 2026-03-10
**Status:** Implemented

---

## Converter 1: Audio → Markdown

**Input:** Audio files (`.wav`, `.mp3`, `.m4a`, `.ogg`, `.webm`, `.flac`) OR live mic (`--live`)

**CLI:**
```bash
# File mode
any2md --audio recording.mp3 -o notes.md
any2md --audio meeting.wav --engine cloud -o notes.md

# Live mode — records from mic, Enter to stop, outputs to stdout
any2md --audio --live
```

**Behavior:**
- **File mode:** Transcribes file → writes `.md` with speaker-detected timestamped sections
- **Live mode:** Records from default mic, Enter key stops (max 1 hour), transcribes, prints markdown to stdout. Local engine only.
- **Engine:** `--engine local` (default, whisper.cpp via `whisper-rs`) or `--engine cloud` (OpenAI Whisper API, requires `OPENAI_API_KEY`)
- **Model management:** Auto-downloads `base` model to `~/.any2md/models/ggml-base.bin` on first use (~148MB). Override with `--model path/to/model.bin`. Download integrity verified by file size.
- **Language:** Auto-detected by Whisper (supports 99 languages)
- **Speaker detection:** Pause-based heuristic (>2s gap alternates Speaker 1/Speaker 2). Not real diarization.

**Module:** `src/converter/audio/mod.rs`
**Crates:** `whisper-rs`, `cpal`, `symphonia`, `reqwest`

---

## Converter 2: Website → Markdown

**Input:** URL via `--url` flag

**CLI:**
```bash
any2md --url https://example.com/article -o article.md
any2md --url https://blog.com/post --images inline -o post.md
```

**Behavior:**
- Plain HTTP fetch (no JS rendering)
- Reader-mode extraction: tries `<article>`, `<main>`, `[role="main"]`, falls back to largest `<div>`
- Strips: `<nav>`, `<footer>`, `<header>`, `<aside>`, `<script>`, `<style>`, `<noscript>`
- Preserves: headings, paragraphs (with bold/italic/code/links), lists (with nesting), tables, code blocks, blockquotes, images, horizontal rules
- Metadata from `<title>`, `<meta name="author">`, `<meta name="date">`, `<time datetime>`
- Images downloaded in both Extract and Inline modes

**Security:**
- SSRF protection: blocks private IPs, localhost, non-HTTP schemes
- HTTP timeouts: 10s connect, 30s total, max 5 redirects
- Size limits: 50MB HTML, 10MB per image
- Recursion depth: 100 levels max

**Module:** `src/converter/web/mod.rs`
**Crates:** `reqwest`, `scraper`

---

## Converter 3: Image OCR → Markdown

**Input:** Single image file (`.png`, `.jpg`, `.jpeg`, `.tiff`, `.bmp`, `.webp`)

**CLI:**
```bash
# Local Tesseract CLI
any2md scan.png -o text.md

# Cloud OCR via OpenAI Vision API
any2md photo.jpg --engine cloud -o text.md
```

**Behavior:**
- **Local:** Calls `tesseract` CLI (not linked as library — no build-time C dependency)
- **Cloud:** Sends image to OpenAI GPT-4o vision API (max 20MB file size)
- Flat paragraphs output (no structure detection in v1)
- Error handling: clear message if tesseract not installed

**Module:** `src/converter/image_ocr/mod.rs`
**Crates:** `base64`, `reqwest`, `serde_json` (for cloud mode)

---

## Architecture

### Dispatch (main.rs)

```
--audio --live  → AudioConverter::convert_live()
--url           → WebConverter::convert_url()
--audio <file>  → AudioConverter::convert_file()
.png/.jpg/...   → ImageOcrConverter::convert_with_engine()
.pdf            → ConverterRegistry → PdfConverter::convert()
                        ↓
                   Document (unified model)
                        ↓
                   MarkdownRenderer → .md file
```

### Module structure

```
src/converter/
  audio/mod.rs      — AudioConverter, AudioOptions, AudioEngine
  web/mod.rs        — WebConverter (standalone, not trait-based)
  image_ocr/mod.rs  — ImageOcrConverter, OcrEngine (standalone)
  pdf/              — PdfConverter (trait-based via ConverterRegistry)
    mod.rs          — Pipeline orchestration
    extractor.rs    — Raw PDF extraction + two-phase merge
    table_detector.rs — Text-edge column detection
    classifier.rs   — Block type classification
    assembler.rs    — Document assembly + merge logic
```

### Error types

```rust
ConvertError::FileNotFound       // Missing input file
ConvertError::UnsupportedFormat  // Unknown file extension
ConvertError::CorruptedFile      // Corrupted/invalid input file
ConvertError::ImageExtractionFailed // OCR/image processing failure
ConvertError::NetworkError       // HTTP/API failures
ConvertError::TranscriptionError // Whisper/audio/device failures
ConvertError::IoError            // File system errors
```
