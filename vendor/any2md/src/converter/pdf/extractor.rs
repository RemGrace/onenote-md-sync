use crate::error::ConvertError;
use lopdf::{Document, Object, ObjectId};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;
use tracing::{debug, trace, warn};

#[derive(Debug, Clone)]
pub struct RawTextBlock {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub end_x: f64,
    pub font_size: f64,
    pub font_name: String,
    pub has_bold: bool,
    pub has_italic: bool,
}

#[derive(Debug, Clone)]
pub struct RawImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Minimum pixel area to keep an image (skip tiny decorations like 1x1 spacers)
const MIN_IMAGE_PIXELS: u32 = 16;

#[derive(Debug, Clone)]
pub enum RawElement {
    Text(RawTextBlock),
    Image(RawImage),
}

/// A token produced while scanning a ToUnicode CMap stream.
#[derive(Debug, Clone, PartialEq)]
enum CMapToken {
    /// A `<...>` hex group decoded into UTF-16 code units.
    Hex(Vec<u16>),
    /// `[` — start of a bfrange destination array.
    Open,
    /// `]` — end of a bfrange destination array.
    Close,
    /// Any bare identifier (e.g. `beginbfchar`, `endbfrange`).
    Word(String),
}

pub struct RawPage {
    pub elements: Vec<RawElement>,
}

/// PDF document metadata extracted from the Info dictionary.
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
}

/// Document-level metrics computed from actual page data.
/// Used to replace hardcoded magic numbers with dynamic thresholds.
#[derive(Debug, Clone)]
pub struct PageMetrics {
    /// Most common (mode) font size across all pages
    pub mode_font_size: f64,
    /// Median vertical spacing between consecutive text lines
    pub median_line_spacing: f64,
    /// Average character width estimated from text blocks
    pub avg_char_width: f64,
    /// Total horizontal range of text on the page (max_x - min_x)
    pub page_x_range: f64,
}

impl PageMetrics {
    /// Compute metrics from extracted raw pages.
    pub fn from_pages(pages: &[RawPage]) -> Self {
        let mut font_sizes: Vec<f64> = Vec::new();
        let mut char_widths: Vec<f64> = Vec::new();
        let mut all_ys: Vec<(usize, f64)> = Vec::new(); // (page_idx, y)
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;

        for (page_idx, page) in pages.iter().enumerate() {
            for el in &page.elements {
                if let RawElement::Text(b) = el {
                    font_sizes.push(b.font_size);
                    all_ys.push((page_idx, b.y));
                    if b.x < min_x {
                        min_x = b.x;
                    }
                    if b.end_x > max_x {
                        max_x = b.end_x;
                    }
                    // Estimate char width from block dimensions
                    let char_count = b.text.chars().count();
                    if char_count > 0 {
                        let width = b.end_x - b.x;
                        if width > 0.0 {
                            char_widths.push(width / char_count as f64);
                        }
                    }
                }
            }
        }

        // Mode font size (most common)
        let mode_font_size = Self::compute_mode_font_size(&font_sizes);

        // Median line spacing: sort Ys per page, compute consecutive gaps
        let median_line_spacing = Self::compute_median_line_spacing(&all_ys);

        // Average char width
        let avg_char_width = if char_widths.is_empty() {
            mode_font_size * 0.5
        } else {
            char_widths.iter().sum::<f64>() / char_widths.len() as f64
        };

        let page_x_range = if max_x > min_x {
            max_x - min_x
        } else {
            612.0 // Default US Letter width in points
        };

        debug!(
            mode_font_size,
            median_line_spacing, avg_char_width, page_x_range, "PageMetrics computed"
        );

        Self {
            mode_font_size,
            median_line_spacing,
            avg_char_width,
            page_x_range,
        }
    }

    fn compute_mode_font_size(sizes: &[f64]) -> f64 {
        if sizes.is_empty() {
            return 12.0;
        }
        let mut freq: HashMap<u64, usize> = HashMap::new();
        for &s in sizes {
            let key = (s * 100.0) as u64;
            *freq.entry(key).or_insert(0) += 1;
        }
        let mode_key = freq.into_iter().max_by_key(|&(_, count)| count).unwrap().0;
        mode_key as f64 / 100.0
    }

    fn compute_median_line_spacing(ys: &[(usize, f64)]) -> f64 {
        if ys.len() < 2 {
            return 14.0; // Sensible default
        }
        // Group by page, sort within page, compute gaps
        let mut page_ys: HashMap<usize, Vec<f64>> = HashMap::new();
        for &(page_idx, y) in ys {
            page_ys.entry(page_idx).or_default().push(y);
        }
        let mut gaps: Vec<f64> = Vec::new();
        for (_, mut page_y_vals) in page_ys {
            page_y_vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
            page_y_vals.dedup_by(|a, b| (*a - *b).abs() < 0.5);
            for w in page_y_vals.windows(2) {
                let gap = (w[1] - w[0]).abs();
                // Only count reasonable line spacings (not page breaks)
                if gap > 1.0 && gap < 100.0 {
                    gaps.push(gap);
                }
            }
        }
        if gaps.is_empty() {
            return 14.0;
        }
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        gaps[gaps.len() / 2]
    }

    // ── Derived thresholds ────────────────

    /// Y tolerance for grouping blocks into the same visual line.
    pub fn y_line_tolerance(&self) -> f64 {
        (self.mode_font_size * 0.25).max(1.5)
    }

    /// Minimum X-range for a Y-line to be considered "wide" (spanning multiple columns).
    pub fn min_wide_x_range(&self) -> f64 {
        (self.page_x_range * 0.33).max(100.0)
    }

    /// Maximum Y gap between consecutive table rows before breaking the region.
    pub fn table_max_y_gap(&self) -> f64 {
        (self.median_line_spacing * 6.0).max(30.0)
    }

    /// X snap tolerance for column edge detection.
    pub fn snap_tolerance(&self) -> f64 {
        self.avg_char_width.max(3.0)
    }

    /// Small Y-gap for extending table with continuation lines after last wide row.
    pub fn table_continuation_gap(&self) -> f64 {
        (self.median_line_spacing * 2.0).max(10.0)
    }

    /// X-distance threshold to assign a block to a column.
    pub fn column_assign_distance(&self) -> f64 {
        // Use a fraction of the average column spacing, fallback to reasonable default
        (self.avg_char_width * 8.0).max(20.0)
    }

    /// X tolerance for "same X" in paragraph/list continuation merging.
    pub fn same_x_tolerance(&self) -> f64 {
        self.avg_char_width.max(3.0)
    }

    /// Y gap threshold for paragraph/list continuation merging.
    pub fn line_height_threshold(&self) -> f64 {
        (self.median_line_spacing * 1.5).max(self.mode_font_size * 1.5)
    }

    /// X-distance for "close X" in list item continuation.
    pub fn list_close_x(&self) -> f64 {
        (self.avg_char_width * 5.0).max(15.0)
    }
}

