//! Audio-to-Markdown converter.
//!
//! Supports two modes:
//! - **File mode:** Transcribe audio files (`.wav`, `.mp3`, `.m4a`, `.ogg`, `.webm`, `.flac`)
//!   to markdown with speaker-detected timestamped sections.
//! - **Live mode:** Record from microphone, stop with Enter key, stream transcribed text to stdout.
//!
//! Two engines:
//! - `local` (default): whisper.cpp via `whisper-rs`
//! - `cloud`: OpenAI Whisper API via `reqwest`

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::ConvertError;
use crate::model::document::{Document, Element, Metadata, Page, RichText, TextSegment};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Engine selection for audio transcription.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AudioEngine {
    /// Local whisper.cpp inference via `whisper-rs`.
    #[default]
    Local,
    /// OpenAI Whisper API (requires `OPENAI_API_KEY` env var).
    Cloud,
}

/// Options controlling how audio is transcribed.
#[derive(Debug, Default, Clone)]
pub struct AudioOptions {
    /// Which engine to use for transcription.
    pub engine: AudioEngine,
    /// Optional explicit path to a GGML whisper model file.
    /// When `None`, the default `~/.any2md/models/ggml-base.bin` is used (auto-downloaded).
    pub model_path: Option<PathBuf>,
}

/// Supported audio file extensions.
const SUPPORTED_EXTENSIONS: &[&str] = &["wav", "mp3", "m4a", "ogg", "webm", "flac"];

/// Minimum pause in seconds between segments to trigger a speaker change.
const SPEAKER_CHANGE_PAUSE_SECS: f64 = 2.0;

/// Default Hugging Face URL for the whisper base model.
const MODEL_DOWNLOAD_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";

/// Default model filename.
const MODEL_FILENAME: &str = "ggml-base.bin";

/// Expected minimum file size for ggml-base.bin (140 MB).
const MODEL_MIN_SIZE: u64 = 140 * 1024 * 1024;

/// Expected maximum file size for ggml-base.bin (160 MB).
const MODEL_MAX_SIZE: u64 = 160 * 1024 * 1024;

/// Maximum recording duration in seconds (1 hour).
const MAX_RECORDING_SECONDS: u64 = 3600;

/// Timeout for model download requests (5 minutes).
const MODEL_DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Timeout for API requests (2 minutes).
const API_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Transcription segment (internal)
// ---------------------------------------------------------------------------

/// A single transcription segment returned by either engine.
#[derive(Debug, Clone)]
struct TranscriptionSegment {
    /// Start time in seconds.
    start: f64,
    /// End time in seconds.
    end: f64,
    /// Transcribed text.
    text: String,
}

// ---------------------------------------------------------------------------
// Speaker section (internal)
// ---------------------------------------------------------------------------

/// A group of segments attributed to the same speaker.
#[derive(Debug, Clone)]
struct SpeakerSection {
    speaker: String,
    start: f64,
    end: f64,
    text: String,
}

// ---------------------------------------------------------------------------
// AudioConverter
// ---------------------------------------------------------------------------

/// Converts audio files (or live microphone input) to Markdown via whisper transcription.
pub struct AudioConverter;

impl AudioConverter {
    /// Transcribe an audio file to a [`Document`].
    ///
    /// The file must have one of the supported extensions: wav, mp3, m4a, ogg, webm, flac.
    pub fn convert_file(path: &Path, options: &AudioOptions) -> Result<Document, ConvertError> {
        // 1. Verify the file exists.
        if !path.exists() {
            return Err(ConvertError::FileNotFound(path.to_path_buf()));
        }

        // 2. Verify the extension is supported.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !is_supported_extension(&ext) {
            return Err(ConvertError::UnsupportedFormat(format!(
                "Unsupported audio format: .{ext}"
            )));
        }

        tracing::info!("Transcribing audio file: {}", path.display());

