#[cfg(feature = "audio")]
use any2md::converter::audio::{AudioConverter, AudioEngine, AudioOptions};
use any2md::converter::image_ocr::{ImageOcrConverter, OcrEngine};
use any2md::converter::pdf::PdfConverter;
use any2md::converter::web::WebConverter;
use any2md::converter::ConverterRegistry;
use any2md::model::options::{ConvertOptions, ImageMode, PageMode};
use any2md::renderer::markdown::MarkdownRenderer;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process;
use tracing::{debug, info};

#[derive(Parser)]
#[command(name = "any2md", about = "Convert files to Markdown")]
struct Cli {
    /// Input file path (not required when using --url or --audio --live)
    input: Option<PathBuf>,

    /// Output file path (default: <input_name>.md)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Image mode: extract or inline
    #[arg(long, default_value = "extract")]
    images: String,

    /// Page mode: single or split
    #[arg(long, default_value = "single")]
    pages: String,

    /// Enable debug logging to console and file (any2md.log)
    #[arg(long)]
    debug: bool,

    /// Path for debug log file (default: any2md.log)
    #[arg(long, default_value = "any2md.log")]
    log_file: PathBuf,

    /// Convert a URL to markdown
    #[arg(long)]
    url: Option<String>,

    /// Audio mode: transcribe audio file or live mic input
    #[arg(long)]
    audio: bool,

    /// Live microphone recording mode (use with --audio)
    #[arg(long)]
    live: bool,

    /// Transcription/OCR engine: local or cloud
    #[arg(long, default_value = "local")]
    engine: String,

    /// Path to Whisper model file (default: auto-download to ~/.any2md/models/)
    #[arg(long)]
    model: Option<PathBuf>,
}

fn setup_logging(debug: bool, log_file: &Path) {
    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    if !debug {
        return;
    }

    let filter = EnvFilter::new("debug");

    let stderr_layer = fmt::layer()
        .with_target(true)
        .with_writer(std::io::stderr)
        .with_ansi(true);

    let file = std::fs::File::create(log_file).unwrap_or_else(|e| {
        eprintln!(
            "Warning: could not create log file '{}': {}",
            log_file.display(),
            e
        );
        process::exit(1);
    });
    let file_layer = fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(file);

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();
}