// --- Thresholds and limits ---
/// Y tolerance for considering blocks on the same line (in PDF points).
const SAME_LINE_Y_TOLERANCE: f64 = 1.0;
/// Average character width as a fraction of font size (proportional font estimate).
const AVG_CHAR_WIDTH_RATIO: f64 = 0.5;
/// Phase 1 merge: max gap multiplier before treating as column boundary.
const PHASE1_MAX_GAP: f64 = 2.0;
/// Phase 2 merge: max gap multiplier for line assembly.
const PHASE2_MAX_GAP: f64 = 4.0;
/// Gap multiplier above which a space is inserted between merged blocks.
const WORD_GAP_THRESHOLD: f64 = 0.3;
/// Minimum negative gap (overlap) before blocks are treated as unrelated.
const MAX_OVERLAP_MULTIPLIER: f64 = 2.0;
/// Maximum characters for fix_end_x adjustment.
const FIX_END_X_MAX_CHARS: usize = 3;
/// Maximum stretch factor for fix_end_x (handles wide chars like W, M).
const FIX_END_X_STRETCH: f64 = 2.0;
/// Default line height multiplier for T* operator (fallback when TL not set).
const DEFAULT_LINE_HEIGHT: f64 = 1.2;
/// Maximum text elements per page to prevent unbounded memory growth.
const MAX_ELEMENTS_PER_PAGE: usize = 100_000;

pub struct PdfExtractor;

impl PdfExtractor {
    pub fn extract(path: &Path) -> Result<Vec<RawPage>, ConvertError> {
        if !path.exists() {
            return Err(ConvertError::FileNotFound(path.to_path_buf()));
        }

        let doc = Document::load(path)
            .map_err(|e| ConvertError::CorruptedFile(format!("Failed to parse PDF: {}", e)))?;

        let mut pages = Vec::new();
        let page_count = doc.get_pages().len().min(u32::MAX as usize);
        debug!(page_count, path = %path.display(), "PDF loaded");

        for page_num in 1..=page_count as u32 {
            debug!(page_num, page_count, "Extracting page");
            let raw_page = Self::extract_page(&doc, page_num)?;
            let text_count = raw_page
                .elements
                .iter()
                .filter(|e| matches!(e, RawElement::Text(_)))
                .count();
            let image_count = raw_page
                .elements
                .iter()
                .filter(|e| matches!(e, RawElement::Image(_)))
                .count();
            debug!(
                page_num,
                text_blocks = text_count,
                images = image_count,
                "Page extracted"
            );
            pages.push(raw_page);
        }

        Ok(pages)
    }

    /// Extract raw pages and metadata from a PDF in a single load.
    pub fn extract_with_metadata(path: &Path) -> Result<(Vec<RawPage>, PdfMetadata), ConvertError> {
        if !path.exists() {
            return Err(ConvertError::FileNotFound(path.to_path_buf()));
        }

        let doc = Document::load(path)
            .map_err(|e| ConvertError::CorruptedFile(format!("Failed to parse PDF: {}", e)))?;

        let mut pages = Vec::new();
        let page_count = doc.get_pages().len().min(u32::MAX as usize);
        debug!(page_count, path = %path.display(), "PDF loaded (with metadata)");

        for page_num in 1..=page_count as u32 {
            debug!(page_num, page_count, "Extracting page");
            let raw_page = Self::extract_page(&doc, page_num)?;
            let text_count = raw_page
                .elements
                .iter()
                .filter(|e| matches!(e, RawElement::Text(_)))
                .count();
            let image_count = raw_page
                .elements
                .iter()
                .filter(|e| matches!(e, RawElement::Image(_)))
                .count();
            debug!(
                page_num,
                text_blocks = text_count,
                images = image_count,
                "Page extracted"
            );
            pages.push(raw_page);
        }

        let metadata = Self::extract_metadata_from_doc(&doc);
        Ok((pages, metadata))
    }

    /// Extract metadata (title, author, date) from the PDF document info dictionary.
    pub fn extract_metadata(path: &Path) -> PdfMetadata {
        let doc = match Document::load(path) {
            Ok(d) => d,
            Err(_) => {
                return PdfMetadata {
                    title: None,
                    author: None,
                    date: None,
                }
            }
        };

        Self::extract_metadata_from_doc(&doc)
    }

    /// Extract metadata from an already-loaded PDF document.
    fn extract_metadata_from_doc(doc: &Document) -> PdfMetadata {
        let info_dict = Self::get_info_dict(doc);

        let title = info_dict
            .as_ref()
            .and_then(|d| Self::get_info_string(d, b"Title"));
        let author = info_dict
            .as_ref()
            .and_then(|d| Self::get_info_string(d, b"Author"));
        let date = info_dict
            .as_ref()
            .and_then(|d| Self::get_info_string(d, b"CreationDate"))
            .map(|s| Self::parse_pdf_date(&s));

        debug!(?title, ?author, ?date, "PDF metadata extracted");
        PdfMetadata {
            title,
            author,
            date,
        }
    }