        // 3. Transcribe via the selected engine.
        eprintln!("🔍 Transcribing ({:?} engine)...", options.engine);
        let segments = match options.engine {
            AudioEngine::Local => transcribe_local(path, &options.model_path)?,
            AudioEngine::Cloud => transcribe_cloud(path)?,
        };
        eprintln!("✅ Transcription complete ({} segments).", segments.len());

        // 4. Detect speaker sections.
        eprintln!("👥 Detecting speakers...");
        let sections = detect_speakers(&segments);

        // 5. Build Document.
        let title = path.file_name().and_then(|n| n.to_str()).map(String::from);

        let doc = build_document(title, &sections);
        Ok(doc)
    }

    /// Live microphone capture — records audio, stops when Enter is pressed,
    /// transcribes via the local engine, and prints the resulting markdown to stdout.
    pub fn convert_live(options: &AudioOptions) -> Result<(), ConvertError> {
        // H-9: Cloud engine is not supported for live recording mode.
        if options.engine == AudioEngine::Cloud {
            return Err(ConvertError::TranscriptionError(
                "Cloud engine is not supported for live recording mode. Use --engine local."
                    .to_string(),
            ));
        }

        tracing::info!("Starting live audio capture. Press Enter to stop recording...");

        let samples = capture_mic_until_enter()?;

        if samples.is_empty() {
            tracing::warn!("No audio samples captured.");
            println!("*(no audio captured)*");
            return Ok(());
        }

        let duration_secs = samples.len() as f64 / 16000.0;
        eprintln!("🎵 Captured {:.1}s of audio.", duration_secs);

        eprintln!("📦 Loading Whisper model...");
        let model_path = ensure_model(&options.model_path)?;

        eprintln!("🔍 Transcribing audio...");
        let segments = run_whisper_inference(&model_path, &samples)?;

        eprintln!("👥 Detecting speakers...");
        let sections = detect_speakers(&segments);
        let doc = build_document(Some("Live Recording".to_string()), &sections);

        // Render to stdout using simple markdown formatting.
        print_document_as_markdown(&doc);

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Extension check
// ---------------------------------------------------------------------------

/// Returns `true` if the extension (lowercase, no dot) is a supported audio format.
fn is_supported_extension(ext: &str) -> bool {
    SUPPORTED_EXTENSIONS.contains(&ext)
}

// ---------------------------------------------------------------------------
// Model management
// ---------------------------------------------------------------------------

/// Ensure a whisper GGML model is available, downloading if necessary.
fn ensure_model(model_path: &Option<PathBuf>) -> Result<PathBuf, ConvertError> {
    if let Some(path) = model_path {
        if path.exists() {
            return Ok(path.clone());
        }
        return Err(ConvertError::FileNotFound(path.clone()));
    }

    let home = dirs::home_dir().ok_or_else(|| {
        ConvertError::TranscriptionError("Unable to determine home directory".to_string())
    })?;
    let default_dir = home.join(".any2md").join("models");
    let model_file = default_dir.join(MODEL_FILENAME);

    if model_file.exists() {
        tracing::debug!("Model found at {}", model_file.display());
        return Ok(model_file);
    }

    // Auto-download.
    std::fs::create_dir_all(&default_dir)?;
    eprintln!("📥 Downloading Whisper base model (~148 MB)...");
    eprintln!("   Destination: {}", model_file.display());
    tracing::info!(
        "Downloading Whisper base model to {} ...",
        model_file.display()
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(MODEL_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| ConvertError::NetworkError(format!("Failed to create HTTP client: {e}")))?;

    let resp = client
        .get(MODEL_DOWNLOAD_URL)
        .send()
        .map_err(|e| ConvertError::NetworkError(format!("Model download failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ConvertError::NetworkError(format!(
            "Model download returned HTTP {}",
            resp.status()
        )));
    }

    let bytes = resp
        .bytes()
        .map_err(|e| ConvertError::NetworkError(format!("Model download failed: {e}")))?;

    // C-5: Basic integrity check — verify the file size is within expected range.
    let size = bytes.len() as u64;
    if !(MODEL_MIN_SIZE..=MODEL_MAX_SIZE).contains(&size) {
        // Do not save the file.
        return Err(ConvertError::NetworkError(format!(
            "Downloaded model file size ({size} bytes) is outside expected range ({MODEL_MIN_SIZE}–{MODEL_MAX_SIZE} bytes). \
             The download may be corrupted or the model has changed."
        )));
    }

    std::fs::write(&model_file, &bytes)?;
    eprintln!(
        "✅ Model downloaded ({:.1} MB).",
        bytes.len() as f64 / 1024.0 / 1024.0
    );
    tracing::info!("Model downloaded successfully ({} bytes).", bytes.len());

    Ok(model_file)
}

// ---------------------------------------------------------------------------
// Local engine
// ---------------------------------------------------------------------------

/// Transcribe an audio file using the local whisper.cpp engine.
fn transcribe_local(
    path: &Path,
    model_path: &Option<PathBuf>,
) -> Result<Vec<TranscriptionSegment>, ConvertError> {
    let model = ensure_model(model_path)?;
    let samples = decode_audio_to_pcm(path)?;
    run_whisper_inference(&model, &samples)
}

/// Decode any supported audio file into mono f32 PCM samples at 16 kHz using symphonia.
fn decode_audio_to_pcm(path: &Path) -> Result<Vec<f32>, ConvertError> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| ConvertError::CorruptedFile(format!("Failed to probe audio format: {e}")))?;

    let mut format_reader = probed.format;

    let track = format_reader
        .default_track()
        .ok_or_else(|| ConvertError::CorruptedFile("No audio track found in file".to_string()))?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let sample_rate = codec_params.sample_rate.unwrap_or(16000);
    let _channels = codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| ConvertError::CorruptedFile(format!("Failed to create audio decoder: {e}")))?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => {
                tracing::warn!("Error reading audio packet: {e}");
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Error decoding audio packet: {e}");
                continue;
            }
        };

        let spec = *decoded.spec();
        let num_frames = decoded.capacity();
        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);

        let interleaved = sample_buf.samples();
        let ch = spec.channels.count().max(1);

        // Mix down to mono by averaging channels.
        for frame in interleaved.chunks(ch) {
            let mono: f32 = frame.iter().sum::<f32>() / ch as f32;
            all_samples.push(mono);
        }
    }

    // Resample to 16 kHz if necessary (simple linear interpolation).
    if sample_rate != 16000 && !all_samples.is_empty() {
        tracing::debug!("Resampling from {} Hz to 16000 Hz", sample_rate);
        all_samples = resample_linear(&all_samples, sample_rate, 16000);
    }

    tracing::debug!(
        "Decoded {} mono samples at 16 kHz ({:.1}s)",
        all_samples.len(),
        all_samples.len() as f64 / 16000.0
    );

    Ok(all_samples)
}

