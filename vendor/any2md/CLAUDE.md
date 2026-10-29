# any2md

Rust CLI for converting PDF, websites, images (OCR), and audio to Markdown.

## Commands

| Command | Description |
|---------|-------------|
| `cargo build --release` | Build release binary → `target/release/any2md` |
| `cargo test` | Run all tests (~130) |
| `cargo test <name>` | Run specific test by name |
| `cargo clippy -- -W clippy::all` | Lint (must pass before commit) |
| `cargo fmt` | Format code |
| `cargo fmt -- --check` | Check formatting without modifying |

## Architecture

Entry point: `src/main.rs` → CLI parsing (clap derive) → converter dispatch

```
src/
├── main.rs              # CLI args, logging setup, dispatch logic
├── lib.rs               # Public API re-exports
├── error.rs             # ConvertError enum (thiserror)
├── converter/
│   ├── mod.rs           # Converter trait + ConverterRegistry
│   ├── pdf/
│   │   ├── mod.rs       # PdfConverter (orchestrates 4 phases)
│   │   ├── extractor.rs # Phase 1: raw text/image extraction from PDF streams
│   │   ├── table_detector.rs # Phase 2: column-alignment table detection
│   │   ├── classifier.rs    # Phase 3: heading/code/list/paragraph classification
│   │   └── assembler.rs     # Phase 4: merge + assemble final elements
│   ├── web/mod.rs       # WebConverter: fetch → reader-mode → DOM walk
│   ├── audio/mod.rs     # AudioConverter: file/live → whisper/cloud → speaker detect
│   └── image_ocr/mod.rs # ImageOcrConverter: tesseract CLI or OpenAI Vision
├── model/
│   ├── document.rs      # Document, Element, RichText, Metadata types
│   └── options.rs       # ConvertOptions, ImageMode, PageMode
└── renderer/
    └── markdown.rs      # MarkdownRenderer: Document → String
```

### Key patterns

- **Converter trait**: `fn convert(&self, input: &Path, options: &ConvertOptions) -> Result<Document, ConvertError>`
- **ConverterRegistry**: `Vec<Box<dyn Converter>>`, lookup by file extension (case-insensitive)
- **Dispatch**: Audio/web/image bypass registry with direct calls; PDF uses registry
- **Document model**: Unified intermediate representation for all converters

## Conventions

- Error handling: `thiserror` with `ConvertError` enum, propagate with `?`, wrap with `map_err()` for context
- Logging: `tracing` crate (`debug!`, `info!`, `warn!`), enabled with `--debug` flag
- CLI args: `clap` derive macros in `main.rs`
- Naming: PascalCase types, snake_case functions, UPPER_SNAKE_CASE constants
- Visibility: Public API via `pub`, helpers stay private, no `pub(crate)` used
- Section markers in code: `// ── Section ────────────────`
- Tests: unit tests in `#[cfg(test)]` modules within source files + integration tests in `tests/`
- Constants at module top with descriptive names

## Testing

- Unit tests: inside each converter module (`#[cfg(test)]`)
- Integration tests: `tests/` directory (converter_tests, integration_test, model_tests, renderer_tests)
- Helper patterns: `plain_text()` in renderer_tests, `MockConverter` in converter_tests
- Run single test file: `cargo test --test integration_test`

## Environment Variables

| Variable | When needed |
|----------|-------------|
| `OPENAI_API_KEY` | Cloud audio transcription (`--engine cloud`) and cloud OCR |

## Gotchas

- Audio converter requires `cmake` at build time (whisper-rs → whisper.cpp)
- Image OCR local mode needs `tesseract` installed on system
- `--pages split` is accepted but not implemented yet
- Live mic recording only supports local engine, not cloud
- PDF extractor (`extractor.rs`) is ~1300 lines — largest file, handle with care
- Whisper model auto-downloads (~148MB) to `~/.any2md/models/` on first use
- No `pub(crate)` — PDF submodules are all `pub` for sibling access