    /// Get the Info dictionary from the document trailer.
    fn get_info_dict(doc: &Document) -> Option<lopdf::Dictionary> {
        let info_obj = doc.trailer.get(b"Info").ok()?;
        match info_obj {
            Object::Dictionary(dict) => Some(dict.clone()),
            Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| {
                if let Object::Dictionary(dict) = o {
                    Some(dict.clone())
                } else {
                    None
                }
            }),
            _ => None,
        }
    }

    /// Extract a string value from an info dictionary by key.
    fn get_info_string(dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
        let obj = dict.get(key).ok()?;
        match obj {
            Object::String(bytes, _) => {
                // Try UTF-16BE first (starts with BOM 0xFE 0xFF)
                if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                    let chars: Vec<u16> = bytes[2..]
                        .chunks(2)
                        .filter_map(|c| {
                            if c.len() == 2 {
                                Some(u16::from_be_bytes([c[0], c[1]]))
                            } else {
                                None
                            }
                        })
                        .collect();
                    String::from_utf16(&chars).ok()
                } else {
                    Some(String::from_utf8_lossy(bytes).into_owned())
                }
            }
            _ => None,
        }
    }

    /// Parse a PDF date string (D:YYYYMMDDHHmmSS) into a simpler format.
    fn parse_pdf_date(raw: &str) -> String {
        let s = raw.strip_prefix("D:").unwrap_or(raw);
        if s.len() >= 8 {
            let year = &s[0..4];
            let month = &s[4..6];
            let day = &s[6..8];
            format!("{}-{}-{}", year, month, day)
        } else {
            raw.to_string()
        }
    }

    fn extract_page(doc: &Document, page_num: u32) -> Result<RawPage, ConvertError> {
        let pages = doc.get_pages();
        let page_id = match pages.get(&page_num) {
            Some(id) => *id,
            None => return Ok(RawPage { elements: vec![] }),
        };

        // Try content-stream parsing first; fall back to extract_text on failure.
        let mut page = match Self::extract_page_from_streams(doc, page_id) {
            Ok(p) if !p.elements.is_empty() => {
                debug!(page_num, "Content stream parsing succeeded");
                p
            }
            Ok(_) => {
                debug!(
                    page_num,
                    "Content stream empty, falling back to extract_text"
                );
                Self::extract_page_fallback(doc, page_num)?
            }
            Err(e) => {
                warn!(page_num, error = %e, "Content stream parsing failed, falling back");
                Self::extract_page_fallback(doc, page_num)?
            }
        };

        // Fix end_x: use actual advance between consecutive same-line blocks
        Self::fix_end_x(&mut page.elements);

        // Merge adjacent text blocks on the same line
        page.elements = Self::merge_text_blocks(page.elements);

        // Extract images from page resources
        let images = Self::extract_page_images(doc, page_id);
        for img in images {
            page.elements.push(RawElement::Image(img));
        }

        Ok(page)
    }

    /// Fix end_x for short text blocks using the actual advance to the next
    /// same-line block. The estimated end_x (char_count * AVG_CHAR_WIDTH_RATIO * font_size)
    /// is inaccurate for proportional fonts. This pass sets end_x based on the
    /// actual gap between blocks, but caps it to avoid spanning column boundaries.
    fn fix_end_x(elements: &mut [RawElement]) {
        let len = elements.len();
        for i in 0..len.saturating_sub(1) {
            let (curr_y, curr_x, curr_end_x, curr_chars) = {
                if let RawElement::Text(ref b) = elements[i] {
                    (b.y, b.x, b.end_x, b.text.trim().chars().count())
                } else {
                    continue;
                }
            };
            if curr_chars > FIX_END_X_MAX_CHARS {
                continue;
            }
            let next_x = if let RawElement::Text(ref b) = elements[i + 1] {
                if (b.y - curr_y).abs() < SAME_LINE_Y_TOLERANCE {
                    b.x
                } else {
                    continue;
                }
            } else {
                continue;
            };
            let max_end = curr_x + (curr_end_x - curr_x) * FIX_END_X_STRETCH;
            let new_end_x = next_x.min(max_end);
            if new_end_x > curr_x {
                if let RawElement::Text(ref mut b) = elements[i] {
                    b.end_x = new_end_x;
                }
            }
        }
    }

    /// Phase 1 merge: merge adjacent text blocks on the same line within
    /// the same table cell. Uses PHASE1_MAX_GAP to preserve column boundaries.
    fn merge_text_blocks(elements: Vec<RawElement>) -> Vec<RawElement> {
        Self::merge_same_line_blocks(elements, PHASE1_MAX_GAP)
    }

    /// Phase 2 merge: assemble same-line blocks into full text lines.
    /// Runs AFTER table detection on non-table elements only.
    /// Uses PHASE2_MAX_GAP for a more permissive column boundary.
    pub fn assemble_lines(elements: Vec<RawElement>) -> Vec<RawElement> {
        Self::merge_same_line_blocks(elements, PHASE2_MAX_GAP)
    }

    /// Core merge: join adjacent same-line text blocks.
    /// `max_gap_multiplier` controls the column-boundary threshold
    /// (gap > avg_char_width * max_gap_multiplier → separate blocks).
    fn merge_same_line_blocks(
        elements: Vec<RawElement>,
        max_gap_multiplier: f64,
    ) -> Vec<RawElement> {
        let mut merged: Vec<RawElement> = Vec::new();

        for el in elements {
            match el {
                RawElement::Text(block) => {
                    if let Some(RawElement::Text(ref mut prev)) = merged.last_mut() {
                        let same_line = (prev.y - block.y).abs() < SAME_LINE_Y_TOLERANCE;
                        if same_line {
                            let gap = block.x - prev.end_x;
                            let avg_char_width = prev.font_size * AVG_CHAR_WIDTH_RATIO;

                            // Large positive gap → column boundary, keep separate
                            if gap > avg_char_width * max_gap_multiplier {
                                merged.push(RawElement::Text(block));
                                continue;
                            }

                            // Large negative gap (significant overlap) → likely
                            // unrelated content (watermark, annotation), keep separate
                            if gap < -(avg_char_width * MAX_OVERLAP_MULTIPLIER) {
                                merged.push(RawElement::Text(block));
                                continue;
                            }

                            // Insert space for word-level gaps
                            let needs_space = gap > avg_char_width * WORD_GAP_THRESHOLD;
                            if needs_space
                                && !block.text.starts_with(' ')
                                && !prev.text.ends_with(' ')
                            {
                                prev.text.push(' ');
                            }
                            prev.text.push_str(&block.text);
                            prev.end_x = block.end_x;
                            prev.has_bold = prev.has_bold || block.has_bold;
                            prev.has_italic = prev.has_italic || block.has_italic;
                            let same_font = prev.font_name == block.font_name
                                && (prev.font_size - block.font_size).abs() < 0.1;
                            if !same_font {
                                prev.font_name = block.font_name;
                                prev.font_size = block.font_size;
                            }
                            continue;
                        }
                    }
                    merged.push(RawElement::Text(block));
                }
                other => merged.push(other),
            }
        }

        // Trim and filter empty blocks
        for el in &mut merged {
            if let RawElement::Text(ref mut b) = el {
                let trimmed = b.text.trim().to_string();
                b.text = trimmed;
            }
        }
        merged
            .into_iter()
            .filter(|el| {
                if let RawElement::Text(ref b) = el {
                    !b.text.is_empty()
                } else {
                    true
                }
            })
            .collect()
    }

    /// Extract Image XObjects from the page resources.
    fn extract_page_images(doc: &Document, page_id: ObjectId) -> Vec<RawImage> {
        let mut images = Vec::new();

        // Get the page dictionary
        let page_obj = match doc.get_object(page_id) {
            Ok(obj) => obj,
            Err(_) => return images,
        };

        let page_dict = match page_obj.as_dict() {
            Ok(d) => d,
            Err(_) => return images,
        };

        // Navigate to Resources -> XObject
        let resources = match page_dict.get(b"Resources") {
            Ok(obj) => Self::resolve_object(doc, obj),
            Err(_) => return images,
        };

        let resources_dict = match resources.as_dict() {
            Ok(d) => d,
            Err(_) => return images,
        };

        let xobject = match resources_dict.get(b"XObject") {
            Ok(obj) => Self::resolve_object(doc, obj),
            Err(_) => return images,
        };

        let xobject_dict = match xobject.as_dict() {
            Ok(d) => d,
            Err(_) => return images,
        };

        // Iterate over XObject entries
        for (_name, obj_ref) in xobject_dict.iter() {
            let obj_id = match obj_ref {
                Object::Reference(id) => *id,
                _ => continue,
            };

            let obj = match doc.get_object(obj_id) {
                Ok(o) => o,
                Err(_) => continue,
            };

            if let Object::Stream(ref stream) = *obj {
                // Check if Subtype is Image
                let subtype = stream.dict.get(b"Subtype").ok().and_then(|s| match s {
                    Object::Name(n) => Some(n.clone()),
                    _ => None,
                });

                if subtype.as_deref() != Some(b"Image") {
                    continue;
                }

                let width = stream
                    .dict
                    .get(b"Width")
                    .ok()
                    .and_then(Self::obj_to_f64)
                    .unwrap_or(0.0) as u32;

                let height = stream
                    .dict
                    .get(b"Height")
                    .ok()
                    .and_then(Self::obj_to_f64)
                    .unwrap_or(0.0) as u32;

                if width == 0 || height == 0 {
                    continue;
                }

                // Skip tiny decoration images (spacers, dots, etc.)
                if width * height < MIN_IMAGE_PIXELS {
                    trace!(width, height, "Skipping tiny image");
                    continue;
                }

                let raw_data = stream.content.clone();
                if raw_data.is_empty() {
                    continue;
                }

                // Check for PNG predictor in DecodeParms
                let predictor = stream
                    .dict
                    .get(b"DecodeParms")
                    .ok()
                    .and_then(|p| p.as_dict().ok())
                    .and_then(|d| d.get(b"Predictor").ok())
                    .and_then(Self::obj_to_f64)
                    .unwrap_or(1.0) as u32;

                debug!(
                    width,
                    height,
                    raw_bytes = raw_data.len(),
                    predictor,
                    first_bytes = format!(
                        "{:02x} {:02x} {:02x} {:02x}",
                        raw_data.get(0).unwrap_or(&0),
                        raw_data.get(1).unwrap_or(&0),
                        raw_data.get(2).unwrap_or(&0),
                        raw_data.get(3).unwrap_or(&0)
                    ),
                    "Image stream raw data"
                );

                // Determine color space to know bytes per pixel
                let bpc = stream
                    .dict
                    .get(b"BitsPerComponent")
                    .ok()
                    .and_then(Self::obj_to_f64)
                    .unwrap_or(8.0) as u32;

                let color_space = Self::get_color_space_name(&stream.dict, doc);
                let channels = match color_space.as_str() {
                    "DeviceGray" | "CalGray" => 1u32,
                    "DeviceCMYK" | "CalCMYK" => 4,
                    _ => 3, // DeviceRGB, CalRGB, or unknown → assume RGB
                };

                // Check if the data is already a known image format (JPEG, PNG)
                let filter = Self::get_filter_name(&stream.dict);
                let png_data = if filter == "DCTDecode" {
                    // JPEG data — already a valid image, wrap as-is
                    // The renderer saves as .png but we'll store JPEG data
                    // and detect format at save time
                    Some(raw_data.clone())
                } else {
                    // Raw pixel data (possibly zlib/FlateDecode compressed)
                    Self::encode_raw_to_png(&raw_data, width, height, channels, bpc, &filter)
                };

                if let Some(data) = png_data {
                    images.push(RawImage {
                        data,
                        width,
                        height,
                    });
                } else {
                    debug!(
                        width,
                        height,
                        filter = filter.as_str(),
                        color_space = color_space.as_str(),
                        "Skipping image: could not decode"
                    );
                }
            }
        }

        images
    }

    /// Get the color space name from a stream dictionary.
    fn get_color_space_name(dict: &lopdf::Dictionary, doc: &Document) -> String {
        match dict.get(b"ColorSpace") {
            Ok(obj) => {
                let resolved = Self::resolve_object(doc, obj);
                match resolved {
                    Object::Name(n) => String::from_utf8_lossy(n).to_string(),
                    Object::Array(arr) if !arr.is_empty() => {
                        // e.g. [/ICCBased 10 0 R] — use the base name
                        match &arr[0] {
                            Object::Name(n) => String::from_utf8_lossy(n).to_string(),
                            _ => "DeviceRGB".to_string(),
                        }
                    }
                    _ => "DeviceRGB".to_string(),
                }
            }
            Err(_) => "DeviceRGB".to_string(),
        }
    }

    /// Get the filter name from a stream dictionary.
    fn get_filter_name(dict: &lopdf::Dictionary) -> String {
        match dict.get(b"Filter") {
            Ok(Object::Name(n)) => String::from_utf8_lossy(n).to_string(),
            Ok(Object::Array(arr)) if !arr.is_empty() => match &arr[0] {
                Object::Name(n) => String::from_utf8_lossy(n).to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    /// Encode raw pixel data (from PDF stream) into a valid PNG.
    fn encode_raw_to_png(
        raw_data: &[u8],
        width: u32,
        height: u32,
        channels: u32,
        bpc: u32,
        filter: &str,
    ) -> Option<Vec<u8>> {
        // Decompress if FlateDecode (zlib)
        let pixels = if filter == "FlateDecode" || filter.is_empty() {
            // Try zlib decompression first
            let mut decoded = Vec::new();
            if flate2::read::ZlibDecoder::new(raw_data)
                .read_to_end(&mut decoded)
                .is_ok()
                && !decoded.is_empty()
            {
                decoded
            } else {
                // Maybe deflate (not zlib) or already raw
                decoded.clear();
                if flate2::read::DeflateDecoder::new(raw_data)
                    .read_to_end(&mut decoded)
                    .is_ok()
                    && !decoded.is_empty()
                {
                    decoded
                } else {
                    raw_data.to_vec() // Treat as raw
                }
            }
        } else {
            return None; // Unsupported filter (JBIG2, JPXDecode, etc.)
        };

        let expected_len = (width * height * channels * bpc / 8) as usize;
        debug!(
            decompressed = pixels.len(),
            expected = expected_len,
            filter,
            width,
            height,
            channels,
            bpc,
            "Image decode stats"
        );
        // Allow some tolerance for padding
        if pixels.len() < expected_len.saturating_sub(width as usize) {
            debug!(
                actual = pixels.len(),
                expected = expected_len,
                "Pixel data length mismatch"
            );
            return None;
        }

        // Encode as PNG using the image crate
        let color_type = match channels {
            1 => image::ColorType::L8,
            3 => image::ColorType::Rgb8,
            4 => {
                // CMYK → convert to RGB
                let mut rgb = Vec::with_capacity((width * height * 3) as usize);
                for chunk in pixels.chunks(4) {
                    if chunk.len() < 4 {
                        break;
                    }
                    let (c, m, y, k) = (
                        chunk[0] as f32,
                        chunk[1] as f32,
                        chunk[2] as f32,
                        chunk[3] as f32,
                    );
                    let r = 255.0 * (1.0 - c / 255.0) * (1.0 - k / 255.0);
                    let g = 255.0 * (1.0 - m / 255.0) * (1.0 - k / 255.0);
                    let b = 255.0 * (1.0 - y / 255.0) * (1.0 - k / 255.0);
                    rgb.push(r as u8);
                    rgb.push(g as u8);
                    rgb.push(b as u8);
                }
                return Self::write_png_bytes(width, height, &rgb, image::ColorType::Rgb8);
            }
            _ => return None,
        };

        Self::write_png_bytes(width, height, &pixels, color_type)
    }

    /// Write pixel data as PNG to a byte vector.
    /// Uses the image crate to create an in-memory image and encode it,
    /// which validates the data and produces universally compatible PNGs.
    fn write_png_bytes(
        width: u32,
        height: u32,
        pixels: &[u8],
        color_type: image::ColorType,
    ) -> Option<Vec<u8>> {
        // Build an image buffer from raw pixels and save as PNG via the image crate.
        // This produces properly structured PNGs that all viewers can open.
        let img = match color_type {
            image::ColorType::L8 => {
                let buf = image::GrayImage::from_raw(width, height, pixels.to_vec())?;
                image::DynamicImage::ImageLuma8(buf)
            }
            image::ColorType::Rgb8 => {
                let buf = image::RgbImage::from_raw(width, height, pixels.to_vec())?;
                image::DynamicImage::ImageRgb8(buf)
            }
            image::ColorType::Rgba8 => {
                let buf = image::RgbaImage::from_raw(width, height, pixels.to_vec())?;
                image::DynamicImage::ImageRgba8(buf)
            }
            _ => return None,
        };

        let mut png_buf = Vec::new();
        let cursor = std::io::Cursor::new(&mut png_buf);
        match img.write_to(cursor, image::ImageFormat::Png) {
            Ok(()) => Some(png_buf),
            Err(e) => {
                debug!(error = %e, "Failed to encode PNG");
                None
            }
        }
    }

    /// Resolve an object reference, returning the referenced object or the object itself.
    fn resolve_object<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
        match obj {
            Object::Reference(id) => doc.get_object(*id).unwrap_or(obj),
            _ => obj,
        }
    }

    /// Parse PDF content streams to extract text with real font metadata.
    fn extract_page_from_streams(
        doc: &Document,
        page_id: ObjectId,
    ) -> Result<RawPage, ConvertError> {
        let content_data = doc
            .get_page_content(page_id)
            .map_err(|e| ConvertError::CorruptedFile(format!("Content stream error: {}", e)))?;

        let content = lopdf::content::Content::decode(&content_data)
            .map_err(|e| ConvertError::CorruptedFile(format!("Content decode error: {}", e)))?;

        // Build font name lookup: /F1 → "Helvetica-Bold", etc.
        let font_map = Self::build_font_map(doc, page_id);
        debug!(fonts = ?font_map, "Font map built");

        // Build ToUnicode CMap tables for text decoding
        let cmap_tables = Self::build_cmap_tables(doc, page_id);
        debug!(cmap_count = cmap_tables.len(), "CMap tables built");

        let mut elements = Vec::new();
        let mut current_font_tag = String::new();
        let mut current_font_size: f64 = 12.0;
        // Cached resolved font name + bold/italic flags (updated on Tf)
        let mut current_font_name = String::from("Unknown");
        let mut current_bold = false;
        let mut current_italic = false;
        let mut x: f64 = 0.0;
        let mut y: f64 = 0.0;
        // Track text matrix position separately for Tm
        let mut tm_x: f64 = 0.0;
        let mut tm_y: f64 = 0.0;
        // Text leading set by TL operator (used by T*, ', " operators)
        let mut text_leading: f64 = 0.0;
        // Track pending space: when a whitespace-only text op is encountered,
        // set this flag so the next non-space text gets a leading space
        let mut pending_space = false;

        for op in &content.operations {
            // Memory safety: cap elements per page
            if elements.len() >= MAX_ELEMENTS_PER_PAGE {
                warn!(
                    "Reached max element limit ({}), truncating page",
                    MAX_ELEMENTS_PER_PAGE
                );
                break;
            }

            match op.operator.as_str() {
                "BT" => {
                    tm_x = 0.0;
                    tm_y = 0.0;
                    x = 0.0;
                    y = 0.0;
                    // Don't reset pending_space — it may carry across BT/ET groups
                }
                "ET" => {}
                "TL" => {
                    // Set text leading for T*, ', " operators
                    if let Some(val) = op.operands.first().and_then(Self::obj_to_f64) {
                        text_leading = val;
                    }
                }
                "Tf" => {
                    if op.operands.len() >= 2 {
                        if let Ok(name_bytes) = op.operands[0].as_name() {
                            current_font_tag = String::from_utf8_lossy(name_bytes).into_owned();
                        }
                        current_font_size = Self::obj_to_f64(&op.operands[1]).unwrap_or(12.0);
                        // Cache resolved font name and style flags (M-3)
                        let resolved = font_map
                            .get(current_font_tag.as_str())
                            .cloned()
                            .unwrap_or_else(|| current_font_tag.clone());
                        current_font_name = if resolved.is_empty() {
                            "Unknown".to_string()
                        } else {
                            resolved
                        };
                        let font_lower = current_font_name.to_lowercase();
                        current_bold = font_lower.contains("bold");
                        current_italic =
                            font_lower.contains("italic") || font_lower.contains("oblique");
                        trace!(font_tag = %current_font_tag, font_size = current_font_size, "Font set");
                    }
                }
                "Td" => {
                    if op.operands.len() >= 2 {
                        let tx = Self::obj_to_f64(&op.operands[0]).unwrap_or(0.0);
                        let ty = Self::obj_to_f64(&op.operands[1]).unwrap_or(0.0);
                        x += tx;
                        y += ty;
                        tm_x = x;
                        tm_y = y;
                    }
                }
                "TD" => {
                    // TD is equivalent to: -ty TL, tx ty Td
                    if op.operands.len() >= 2 {
                        let tx = Self::obj_to_f64(&op.operands[0]).unwrap_or(0.0);
                        let ty = Self::obj_to_f64(&op.operands[1]).unwrap_or(0.0);
                        text_leading = -ty;
                        x += tx;
                        y += ty;
                        tm_x = x;
                        tm_y = y;
                    }
                }
                "Tm" => {
                    // Text matrix: [a b c d e f]
                    if op.operands.len() >= 6 {
                        let e = Self::obj_to_f64(&op.operands[4]).unwrap_or(0.0);
                        let f = Self::obj_to_f64(&op.operands[5]).unwrap_or(0.0);
                        x = e;
                        y = f;
                        tm_x = e;
                        tm_y = f;
                    }
                }
                "T*" => {
                    let lead = if text_leading != 0.0 {
                        text_leading
                    } else {
                        current_font_size * DEFAULT_LINE_HEIGHT
                    };
                    y -= lead;
                    tm_y = y;
                }
                "Tj" => {
                    let cmap = cmap_tables.get(current_font_tag.as_str());
                    if let Some(text) = Self::extract_tj_text_decoded(&op.operands, cmap) {
                        Self::emit_text_block(
                            text,
                            &current_font_name,
                            current_font_size,
                            current_bold,
                            current_italic,
                            tm_x,
                            tm_y,
                            &mut pending_space,
                            &mut elements,
                        );
                    }
                }
                "TJ" => {
                    let cmap = cmap_tables.get(current_font_tag.as_str());
                    if let Some(text) = Self::extract_tj_array_text_decoded(&op.operands, cmap) {
                        Self::emit_text_block(
                            text,
                            &current_font_name,
                            current_font_size,
                            current_bold,
                            current_italic,
                            tm_x,
                            tm_y,
                            &mut pending_space,
                            &mut elements,
                        );
                    }
                }
                "'" | "\"" => {
                    // ' = T* then Tj; " = set spacing then T* then Tj
                    let lead = if text_leading != 0.0 {
                        text_leading
                    } else {
                        current_font_size * DEFAULT_LINE_HEIGHT
                    };
                    y -= lead;
                    tm_y = y;
                    let cmap = cmap_tables.get(current_font_tag.as_str());
                    if let Some(Object::String(bytes, _)) = op.operands.last() {
                        let text = Self::decode_text_with_cmap(bytes, cmap);
                        Self::emit_text_block(
                            text,
                            &current_font_name,
                            current_font_size,
                            current_bold,
                            current_italic,
                            tm_x,
                            tm_y,
                            &mut pending_space,
                            &mut elements,
                        );
                    }
                }
                _ => {}
            }
        }

        Ok(RawPage { elements })
    }

    /// Process decoded text from a Tj/TJ/"'"/"\"" operator: handle pending_space,
    /// compute estimated width, and emit a RawTextBlock into the elements list.
    #[allow(clippy::too_many_arguments)]
    fn emit_text_block(
        text: String,
        font_name: &str,
        font_size: f64,
        is_bold: bool,
        is_italic: bool,
        x: f64,
        y: f64,
        pending_space: &mut bool,
        elements: &mut Vec<RawElement>,
    ) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            if text.contains(' ') {
                *pending_space = true;
            }
            return;
        }
        let final_text = if *pending_space {
            *pending_space = false;
            format!(" {}", trimmed)
        } else {
            trimmed.to_string()
        };
        let char_count = final_text.trim().chars().count() as f64;
        let estimated_width = char_count * font_size.abs() * AVG_CHAR_WIDTH_RATIO;
        elements.push(RawElement::Text(RawTextBlock {
            text: final_text,
            x,
            y,
            end_x: x + estimated_width,
            font_size: font_size.abs(),
            has_bold: is_bold,
            has_italic: is_italic,
            font_name: font_name.to_string(),
        }));
    }

    /// Build a mapping from font resource names (e.g. "F1") to their /BaseFont names.
    fn build_font_map(doc: &Document, page_id: ObjectId) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();

        let fonts = match doc.get_page_fonts(page_id) {
            Ok(f) => f,
            Err(_) => return map,
        };

        for (name_bytes, font_dict) in &fonts {
            let tag = String::from_utf8_lossy(name_bytes).into_owned();
            let base_font = font_dict
                .get(b"BaseFont")
                .ok()
                .and_then(|obj| match obj {
                    Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
                    Object::Reference(id) => doc
                        .get_object(*id)
                        .ok()
                        .and_then(|o| o.as_name().ok())
                        .map(|n| String::from_utf8_lossy(n).into_owned()),
                    _ => None,
                })
                .unwrap_or_else(|| tag.clone());

            map.insert(tag, base_font);
        }

        map
    }

    /// Build ToUnicode CMap lookup tables for all fonts on a page.
    /// Returns a map from font tag (e.g. "F9") to a CID→Unicode mapping.
    fn build_cmap_tables(
        doc: &Document,
        page_id: ObjectId,
    ) -> BTreeMap<String, HashMap<u16, String>> {
        let mut cmaps = BTreeMap::new();

        let page_obj = match doc.get_object(page_id) {
            Ok(o) => o,
            Err(_) => return cmaps,
        };
        let page_dict = match page_obj.as_dict() {
            Ok(d) => d,
            Err(_) => return cmaps,
        };
        let resources = match page_dict.get(b"Resources") {
            Ok(obj) => Self::resolve_object(doc, obj),
            Err(_) => return cmaps,
        };
        let res_dict = match resources.as_dict() {
            Ok(d) => d,
            Err(_) => return cmaps,
        };
        let font_obj = match res_dict.get(b"Font") {
            Ok(obj) => Self::resolve_object(doc, obj),
            Err(_) => return cmaps,
        };
        let font_dict = match font_obj.as_dict() {
            Ok(d) => d,
            Err(_) => return cmaps,
        };

        for (name, value) in font_dict.iter() {
            let tag = String::from_utf8_lossy(name).into_owned();
            let font = Self::resolve_object(doc, value);
            if let Ok(fd) = font.as_dict() {
                if let Ok(Object::Reference(id)) = fd.get(b"ToUnicode") {
                    if let Ok(Object::Stream(ref stream)) = doc.get_object(*id) {
                        // ToUnicode CMaps may be FlateDecode-compressed or stored raw
                        // (no /Filter). lopdf's decompressed_content() returns a DictKey
                        // error when no filter is present, so fall back to the raw bytes.
                        let data = stream
                            .decompressed_content()
                            .unwrap_or_else(|_| stream.content.clone());
                        let cmap_text = String::from_utf8_lossy(&data);
                        let table = Self::parse_cmap(&cmap_text);
                        if !table.is_empty() {
                            debug!(font = %tag, entries = table.len(), "CMap parsed");
                            cmaps.insert(tag, table);
                        }
                    }
                }
            }
        }

        cmaps
    }

    /// Parse a ToUnicode CMap stream into a CID → Unicode string mapping.
    ///
    /// Handles all of the destination forms permitted by the PDF spec:
    /// - `beginbfchar`: `<src> <dst>` pairs
    /// - `beginbfrange` simple form: `<lo> <hi> <dst>` (offset added to the last
    ///   UTF-16 code unit of `dst`)
    /// - `beginbfrange` array form: `<lo> <hi> [<d0> <d1> ...]` (one destination
    ///   per source code in the range)
    ///
    /// Destinations may span multiple UTF-16 code units (ligatures, surrogate
    /// pairs), which are decoded together. The scan is token-based rather than
    /// line-based, so entries split across lines parse correctly.
    fn parse_cmap(cmap: &str) -> HashMap<u16, String> {
        let mut map = HashMap::new();
        let toks = Self::tokenize_cmap(cmap);
        let mut i = 0;

        while i < toks.len() {
            match &toks[i] {
                CMapToken::Word(w) if w == "beginbfchar" => {
                    i += 1;
                    // Consume <src> <dst> pairs until endbfchar.
                    while i < toks.len() {
                        if matches!(&toks[i], CMapToken::Word(w) if w == "endbfchar") {
                            break;
                        }
                        if let (Some(src), Some(dst)) = (toks.get(i), toks.get(i + 1)) {
                            if let (CMapToken::Hex(s), CMapToken::Hex(d)) = (src, dst) {
                                if let Some(&code) = s.first() {
                                    map.insert(code, Self::units_to_string(d));
                                }
                                i += 2;
                                continue;
                            }
                        }
                        i += 1;
                    }
                }
                CMapToken::Word(w) if w == "beginbfrange" => {
                    i += 1;
                    // Each entry: <lo> <hi> followed by either <dst> or [ ... ].
                    while i < toks.len() {
                        if matches!(&toks[i], CMapToken::Word(w) if w == "endbfrange") {
                            break;
                        }
                        let lo = match toks.get(i) {
                            Some(CMapToken::Hex(u)) => u.first().copied(),
                            _ => {
                                i += 1;
                                continue;
                            }
                        };
                        let hi = match toks.get(i + 1) {
                            Some(CMapToken::Hex(u)) => u.first().copied(),
                            _ => {
                                i += 1;
                                continue;
                            }
                        };
                        let (Some(lo), Some(hi)) = (lo, hi) else {
                            i += 1;
                            continue;
                        };

                        match toks.get(i + 2) {
                            // Array form: one destination per code in [lo, hi].
                            Some(CMapToken::Open) => {
                                let mut j = i + 3;
                                let mut code = lo;
                                while let Some(tok) = toks.get(j) {
                                    match tok {
                                        CMapToken::Close => break,
                                        CMapToken::Hex(d) => {
                                            if code <= hi {
                                                map.insert(code, Self::units_to_string(d));
                                            }
                                            code = code.saturating_add(1);
                                        }
                                        _ => {}
                                    }
                                    j += 1;
                                }
                                i = j + 1;
                            }
                            // Simple form: increment the last UTF-16 unit of dst.
                            Some(CMapToken::Hex(dst)) => {
                                for offset in 0..=(hi.saturating_sub(lo)) {
                                    let mut units = dst.clone();
                                    if let Some(last) = units.last_mut() {
                                        *last = last.wrapping_add(offset);
                                    }
                                    map.insert(lo + offset, Self::units_to_string(&units));
                                }
                                i += 3;
                            }
                            _ => i += 1,
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }

        map
    }

    /// Tokenize a CMap stream into hex strings (`<...>`), array delimiters
    /// (`[`, `]`), and bare words. Header constructs like `<<...>>` dicts and
    /// `(literal)` strings are emitted as tokens too, but are harmless because
    /// the parser only consumes hex inside bf-char/bf-range blocks.
    fn tokenize_cmap(cmap: &str) -> Vec<CMapToken> {
        let mut toks = Vec::new();
        let mut chars = cmap.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '<' => {
                    let mut inner = String::new();
                    for ch in chars.by_ref() {
                        if ch == '>' {
                            break;
                        }
                        inner.push(ch);
                    }
                    toks.push(CMapToken::Hex(Self::hex_units(&inner)));
                }
                '[' => toks.push(CMapToken::Open),
                ']' => toks.push(CMapToken::Close),
                c if c.is_whitespace() => {}
                _ => {
                    let mut word = String::from(c);
                    while let Some(&ch) = chars.peek() {
                        if ch.is_whitespace() || matches!(ch, '<' | '>' | '[' | ']') {
                            break;
                        }
                        word.push(ch);
                        chars.next();
                    }
                    toks.push(CMapToken::Word(word));
                }
            }
        }
        toks
    }

    /// Parse the inner hex of a `<...>` token into a sequence of UTF-16 code
    /// units. 4 hex digits per unit; lengths that aren't a multiple of 4 (e.g.
    /// single-byte `<26>`) are parsed as one value. Non-hex content yields an
    /// empty vec.
    fn hex_units(inner: &str) -> Vec<u16> {
        let s: String = inner.chars().filter(|c| !c.is_whitespace()).collect();
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Vec::new();
        }
        if s.len() % 4 == 0 {
            s.as_bytes()
                .chunks(4)
                .filter_map(|ch| u16::from_str_radix(std::str::from_utf8(ch).ok()?, 16).ok())
                .collect()
        } else {
            u32::from_str_radix(&s, 16)
                .ok()
                .and_then(|v| u16::try_from(v).ok())
                .map(|v| vec![v])
                .unwrap_or_default()
        }
    }

    /// Decode a sequence of UTF-16 code units (handling surrogate pairs) into a
    /// String, substituting U+FFFD for invalid units.
    fn units_to_string(units: &[u16]) -> String {
        char::decode_utf16(units.iter().copied())
            .map(|r| r.unwrap_or('\u{FFFD}'))
            .collect()
    }

    /// Decode raw bytes from a Tj/TJ string using the CMap for the current font.
    /// For Identity-H encoded fonts, bytes are 2-byte CIDs.
    fn decode_text_with_cmap(raw_bytes: &[u8], cmap: Option<&HashMap<u16, String>>) -> String {
        match cmap {
            Some(table) if !table.is_empty() => {
                let mut result = String::new();
                // Identity-H: 2-byte CIDs
                let chunks = raw_bytes.chunks(2);
                for chunk in chunks {
                    let cid = if chunk.len() == 2 {
                        u16::from_be_bytes([chunk[0], chunk[1]])
                    } else {
                        chunk[0] as u16
                    };
                    if let Some(unicode) = table.get(&cid) {
                        result.push_str(unicode);
                    } else {
                        // Fallback: try as single byte if CID not found
                        if cid < 128 {
                            if let Some(ch) = char::from_u32(cid as u32) {
                                result.push(ch);
                            }
                        }
                    }
                }
                result
            }
            _ => {
                // No CMap — use plain UTF-8 (works for standard encoding)
                String::from_utf8_lossy(raw_bytes).into_owned()
            }
        }
    }

    /// Convert an lopdf Object (Integer or Real) to f64.
    fn obj_to_f64(obj: &Object) -> Option<f64> {
        match obj {
            Object::Real(f) => Some(*f as f64),
            Object::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Extract text from a Tj operand list (no CMap, for tests/backward compat).
    #[cfg(test)]
    fn extract_tj_text(operands: &[Object]) -> Option<String> {
        for operand in operands {
            if let Object::String(bytes, _) = operand {
                return Some(String::from_utf8_lossy(bytes).into_owned());
            }
        }
        None
    }

    /// Extract text from a TJ array operand (no CMap, for tests/backward compat).
    #[cfg(test)]
    fn extract_tj_array_text(operands: &[Object]) -> Option<String> {
        for operand in operands {
            if let Object::Array(items) = operand {
                let mut result = String::new();
                for item in items {
                    match item {
                        Object::String(bytes, _) => {
                            result.push_str(&String::from_utf8_lossy(bytes));
                        }
                        Object::Integer(i) if *i < -100 => {
                            result.push(' ');
                        }
                        _ => {}
                    }
                }
                return Some(result);
            }
        }
        None
    }

    /// Extract and decode text from Tj operand using CMap.
    fn extract_tj_text_decoded(
        operands: &[Object],
        cmap: Option<&HashMap<u16, String>>,
    ) -> Option<String> {
        for operand in operands {
            if let Object::String(bytes, _) = operand {
                return Some(Self::decode_text_with_cmap(bytes, cmap));
            }
        }
        None
    }

    /// Extract and decode text from TJ array operand using CMap.
    fn extract_tj_array_text_decoded(
        operands: &[Object],
        cmap: Option<&HashMap<u16, String>>,
    ) -> Option<String> {
        for operand in operands {
            if let Object::Array(items) = operand {
                let mut result = String::new();
                for item in items {
                    match item {
                        Object::String(bytes, _) => {
                            result.push_str(&Self::decode_text_with_cmap(bytes, cmap));
                        }
                        Object::Integer(i) if *i < -100 => {
                            result.push(' ');
                        }
                        _ => {}
                    }
                }
                return Some(result);
            }
        }
        None
    }

    /// Fallback: use doc.extract_text() when content stream parsing fails.
    fn extract_page_fallback(doc: &Document, page_num: u32) -> Result<RawPage, ConvertError> {
        tracing::warn!(
            page = page_num,
            "Using fallback text extraction — table detection may be degraded"
        );
        let mut elements = Vec::new();

        if let Ok(content) = doc.extract_text(&[page_num]) {
            let lines: Vec<&str> = content.lines().collect();
            let mut y_pos = 800.0;
            for line in lines {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let estimated_width = trimmed.chars().count() as f64 * 12.0 * 0.5;
                    elements.push(RawElement::Text(RawTextBlock {
                        text: trimmed.to_string(),
                        x: 72.0,
                        y: y_pos,
                        end_x: 72.0 + estimated_width,
                        font_size: 12.0,
                        font_name: "Unknown".to_string(),
                        has_bold: false,
                        has_italic: false,
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
            end_x: 72.0 + 5.0 * 12.0 * 0.5,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
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
    fn test_parse_cmap_bfchar() {
        let cmap = "\
beginbfchar
<0003> <0041>
<0004> <0042>
endbfchar";
        let map = PdfExtractor::parse_cmap(cmap);
        assert_eq!(map.get(&0x0003).map(String::as_str), Some("A"));
        assert_eq!(map.get(&0x0004).map(String::as_str), Some("B"));
    }

    #[test]
    fn test_parse_cmap_bfrange_simple() {
        // <lo> <hi> <dst>: offset added to the last UTF-16 unit of dst.
        let cmap = "\
beginbfrange
<0010> <0012> <0061>
endbfrange";
        let map = PdfExtractor::parse_cmap(cmap);
        assert_eq!(map.get(&0x0010).map(String::as_str), Some("a"));
        assert_eq!(map.get(&0x0011).map(String::as_str), Some("b"));
        assert_eq!(map.get(&0x0012).map(String::as_str), Some("c"));
    }

    #[test]
    fn test_parse_cmap_bfrange_array() {
        // The array destination form used by LibreOffice/TurboScribe exports:
        // each source code in [lo, hi] maps to the matching array element.
        let cmap = "\
2 beginbfrange
<0000> <0000> <0000>
<0001> <0003> [<004F> <006E> <0020> ]
endbfrange";
        let map = PdfExtractor::parse_cmap(cmap);
        assert_eq!(map.get(&0x0001).map(String::as_str), Some("O"));
        assert_eq!(map.get(&0x0002).map(String::as_str), Some("n"));
        assert_eq!(map.get(&0x0003).map(String::as_str), Some(" "));
    }

    #[test]
    fn test_parse_cmap_multi_unit_destination() {
        // Ligature destination spanning two UTF-16 code units ("fi").
        let cmap = "\
beginbfchar
<0005> <00660069>
endbfchar";
        let map = PdfExtractor::parse_cmap(cmap);
        assert_eq!(map.get(&0x0005).map(String::as_str), Some("fi"));
    }

    #[test]
    fn test_parse_cmap_ignores_header() {
        // Header dicts (<<...>>) and literals must not leak into the table; the
        // codespacerange hex must not be mistaken for bf-char/range entries.
        let cmap = "\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
beginbfchar
<0001> <0058>
endbfchar";
        let map = PdfExtractor::parse_cmap(cmap);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&0x0001).map(String::as_str), Some("X"));
    }

    #[test]
    fn test_extract_nonexistent_file() {
        let result = PdfExtractor::extract(Path::new("nonexistent.pdf"));
        assert!(result.is_err());
    }

    #[test]
    fn test_obj_to_f64_real() {
        let obj = Object::Real(12.5);
        assert_eq!(PdfExtractor::obj_to_f64(&obj), Some(12.5));
    }

    #[test]
    fn test_obj_to_f64_integer() {
        let obj = Object::Integer(14);
        assert_eq!(PdfExtractor::obj_to_f64(&obj), Some(14.0));
    }

    #[test]
    fn test_obj_to_f64_invalid() {
        let obj = Object::Boolean(true);
        assert_eq!(PdfExtractor::obj_to_f64(&obj), None);
    }

    #[test]
    fn test_extract_tj_text() {
        let operands = vec![Object::String(
            b"Hello World".to_vec(),
            lopdf::StringFormat::Literal,
        )];
        let text = PdfExtractor::extract_tj_text(&operands);
        assert_eq!(text, Some("Hello World".to_string()));
    }

    #[test]
    fn test_extract_tj_text_empty() {
        let operands: Vec<Object> = vec![Object::Integer(42)];
        let text = PdfExtractor::extract_tj_text(&operands);
        assert_eq!(text, None);
    }

    #[test]
    fn test_extract_tj_array_text() {
        let operands = vec![Object::Array(vec![
            Object::String(b"Hel".to_vec(), lopdf::StringFormat::Literal),
            Object::Integer(-50),
            Object::String(b"lo".to_vec(), lopdf::StringFormat::Literal),
        ])];
        let text = PdfExtractor::extract_tj_array_text(&operands);
        assert_eq!(text, Some("Hello".to_string()));
    }

    #[test]
    fn test_extract_tj_array_with_large_kerning_inserts_space() {
        let operands = vec![Object::Array(vec![
            Object::String(b"Word1".to_vec(), lopdf::StringFormat::Literal),
            Object::Integer(-200),
            Object::String(b"Word2".to_vec(), lopdf::StringFormat::Literal),
        ])];
        let text = PdfExtractor::extract_tj_array_text(&operands);
        assert_eq!(text, Some("Word1 Word2".to_string()));
    }

    #[test]
    fn test_parse_pdf_date() {
        assert_eq!(
            PdfExtractor::parse_pdf_date("D:20240115120000"),
            "2024-01-15"
        );
    }

    #[test]
    fn test_parse_pdf_date_no_prefix() {
        assert_eq!(PdfExtractor::parse_pdf_date("20240115"), "2024-01-15");
    }

    #[test]
    fn test_parse_pdf_date_short() {
        assert_eq!(PdfExtractor::parse_pdf_date("D:2024"), "D:2024");
    }

    #[test]
    fn test_build_font_map_empty_doc() {
        // Just verifies it doesn't panic on an empty document
        let doc = Document::with_version("1.5");
        let map = PdfExtractor::build_font_map(&doc, (1, 0));
        assert!(map.is_empty());
    }

    #[test]
    fn test_metadata_nonexistent_file() {
        let meta = PdfExtractor::extract_metadata(Path::new("nonexistent.pdf"));
        assert!(meta.title.is_none());
        assert!(meta.author.is_none());
        assert!(meta.date.is_none());
    }

    #[test]
    fn test_merge_blocks_within_cell() {
        // Phase 1 merge: gap=10 < 2.0 * avg_char_width(6) = 12 → merge with space
        let elements = vec![
            RawElement::Text(RawTextBlock {
                text: "Hello".to_string(),
                x: 50.0,
                y: 100.0,
                end_x: 110.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
            RawElement::Text(RawTextBlock {
                text: "World".to_string(),
                x: 120.0,
                y: 100.0,
                end_x: 150.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
        ];
        let merged = PdfExtractor::merge_text_blocks(elements);
        assert_eq!(merged.len(), 1);
        if let RawElement::Text(b) = &merged[0] {
            assert_eq!(b.text, "Hello World");
        } else {
            panic!("Expected Text element");
        }
    }

    #[test]
    fn test_merge_blocks_column_gap_stays_separate() {
        // Phase 1 merge: gap=290 >> 2.0 * avg_char_width(6) = 12 → column boundary, NOT merged
        let elements = vec![
            RawElement::Text(RawTextBlock {
                text: "Col1".to_string(),
                x: 50.0,
                y: 100.0,
                end_x: 80.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
            RawElement::Text(RawTextBlock {
                text: "Col2".to_string(),
                x: 400.0,
                y: 100.0,
                end_x: 430.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
        ];
        let merged = PdfExtractor::merge_text_blocks(elements);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_assemble_lines_gap_based_space() {
        // Phase 2: moderate gap → merge with space
        let elements = vec![
            RawElement::Text(RawTextBlock {
                text: "Hello".to_string(),
                x: 50.0,
                y: 100.0,
                end_x: 110.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
            RawElement::Text(RawTextBlock {
                text: "World".to_string(),
                x: 120.0,
                y: 100.0,
                end_x: 150.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
        ];
        let merged = PdfExtractor::assemble_lines(elements);
        assert_eq!(merged.len(), 1);
        if let RawElement::Text(b) = &merged[0] {
            assert_eq!(b.text, "Hello World");
        } else {
            panic!("Expected Text element");
        }
    }

    #[test]
    fn test_assemble_lines_large_gap_separates() {
        // Phase 2: large gap → separate blocks
        let elements = vec![
            RawElement::Text(RawTextBlock {
                text: "Col1".to_string(),
                x: 50.0,
                y: 100.0,
                end_x: 80.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
            RawElement::Text(RawTextBlock {
                text: "Col2".to_string(),
                x: 400.0,
                y: 100.0,
                end_x: 430.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                has_bold: false,
                has_italic: false,
            }),
        ];
        let merged = PdfExtractor::assemble_lines(elements);
        assert_eq!(merged.len(), 2);
    }
}