/// Simple linear interpolation resampler.
fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == to_rate {
        return input.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((input.len() as f64) / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_idx = i as f64 * ratio;
        let idx_floor = src_idx.floor() as usize;
        let frac = (src_idx - idx_floor as f64) as f32;

        let sample = if idx_floor + 1 < input.len() {
            input[idx_floor] * (1.0 - frac) + input[idx_floor + 1] * frac
        } else if idx_floor < input.len() {
            input[idx_floor]
        } else {
            0.0
        };
        output.push(sample);
    }

    output
}

/// Run whisper.cpp inference on PCM f32 samples (16 kHz mono).
fn run_whisper_inference(
    model_path: &Path,
    samples: &[f32],
) -> Result<Vec<TranscriptionSegment>, ConvertError> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let ctx = WhisperContext::new_with_params(
        model_path.to_str().ok_or_else(|| {
            ConvertError::TranscriptionError("Invalid model path encoding".into())
        })?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| ConvertError::TranscriptionError(format!("Failed to load whisper model: {e}")))?;

    let mut state = ctx.create_state().map_err(|e| {
        ConvertError::TranscriptionError(format!("Failed to create whisper state: {e}"))
    })?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    // H-10: Use None to enable Whisper auto-detection instead of hardcoding English.
    params.set_language(None);
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_token_timestamps(true);

    state
        .full(params, samples)
        .map_err(|e| ConvertError::TranscriptionError(format!("Whisper inference failed: {e}")))?;

    let n_segments = state.full_n_segments().map_err(|e| {
        ConvertError::TranscriptionError(format!("Failed to get segment count: {e}"))
    })?;

    let mut segments = Vec::new();
    for i in 0..n_segments {
        let start = state.full_get_segment_t0(i).map_err(|e| {
            ConvertError::TranscriptionError(format!("Failed to get segment start: {e}"))
        })?;
        let end = state.full_get_segment_t1(i).map_err(|e| {
            ConvertError::TranscriptionError(format!("Failed to get segment end: {e}"))
        })?;
        let text = state.full_get_segment_text(i).map_err(|e| {
            ConvertError::TranscriptionError(format!("Failed to get segment text: {e}"))
        })?;

        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }

        segments.push(TranscriptionSegment {
            // whisper-rs returns timestamps in centiseconds (hundredths of a second).
            start: start as f64 / 100.0,
            end: end as f64 / 100.0,
            text,
        });
    }

    tracing::info!("Whisper produced {} segments", segments.len());
    Ok(segments)
}