fn main() {
    let cli = Cli::parse();

    setup_logging(cli.debug, &cli.log_file);

    let image_mode = match cli.images.as_str() {
        "extract" => ImageMode::Extract,
        "inline" => ImageMode::Inline,
        other => {
            eprintln!(
                "Error: unknown image mode '{}'. Use 'extract' or 'inline'.",
                other
            );
            process::exit(1);
        }
    };

    let page_mode = match cli.pages.as_str() {
        "single" => PageMode::SingleFile,
        "split" => PageMode::SplitPages,
        other => {
            eprintln!(
                "Error: unknown page mode '{}'. Use 'single' or 'split'.",
                other
            );
            process::exit(1);
        }
    };

    #[cfg(feature = "audio")]
    let audio_engine = match cli.engine.as_str() {
        "local" => AudioEngine::Local,
        "cloud" => AudioEngine::Cloud,
        other => {
            eprintln!("Error: unknown engine '{}'. Use 'local' or 'cloud'.", other);
            process::exit(1);
        }
    };

    // --- Dispatch: Audio live mode (no file input) ---
    if cli.audio && cli.live {
        #[cfg(feature = "audio")]
        {
            eprintln!("🎙  Starting live recording...");
            let audio_opts = AudioOptions {
                engine: audio_engine,
                model_path: cli.model,
            };
            match AudioConverter::convert_live(&audio_opts) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            }
            return;
        }
        #[cfg(not(feature = "audio"))]
        {
            eprintln!(
                "Error: audio support was not compiled in. \
                 Rebuild with: cargo build --release --features audio"
            );
            process::exit(1);
        }
    }

    // --- Dispatch: URL mode ---
    if let Some(ref url) = cli.url {
        let output_path = cli.output.unwrap_or_else(|| PathBuf::from("page.md"));
        let image_output_dir = image_dir_for(&output_path);
        let options = ConvertOptions {
            image_mode,
            page_mode,
            image_output_dir,
        };

        eprintln!("🌐 Fetching {}...", url);
        let doc = match WebConverter::convert_url(url, &options) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        };

        render_and_write(&doc, &options, &output_path);
        return;
    }

    // --- All other modes require an input file ---
    let input = match cli.input {
        Some(ref p) => p,
        None => {
            eprintln!("Error: input file is required (or use --url / --audio --live)");
            process::exit(1);
        }
    };

    let output_path = cli.output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default();
        PathBuf::from(format!("{}.md", stem.to_string_lossy()))
    });

    let image_output_dir = image_dir_for(&output_path);

    let options = ConvertOptions {
        image_mode,
        page_mode,
        image_output_dir,
    };

    debug!(input = %input.display(), "Starting conversion");
    debug!(?options, "Conversion options");

    // --- Dispatch: Audio file mode ---
    if cli.audio {
        #[cfg(feature = "audio")]
        {
            let audio_opts = AudioOptions {
                engine: audio_engine,
                model_path: cli.model,
            };

            eprintln!("🎵 Transcribing {}...", input.display());
            let doc = match AudioConverter::convert_file(input, &audio_opts) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    process::exit(1);
                }
            };

            render_and_write(&doc, &options, &output_path);
            return;
        }
        #[cfg(not(feature = "audio"))]
        {
            eprintln!(
                "Error: audio support was not compiled in. \
                 Rebuild with: cargo build --release --features audio"
            );
            process::exit(1);
        }
    }

    // --- Dispatch: File-based converters (PDF, Image OCR) ---
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Check if this is an image for OCR (handle engine flag)
    let image_extensions = ["png", "jpg", "jpeg", "tiff", "bmp", "webp"];
    let is_image = image_extensions.iter().any(|e| e.eq_ignore_ascii_case(ext));
    if is_image {
        let ocr_engine = match cli.engine.as_str() {
            "local" => OcrEngine::Local,
            "cloud" => OcrEngine::Cloud,
            _ => OcrEngine::Local,
        };
        eprintln!(
            "🖼  Converting {} (OCR {:?})...",
            input.display(),
            ocr_engine
        );
        let doc = match ImageOcrConverter::convert_with_engine(input, &options, ocr_engine) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        };
        render_and_write(&doc, &options, &output_path);
        return;
    }

    // Local HTML file: the web converter without the network. `--url` cannot do
    // this — validate_url blocks file://, localhost and private IPs, so there is
    // no way to hand a local page to the HTML pipeline otherwise.
    let html_extensions = ["html", "htm"];
    if html_extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
        eprintln!("🌐 Converting {} (local HTML)...", input.display());
        let html_text = match std::fs::read_to_string(input) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error: cannot read {}: {}", input.display(), e);
                process::exit(1);
            }
        };
        let doc = match WebConverter::convert_html(&html_text, None, &options) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        };
        render_and_write(&doc, &options, &output_path);
        return;
    }

    // Other file-based converters (PDF, etc.)
    let mut registry = ConverterRegistry::new();
    registry.register(Box::new(PdfConverter));

    let converter = match registry.find_by_extension(ext) {
        Some(c) => c,
        None => {
            eprintln!("Error: unsupported format '.{}'", ext);
            process::exit(1);
        }
    };

    info!(
        converter = converter.name(),
        extension = ext,
        "Found converter"
    );
    eprintln!("📄 Converting {}...", input.display());

    let doc = match converter.convert(input, &options) {
        Ok(d) => {
            info!(
                pages = d.pages.len(),
                title = ?d.metadata.title,
                "Conversion complete"
            );
            d
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    };

    render_and_write(&doc, &options, &output_path);
}

/// Image directory for one document: `<output_stem>_images/` next to the .md,
/// so two conversions in the same folder never overwrite each other's images.
fn image_dir_for(output_path: &Path) -> PathBuf {
    let stem = output_path.file_stem().unwrap_or_default();
    output_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{}_images", stem.to_string_lossy()))
}

fn render_and_write(
    doc: &any2md::model::document::Document,
    options: &ConvertOptions,
    output_path: &Path,
) {
    debug!("Rendering document to markdown");

    let markdown = match MarkdownRenderer::render(doc, options) {
        Ok(md) => {
            debug!(output_bytes = md.len(), "Rendering complete");
            md
        }
        Err(e) => {
            eprintln!("Error rendering: {}", e);
            process::exit(1);
        }
    };

    match std::fs::write(output_path, &markdown) {
        Ok(_) => {
            info!(output = %output_path.display(), "Written successfully");
            eprintln!("✅ Written to {}", output_path.display());
        }
        Err(e) => {
            eprintln!("Error writing output: {}", e);
            process::exit(1);
        }
    }
}