// ---------------------------------------------------------------------------
// Cloud engine
// ---------------------------------------------------------------------------

/// Transcribe an audio file using the OpenAI Whisper API.
fn transcribe_cloud(path: &Path) -> Result<Vec<TranscriptionSegment>, ConvertError> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ConvertError::TranscriptionError(
            "OPENAI_API_KEY environment variable not set (required for cloud engine)".to_string(),
        )
    })?;

    let file_bytes = std::fs::read(path)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let part = reqwest::blocking::multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("application/octet-stream")
        .map_err(|e| ConvertError::NetworkError(format!("Failed to build multipart part: {e}")))?;

    let form = reqwest::blocking::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "verbose_json")
        .text("timestamp_granularities[]", "segment")
        .part("file", part);

    let client = reqwest::blocking::Client::builder()
        .timeout(API_REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ConvertError::NetworkError(format!("Failed to create HTTP client: {e}")))?;

    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(&api_key)
        .multipart(form)
        .send()
        .map_err(|e| ConvertError::NetworkError(format!("OpenAI API request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(ConvertError::NetworkError(format!(
            "OpenAI API returned HTTP {status}: {body}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .map_err(|e| ConvertError::NetworkError(format!("Failed to parse API response: {e}")))?;

    let segments_json = body["segments"].as_array().ok_or_else(|| {
        ConvertError::NetworkError("API response missing 'segments' array".to_string())
    })?;

    let mut segments = Vec::new();
    for seg in segments_json {
        let start = seg["start"].as_f64().unwrap_or(0.0);
        let end = seg["end"].as_f64().unwrap_or(0.0);
        let text = seg["text"].as_str().unwrap_or("").trim().to_string();

        if text.is_empty() {
            continue;
        }

        segments.push(TranscriptionSegment { start, end, text });
    }

    tracing::info!("OpenAI API returned {} segments", segments.len());
    Ok(segments)
}

// ---------------------------------------------------------------------------
// Speaker detection
// ---------------------------------------------------------------------------

/// Detect speaker changes based on pauses between consecutive segments.
///
/// When the gap between the end of one segment and the start of the next exceeds
/// [`SPEAKER_CHANGE_PAUSE_SECS`], the speaker label alternates.
fn detect_speakers(segments: &[TranscriptionSegment]) -> Vec<SpeakerSection> {
    if segments.is_empty() {
        return Vec::new();
    }

    let mut sections: Vec<SpeakerSection> = Vec::new();
    let mut current_speaker_idx: usize = 0;

    for seg in segments {
        let switch = if let Some(last) = sections.last() {
            // If there is a significant pause, alternate speaker.
            (seg.start - last.end) >= SPEAKER_CHANGE_PAUSE_SECS
        } else {
            false
        };

        if switch {
            current_speaker_idx = if current_speaker_idx == 0 { 1 } else { 0 };
        }

        let speaker_label = format!("Speaker {}", current_speaker_idx + 1);

        // Merge into the last section if same speaker, otherwise create a new one.
        if let Some(last) = sections.last_mut() {
            if last.speaker == speaker_label {
                last.end = seg.end;
                last.text.push(' ');
                last.text.push_str(&seg.text);
                continue;
            }
        }

        sections.push(SpeakerSection {
            speaker: speaker_label,
            start: seg.start,
            end: seg.end,
            text: seg.text.clone(),
        });
    }

    sections
}

// ---------------------------------------------------------------------------
// Document building
// ---------------------------------------------------------------------------

/// Build a [`Document`] from speaker sections.
fn build_document(title: Option<String>, sections: &[SpeakerSection]) -> Document {
    let mut elements: Vec<Element> = Vec::new();

    // Optional title heading.
    if let Some(ref t) = title {
        elements.push(Element::Heading {
            level: 1,
            text: t.clone(),
        });
    }

    for section in sections {
        let heading_text = format!(
            "[{} - {}] {}",
            format_timestamp(section.start),
            format_timestamp(section.end),
            section.speaker
        );

        elements.push(Element::Heading {
            level: 2,
            text: heading_text,
        });

        elements.push(Element::Paragraph {
            text: RichText {
                segments: vec![TextSegment {
                    text: section.text.clone(),
                    bold: false,
                    italic: false,
                    code: false,
                    link: None,
                }],
            },
        });
    }

    Document {
        metadata: Metadata {
            title,
            author: None,
            date: None,
        },
        pages: vec![Page { elements }],
    }
}

// ---------------------------------------------------------------------------
// Timestamp formatting
// ---------------------------------------------------------------------------

/// Format a time in seconds as `MM:SS`.
fn format_timestamp(seconds: f64) -> String {
    let total_secs = seconds.max(0.0);
    let mins = (total_secs / 60.0) as u32;
    let secs = (total_secs % 60.0) as u32;
    format!("{:02}:{:02}", mins, secs)
}

// ---------------------------------------------------------------------------
// Live microphone capture
// ---------------------------------------------------------------------------

/// Capture audio from the default microphone until the user presses Enter.
///
/// Returns mono f32 PCM samples at 16 kHz.
fn capture_mic_until_enter() -> Result<Vec<f32>, ConvertError> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| {
        ConvertError::TranscriptionError("No default audio input device found".to_string())
    })?;

    tracing::debug!("Using input device: {:?}", device.name());

    let config = device.default_input_config().map_err(|e| {
        ConvertError::TranscriptionError(format!("Failed to get input config: {e}"))
    })?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;

    tracing::debug!(
        "Input config: {} Hz, {} channel(s), {:?}",
        sample_rate,
        channels,
        config.sample_format()
    );

    let buffer: Arc<std::sync::Mutex<Vec<f32>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let buffer_clone = Arc::clone(&buffer);

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_flag);

    // Build stream based on sample format.
    let err_fn = |err: cpal::StreamError| {
        tracing::error!("Audio stream error: {err}");
    };

    let stream_config: cpal::StreamConfig = config.clone().into();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if stop_clone.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut buf = buffer_clone.lock().unwrap_or_else(|e| e.into_inner());
                    // Mix to mono.
                    for frame in data.chunks(channels) {
                        let mono: f32 = frame.iter().sum::<f32>() / channels as f32;
                        buf.push(mono);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| {
                ConvertError::TranscriptionError(format!("Failed to build input stream: {e}"))
            })?,
        cpal::SampleFormat::I16 => {
            let buffer_clone2 = Arc::clone(&buffer);
            let stop_clone2 = Arc::clone(&stop_flag);
            device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if stop_clone2.load(Ordering::Relaxed) {
                            return;
                        }
                        let mut buf = buffer_clone2.lock().unwrap_or_else(|e| e.into_inner());
                        for frame in data.chunks(channels) {
                            let mono: f32 = frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>()
                                / channels as f32;
                            buf.push(mono);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| {
                    ConvertError::TranscriptionError(format!("Failed to build input stream: {e}"))
                })?
        }
        cpal::SampleFormat::U16 => {
            let buffer_clone2 = Arc::clone(&buffer);
            let stop_clone2 = Arc::clone(&stop_flag);
            device
                .build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        if stop_clone2.load(Ordering::Relaxed) {
                            return;
                        }
                        let mut buf = buffer_clone2.lock().unwrap_or_else(|e| e.into_inner());
                        for frame in data.chunks(channels) {
                            let mono: f32 = frame
                                .iter()
                                .map(|&s| (s as f32 - 32768.0) / 32768.0)
                                .sum::<f32>()
                                / channels as f32;
                            buf.push(mono);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| {
                    ConvertError::TranscriptionError(format!("Failed to build input stream: {e}"))
                })?
        }
        other => {
            return Err(ConvertError::TranscriptionError(format!(
                "Unsupported sample format: {other:?}"
            )));
        }
    };

    stream.play().map_err(|e| {
        ConvertError::TranscriptionError(format!("Failed to start audio stream: {e}"))
    })?;

    eprintln!("🎙  Recording... Press Enter to stop.");

    // Spawn a thread that prints elapsed time every second.
    let timer_flag = Arc::clone(&stop_flag);
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        loop {
            if timer_flag.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            if timer_flag.load(Ordering::Relaxed) {
                break;
            }
            let elapsed = start.elapsed().as_secs();
            let mins = elapsed / 60;
            let secs = elapsed % 60;
            eprint!("\r⏺  Recording: {:02}:{:02}  ", mins, secs);
        }
    });

    // Wait for Enter on a separate thread using raw byte reads for reliability.
    let enter_flag = Arc::clone(&stop_flag);
    let handle = std::thread::spawn(move || {
        use std::io::Read;
        let stdin = std::io::stdin();
        let mut buf = [0u8; 1];
        loop {
            match stdin.lock().read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if buf[0] == b'\n' || buf[0] == b'\r' {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        enter_flag.store(true, Ordering::Relaxed);
    });

    // H-11: Enforce maximum recording duration.
    let recording_start = std::time::Instant::now();
    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
        if recording_start.elapsed().as_secs() >= MAX_RECORDING_SECONDS {
            eprintln!(
                "Warning: Maximum recording duration ({MAX_RECORDING_SECONDS}s) reached. Stopping automatically."
            );
            stop_flag.store(true, Ordering::Relaxed);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // The Enter thread will exit quickly since stop_flag is set or it already set the flag.
    // Use a timeout to avoid blocking forever if stdin is stuck.
    let join_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < join_deadline {
        if handle.is_finished() {
            let _ = handle.join();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Stop recording.
    drop(stream);
    eprintln!("\r✅ Recording stopped.                ");

    let raw_samples = buffer.lock().unwrap_or_else(|e| e.into_inner()).clone();

    // Resample to 16 kHz if needed.
    let samples = if sample_rate != 16000 {
        tracing::debug!("Resampling mic audio from {} Hz to 16000 Hz", sample_rate);
        resample_linear(&raw_samples, sample_rate, 16000)
    } else {
        raw_samples
    };

    Ok(samples)
}

// ---------------------------------------------------------------------------
// Simple markdown printer (for live mode stdout)
// ---------------------------------------------------------------------------

/// Print a [`Document`] to stdout as markdown text.
fn print_document_as_markdown(doc: &Document) {
    for page in &doc.pages {
        for element in &page.elements {
            match element {
                Element::Heading { level, text } => {
                    let prefix = "#".repeat(*level as usize);
                    println!("{prefix} {text}");
                    println!();
                }
                Element::Paragraph { text } => {
                    let plain: String = text.segments.iter().map(|s| s.text.as_str()).collect();
                    println!("{plain}");
                    println!();
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp_zero() {
        assert_eq!(format_timestamp(0.0), "00:00");
    }

    #[test]
    fn test_format_timestamp_seconds_only() {
        assert_eq!(format_timestamp(5.0), "00:05");
        assert_eq!(format_timestamp(59.0), "00:59");
    }

    #[test]
    fn test_format_timestamp_minutes_and_seconds() {
        assert_eq!(format_timestamp(61.0), "01:01");
        assert_eq!(format_timestamp(125.5), "02:05");
        assert_eq!(format_timestamp(3600.0), "60:00");
    }

    #[test]
    fn test_format_timestamp_negative_clamped() {
        assert_eq!(format_timestamp(-10.0), "00:00");
    }

    #[test]
    fn test_format_timestamp_fractional() {
        // Fractional seconds are truncated.
        assert_eq!(format_timestamp(90.9), "01:30");
    }

    #[test]
    fn test_supported_extensions() {
        assert!(is_supported_extension("wav"));
        assert!(is_supported_extension("mp3"));
        assert!(is_supported_extension("m4a"));
        assert!(is_supported_extension("ogg"));
        assert!(is_supported_extension("webm"));
        assert!(is_supported_extension("flac"));
        assert!(!is_supported_extension("txt"));
        assert!(!is_supported_extension("pdf"));
        assert!(!is_supported_extension("WAV")); // case-sensitive; callers lowercase first
    }

    #[test]
    fn test_detect_speakers_empty() {
        let sections = detect_speakers(&[]);
        assert!(sections.is_empty());
    }

    #[test]
    fn test_detect_speakers_single_segment() {
        let segments = vec![TranscriptionSegment {
            start: 0.0,
            end: 5.0,
            text: "Hello world".to_string(),
        }];
        let sections = detect_speakers(&segments);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].speaker, "Speaker 1");
        assert_eq!(sections[0].text, "Hello world");
    }

    #[test]
    fn test_detect_speakers_no_pause_same_speaker() {
        let segments = vec![
            TranscriptionSegment {
                start: 0.0,
                end: 5.0,
                text: "Hello".to_string(),
            },
            TranscriptionSegment {
                start: 5.1,
                end: 10.0,
                text: "world".to_string(),
            },
        ];
        let sections = detect_speakers(&segments);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].speaker, "Speaker 1");
        assert!(sections[0].text.contains("Hello"));
        assert!(sections[0].text.contains("world"));
    }

    #[test]
    fn test_detect_speakers_pause_triggers_change() {
        let segments = vec![
            TranscriptionSegment {
                start: 0.0,
                end: 5.0,
                text: "Hello from speaker one".to_string(),
            },
            TranscriptionSegment {
                start: 7.5, // 2.5s gap > 2.0s threshold
                end: 12.0,
                text: "Hi from speaker two".to_string(),
            },
        ];
        let sections = detect_speakers(&segments);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].speaker, "Speaker 1");
        assert_eq!(sections[1].speaker, "Speaker 2");
    }

    #[test]
    fn test_detect_speakers_alternates_back() {
        let segments = vec![
            TranscriptionSegment {
                start: 0.0,
                end: 3.0,
                text: "A".to_string(),
            },
            TranscriptionSegment {
                start: 5.5, // gap = 2.5
                end: 8.0,
                text: "B".to_string(),
            },
            TranscriptionSegment {
                start: 10.5, // gap = 2.5
                end: 13.0,
                text: "C".to_string(),
            },
        ];
        let sections = detect_speakers(&segments);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].speaker, "Speaker 1");
        assert_eq!(sections[1].speaker, "Speaker 2");
        assert_eq!(sections[2].speaker, "Speaker 1");
    }

    #[test]
    fn test_file_not_found_error() {
        let path = Path::new("/nonexistent/audio.wav");
        let opts = AudioOptions::default();
        let result = AudioConverter::convert_file(path, &opts);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConvertError::FileNotFound(p) => assert_eq!(p, path),
            other => panic!("Expected FileNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_unsupported_format_error() {
        // Create a temp file with an unsupported extension.
        let dir = std::env::temp_dir();
        let path = dir.join("test_audio.xyz");
        std::fs::write(&path, b"fake").unwrap();

        let opts = AudioOptions::default();
        let result = AudioConverter::convert_file(&path, &opts);

        let _ = std::fs::remove_file(&path);

        assert!(result.is_err());
        match result.unwrap_err() {
            ConvertError::UnsupportedFormat(msg) => {
                assert!(msg.contains("xyz"), "Expected extension in message: {msg}");
            }
            other => panic!("Expected UnsupportedFormat, got: {other:?}"),
        }
    }

    #[test]
    fn test_model_path_explicit_not_found() {
        let fake_path = PathBuf::from("/tmp/nonexistent_model.bin");
        let result = ensure_model(&Some(fake_path.clone()));
        assert!(result.is_err());
        match result.unwrap_err() {
            ConvertError::FileNotFound(p) => assert_eq!(p, fake_path),
            other => panic!("Expected FileNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_model_path_explicit_exists() {
        let dir = std::env::temp_dir();
        let path = dir.join("test_model_exists.bin");
        std::fs::write(&path, b"fake model").unwrap();

        let result = ensure_model(&Some(path.clone()));
        let _ = std::fs::remove_file(&path);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), path);
    }

    #[test]
    fn test_build_document_structure() {
        let sections = vec![
            SpeakerSection {
                speaker: "Speaker 1".to_string(),
                start: 0.0,
                end: 30.0,
                text: "Hello there".to_string(),
            },
            SpeakerSection {
                speaker: "Speaker 2".to_string(),
                start: 32.0,
                end: 60.0,
                text: "Hi back".to_string(),
            },
        ];

        let doc = build_document(Some("test.wav".to_string()), &sections);

        assert_eq!(doc.metadata.title.as_deref(), Some("test.wav"));
        assert_eq!(doc.pages.len(), 1);

        let elems = &doc.pages[0].elements;
        // Title heading + 2 * (heading + paragraph) = 5 elements
        assert_eq!(elems.len(), 5);

        match &elems[0] {
            Element::Heading { level, text } => {
                assert_eq!(*level, 1);
                assert_eq!(text, "test.wav");
            }
            other => panic!("Expected Heading, got: {other:?}"),
        }

        match &elems[1] {
            Element::Heading { level, text } => {
                assert_eq!(*level, 2);
                assert!(text.contains("Speaker 1"));
                assert!(text.contains("00:00"));
                assert!(text.contains("00:30"));
            }
            other => panic!("Expected Heading, got: {other:?}"),
        }
    }

    #[test]
    fn test_resample_linear_same_rate() {
        let input = vec![1.0, 2.0, 3.0];
        let output = resample_linear(&input, 16000, 16000);
        assert_eq!(output, input);
    }

    #[test]
    fn test_resample_linear_empty() {
        let output = resample_linear(&[], 44100, 16000);
        assert!(output.is_empty());
    }

    #[test]
    fn test_resample_linear_downsample() {
        // Simple check: downsampling from 32 kHz to 16 kHz should roughly halve the length.
        let input: Vec<f32> = (0..3200).map(|i| (i as f32).sin()).collect();
        let output = resample_linear(&input, 32000, 16000);
        // Should be approximately 1600 samples.
        assert!((output.len() as i64 - 1600).abs() <= 1);
    }

    #[test]
    fn test_convert_live_rejects_cloud_engine() {
        let opts = AudioOptions {
            engine: AudioEngine::Cloud,
            model_path: None,
        };
        let result = AudioConverter::convert_live(&opts);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConvertError::TranscriptionError(msg) => {
                assert!(msg.contains("Cloud engine is not supported"));
            }
            other => panic!("Expected TranscriptionError, got: {other:?}"),
        }
    }
}
