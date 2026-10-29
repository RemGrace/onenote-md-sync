use crate::converter::pdf::classifier::{BlockType, ClassifiedElement};
use crate::converter::pdf::extractor::{PageMetrics, RawTextBlock};
use crate::model::document::*;

pub struct Assembler;

// ── Header/footer noise patterns ────────────────
/// Matches standalone page indicators like "1/3", "12/30"
fn is_page_number(text: &str) -> bool {
    let t = text.trim();
    if let Some(pos) = t.find('/') {
        let left = &t[..pos];
        let right = &t[pos + 1..];
        left.chars().all(|c| c.is_ascii_digit())
            && right.chars().all(|c| c.is_ascii_digit())
            && !left.is_empty()
            && !right.is_empty()
    } else {
        false
    }
}

/// Matches timestamps like "12/03/2026, 12:41"
fn is_timestamp_line(text: &str) -> bool {
    let t = text.trim();
    // Pattern: dd/mm/yyyy, HH:MM or similar
    if t.len() >= 10 && t.len() <= 25 {
        let has_date_slash = t.chars().filter(|&c| c == '/').count() == 2;
        let has_colon = t.contains(':');
        let has_comma = t.contains(',');
        has_date_slash && has_colon && has_comma
    } else {
        false
    }
}

/// Check if a line is likely header/footer noise
fn is_header_footer_noise(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    // Standalone "OneNote"
    if t == "OneNote" {
        return true;
    }
    // Page numbers like "1/3"
    if is_page_number(t) {
        return true;
    }
    // Timestamps
    if is_timestamp_line(t) {
        return true;
    }
    // Very long SharePoint/OneNote URLs that are just navigation artifacts
    if t.starts_with("https://") && t.len() > 150 && t.contains("sharepoint.com") {
        return true;
    }
    false
}

/// Check if text ends in a way that suggests the sentence continues
fn text_continues(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    let last = t.chars().last().unwrap();
    // Ends with hyphen = word break
    if last == '-' && !t.ends_with(" -") && !t.ends_with(" —") {
        return true;
    }
    // Doesn't end with sentence-ending punctuation = likely continues
    !matches!(
        last,
        '.' | '!' | '?' | ':' | ';' | ')' | ']' | '}' | '"' | '»'
    )
}

/// Check if text starts with lowercase (likely continuation of previous sentence)
fn starts_lowercase(text: &str) -> bool {
    let t = text.trim();
    t.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
}

impl Assembler {
    pub fn assemble(
        classified_pages: Vec<Vec<ClassifiedElement>>,
        metadata: Metadata,
        metrics: &PageMetrics,
    ) -> Document {
        let pages: Vec<Page> = classified_pages
            .into_iter()
            .map(|elems| Self::assemble_page(Self::filter_noise(elems), metrics))
            .collect();

        let pages = Self::merge_cross_page_tables(pages);

        Document { metadata, pages }
    }

    /// Remove header/footer noise elements
    fn filter_noise(elems: Vec<ClassifiedElement>) -> Vec<ClassifiedElement> {
        elems
            .into_iter()
            .filter(|el| match el {
                ClassifiedElement::Text(block, _) => !is_header_footer_noise(&block.text),
                _ => true,
            })
            .collect()
    }

    /// Merge tables across page boundaries when consecutive pages end/start
    /// with tables that have the same column count.
    fn merge_cross_page_tables(pages: Vec<Page>) -> Vec<Page> {
        if pages.len() < 2 {
            return pages;
        }

        let mut result: Vec<Page> = Vec::with_capacity(pages.len());

        for page in pages {
            let should_merge = if let Some(prev_page) = result.last() {
                matches!(
                    (prev_page.elements.last(), page.elements.first()),
                    (
                        Some(Element::Table { headers: h1, .. }),
                        Some(Element::Table { headers: h2, .. }),
                    ) if h1.len() == h2.len()
                )
            } else {
                false
            };

            if should_merge {
                let mut page_elements = page.elements;
                // Remove the continuation table from the start of this page
                let next_table = page_elements.remove(0);
                if let Element::Table {
                    headers: next_headers,
                    rows: mut next_rows,
                } = next_table
                {
                    // Merge into the previous page's last table
                    if let Some(prev_page) = result.last_mut() {
                        if let Some(Element::Table { rows, .. }) = prev_page.elements.last_mut() {
                            // The "header" of the continuation is really a data row
                            rows.push(next_headers);
                            rows.append(&mut next_rows);
                        }
                    }
                }
                // Add remaining elements from this page (if any) to a new page
                if !page_elements.is_empty() {
                    result.push(Page {
                        elements: page_elements,
                    });
                }
            } else {
                result.push(page);
            }
        }

        result
    }

    fn assemble_page(elems: Vec<ClassifiedElement>, metrics: &PageMetrics) -> Page {
        let mut elements = Vec::new();
        let mut i = 0;

        while i < elems.len() {
            match &elems[i] {
                ClassifiedElement::Image(img) => {
                    elements.push(Element::Image {
                        data: img.data.clone(),
                        alt: None,
                    });
                    i += 1;
                }
                ClassifiedElement::PreBuilt(el) => {
                    elements.push(el.clone());
                    i += 1;
                }
                ClassifiedElement::Text(block, block_type) => match block_type {
                    BlockType::Heading(level) => {
                        let mut heading_text = block.text.clone();
                        let heading_level = *level;
                        let mut last_y = block.y;
                        i += 1;
                        // Merge consecutive headings at the same level (wrapped text)
                        // Use the heading's own font size for the gap, since headings are larger
                        let heading_merge_gap =
                            (block.font_size * 2.0).max(metrics.line_height_threshold());
                        while i < elems.len() {
                            if let ClassifiedElement::Text(
                                next_block,
                                BlockType::Heading(next_level),
                            ) = &elems[i]
                            {
                                if *next_level == heading_level {
                                    let y_gap = (next_block.y - last_y).abs();
                                    if y_gap < heading_merge_gap {
                                        heading_text.push(' ');
                                        heading_text.push_str(&next_block.text);
                                        last_y = next_block.y;
                                        i += 1;
                                        continue;
                                    }
                                }
                            }
                            break;
                        }
                        elements.push(Element::Heading {
                            level: heading_level,
                            text: heading_text,
                        });
                    }
                    BlockType::CodeBlock => {
                        let mut code_lines = vec![block.text.clone()];
                        i += 1;
                        while i < elems.len() {
                            if let ClassifiedElement::Text(b, BlockType::CodeBlock) = &elems[i] {
                                code_lines.push(b.text.clone());
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
                        let start = i;
                        let mut items = Vec::new();
                        while i < elems.len() {
                            if let ClassifiedElement::Text(b, BlockType::ListItem) = &elems[i] {
                                let text = Self::strip_list_marker(&b.text);
                                let mut item_y = b.y;
                                let item_x = b.x;
                                let mut item_text = text.clone();
                                let item_block = b;
                                i += 1;

                                // Merge continuation paragraphs into this list item
                                let list_close_x = metrics.list_close_x();
                                let list_line_height = metrics.line_height_threshold();
                                while i < elems.len() {
                                    if let ClassifiedElement::Text(nb, BlockType::Paragraph) =
                                        &elems[i]
                                    {
                                        let y_gap = (nb.y - item_y).abs();
                                        let close_x = (nb.x - item_x).abs() < list_close_x;
                                        let line_height = list_line_height;
                                        if close_x && y_gap < line_height {
                                            Self::append_continuation(&mut item_text, &nb.text);
                                            item_y = nb.y;
                                            i += 1;
                                            continue;
                                        }
                                    }
                                    break;
                                }

                                items.push(ListItem {
                                    text: Self::rich_text_from_block(&item_text, item_block),
                                    children: vec![],
                                });
                            } else {
                                break;
                            }
                        }
                        let first_text = if let ClassifiedElement::Text(b, _) = &elems[start] {
                            b.text.as_str()
                        } else {
                            ""
                        };
                        let ordered = Self::detect_ordered(first_text);
                        elements.push(Element::List { ordered, items });
                    }
                    BlockType::Paragraph => {
                        let mut para_text = block.text.clone();
                        let mut current_y = block.y;
                        let mut current_bold = block.has_bold;
                        let mut current_italic = block.has_italic;
                        i += 1;

                        // Merge continuation paragraphs: same X, close Y, text flows
                        let same_x_tol = metrics.same_x_tolerance();
                        let para_line_height = metrics.line_height_threshold();
                        while i < elems.len() {
                            if let ClassifiedElement::Text(next_block, BlockType::Paragraph) =
                                &elems[i]
                            {
                                let y_gap = (next_block.y - current_y).abs();
                                let same_x = (next_block.x - block.x).abs() < same_x_tol;
                                let line_height = para_line_height;

                                if same_x && y_gap < line_height {
                                    // Always merge if: URL continuation, text continues,
                                    // or next line starts lowercase
                                    let is_url_cont =
                                        para_text.contains("://") && para_text.ends_with('-');
                                    let flowing = text_continues(&para_text)
                                        || starts_lowercase(&next_block.text);
                                    let same_style = block.has_bold == next_block.has_bold
                                        && block.has_italic == next_block.has_italic;

                                    if is_url_cont || flowing || same_style {
                                        Self::append_continuation(&mut para_text, &next_block.text);
                                        current_y = next_block.y;
                                        current_bold = current_bold || next_block.has_bold;
                                        current_italic = current_italic || next_block.has_italic;
                                        i += 1;
                                        continue;
                                    }
                                }
                            }
                            break;
                        }

                        let mut result_block = block.clone();
                        result_block.has_bold = current_bold;
                        result_block.has_italic = current_italic;
                        elements.push(Element::Paragraph {
                            text: Self::rich_text_from_block(&para_text, &result_block),
                        });
                    }
                },
            }
        }

        Page { elements }
    }

    /// Append continuation text, handling hyphenated word breaks
    fn append_continuation(existing: &mut String, next: &str) {
        let trimmed = existing.trim_end();
        let is_url = trimmed.contains("://");
        if trimmed.ends_with('-') && !trimmed.ends_with(" -") && !trimmed.ends_with("--") && !is_url
        {
            // Hyphenated word break: remove hyphen and join directly
            existing.truncate(existing.trim_end().len() - 1);
            existing.push_str(next);
        } else if is_url && trimmed.ends_with('-') {
            // URL continuation: keep hyphen, no space
            existing.push_str(next);
        } else {
            existing.push(' ');
            existing.push_str(next);
        }
    }

    fn strip_list_marker(text: &str) -> String {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("• ") {
            return rest.to_string();
        }
        if text.starts_with("- ") || text.starts_with("* ") {
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
        trimmed
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    }

    fn rich_text_from_block(text: &str, block: &RawTextBlock) -> RichText {
        RichText {
            segments: vec![TextSegment {
                text: text.to_string(),
                bold: block.has_bold,
                italic: block.has_italic,
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
        let end_x = 72.0 + text.chars().count() as f64 * 12.0 * 0.5;
        RawTextBlock {
            text: text.to_string(),
            x: 72.0,
            y: 700.0,
            end_x,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        }
    }

    fn make_block_with_font(text: &str, font_name: &str) -> RawTextBlock {
        let end_x = 72.0 + text.chars().count() as f64 * 12.0 * 0.5;
        let font_lower = font_name.to_lowercase();
        RawTextBlock {
            text: text.to_string(),
            x: 72.0,
            y: 700.0,
            end_x,
            font_size: 12.0,
            font_name: font_name.to_string(),
            has_bold: font_lower.contains("bold"),
            has_italic: font_lower.contains("italic") || font_lower.contains("oblique"),
        }
    }

    fn empty_metadata() -> Metadata {
        Metadata {
            title: None,
            author: None,
            date: None,
        }
    }

    fn test_metrics() -> PageMetrics {
        PageMetrics {
            mode_font_size: 12.0,
            median_line_spacing: 14.0,
            avg_char_width: 6.0,
            page_x_range: 468.0, // Standard US Letter text area
        }
    }

    fn ce(block: RawTextBlock, bt: BlockType) -> ClassifiedElement {
        ClassifiedElement::Text(block, bt)
    }

    #[test]
    fn test_assemble_headings() {
        let blocks = vec![
            ce(make_block("Title"), BlockType::Heading(1)),
            ce(make_block("Subtitle"), BlockType::Heading(2)),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages.len(), 1);
        assert!(
            matches!(&doc.pages[0].elements[0], Element::Heading { level: 1, text } if text == "Title")
        );
        assert!(
            matches!(&doc.pages[0].elements[1], Element::Heading { level: 2, text } if text == "Subtitle")
        );
    }

    #[test]
    fn test_assemble_consecutive_code_blocks_merged() {
        let blocks = vec![
            ce(make_block("fn main() {"), BlockType::CodeBlock),
            ce(make_block("    println!(\"hi\");"), BlockType::CodeBlock),
            ce(make_block("}"), BlockType::CodeBlock),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
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
            ce(make_block("- First"), BlockType::ListItem),
            ce(make_block("- Second"), BlockType::ListItem),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
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
            ce(make_block("1. First"), BlockType::ListItem),
            ce(make_block("2. Second"), BlockType::ListItem),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        if let Element::List { ordered, items } = &doc.pages[0].elements[0] {
            assert!(ordered);
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected ordered List");
        }
    }

    #[test]
    fn test_assemble_paragraphs_far_apart_stay_separate() {
        // Paragraphs at very different Y positions should stay separate
        let b1 = RawTextBlock {
            text: "Some text.".to_string(),
            x: 72.0,
            y: 100.0,
            end_x: 200.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let b2 = RawTextBlock {
            text: "More text.".to_string(),
            x: 72.0,
            y: 300.0,
            end_x: 200.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let blocks = vec![
            ClassifiedElement::Text(b1, BlockType::Paragraph),
            ClassifiedElement::Text(b2, BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages[0].elements.len(), 2);
    }

    #[test]
    fn test_assemble_mixed_content() {
        let blocks = vec![
            ce(make_block("Title"), BlockType::Heading(1)),
            ce(make_block("Intro text"), BlockType::Paragraph),
            ce(make_block("let x = 1;"), BlockType::CodeBlock),
            ce(make_block("- item"), BlockType::ListItem),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
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

    #[test]
    fn test_bold_font_produces_bold_paragraph() {
        let blocks = vec![ce(
            make_block_with_font("Bold text", "Helvetica-Bold"),
            BlockType::Paragraph,
        )];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        if let Element::Paragraph { text } = &doc.pages[0].elements[0] {
            assert!(text.segments[0].bold);
            assert!(!text.segments[0].italic);
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn test_italic_font_produces_italic_paragraph() {
        let blocks = vec![ce(
            make_block_with_font("Italic text", "Helvetica-Oblique"),
            BlockType::Paragraph,
        )];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        if let Element::Paragraph { text } = &doc.pages[0].elements[0] {
            assert!(!text.segments[0].bold);
            assert!(text.segments[0].italic);
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn test_bold_italic_font_produces_both() {
        let blocks = vec![ce(
            make_block_with_font("Bold italic text", "Helvetica-BoldOblique"),
            BlockType::Paragraph,
        )];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        if let Element::Paragraph { text } = &doc.pages[0].elements[0] {
            assert!(text.segments[0].bold);
            assert!(text.segments[0].italic);
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn test_plain_font_no_bold_no_italic() {
        let blocks = vec![ce(
            make_block_with_font("Plain text", "Helvetica"),
            BlockType::Paragraph,
        )];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        if let Element::Paragraph { text } = &doc.pages[0].elements[0] {
            assert!(!text.segments[0].bold);
            assert!(!text.segments[0].italic);
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn test_consecutive_headings_same_level_merged() {
        let b1 = RawTextBlock {
            text: "Client - Predefined product list (To review by".to_string(),
            x: 50.0,
            y: 700.0,
            end_x: 600.0,
            font_size: 26.7,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let b2 = RawTextBlock {
            text: "ACME)".to_string(),
            x: 50.0,
            y: 720.0,
            end_x: 130.0,
            font_size: 26.7,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let blocks = vec![
            ClassifiedElement::Text(b1, BlockType::Heading(2)),
            ClassifiedElement::Text(b2, BlockType::Heading(2)),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages[0].elements.len(), 1);
        if let Element::Heading { level, text } = &doc.pages[0].elements[0] {
            assert_eq!(*level, 2);
            assert!(text.contains("Client"));
            assert!(text.contains("ACME)"));
        } else {
            panic!("Expected merged heading");
        }
    }

    #[test]
    fn test_assemble_image_element() {
        use crate::converter::pdf::extractor::RawImage;
        let blocks = vec![
            ce(make_block("Before image"), BlockType::Paragraph),
            ClassifiedElement::Image(RawImage {
                data: vec![0x89, 0x50, 0x4E, 0x47],
                width: 200,
                height: 100,
            }),
            ce(make_block("After image"), BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages[0].elements.len(), 3);
        assert!(matches!(
            &doc.pages[0].elements[0],
            Element::Paragraph { .. }
        ));
        if let Element::Image { data, alt } = &doc.pages[0].elements[1] {
            assert_eq!(data, &vec![0x89, 0x50, 0x4E, 0x47]);
            assert!(alt.is_none());
        } else {
            panic!("Expected Image element");
        }
        assert!(matches!(
            &doc.pages[0].elements[2],
            Element::Paragraph { .. }
        ));
    }

    #[test]
    fn test_url_continuation_merged_across_line_break() {
        let b1 = RawTextBlock {
            text: "https://example.com/very/long/path/that-".to_string(),
            x: 72.0,
            y: 700.0,
            end_x: 500.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let b2 = RawTextBlock {
            text: "continues/here".to_string(),
            x: 72.0,
            y: 714.0, // within 1.5 * font_size = 18.0
            end_x: 200.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let blocks = vec![
            ClassifiedElement::Text(b1, BlockType::Paragraph),
            ClassifiedElement::Text(b2, BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages[0].elements.len(), 1);
        if let Element::Paragraph { text } = &doc.pages[0].elements[0] {
            assert_eq!(
                text.segments[0].text,
                "https://example.com/very/long/path/that-continues/here"
            );
        } else {
            panic!("Expected merged Paragraph");
        }
    }

    #[test]
    fn test_hyphenated_word_break_merged() {
        // Hyphenated word break should be merged with hyphen removed
        let b1 = RawTextBlock {
            text: "some regular text that ends with a hyphen-".to_string(),
            x: 72.0,
            y: 700.0,
            end_x: 500.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let b2 = RawTextBlock {
            text: "ated word".to_string(),
            x: 72.0,
            y: 714.0,
            end_x: 200.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let blocks = vec![
            ClassifiedElement::Text(b1, BlockType::Paragraph),
            ClassifiedElement::Text(b2, BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages[0].elements.len(), 1);
        if let Element::Paragraph { text } = &doc.pages[0].elements[0] {
            assert_eq!(
                text.segments[0].text,
                "some regular text that ends with a hyphenated word"
            );
        } else {
            panic!("Expected merged Paragraph");
        }
    }

    #[test]
    fn test_continuation_paragraph_merged() {
        // Text that doesn't end with sentence-ending punctuation merges with next line
        let b1 = RawTextBlock {
            text: "The title of the project is automatically created from the project name"
                .to_string(),
            x: 72.0,
            y: 700.0,
            end_x: 500.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let b2 = RawTextBlock {
            text: "and the project ID (see detail for each below).".to_string(),
            x: 72.0,
            y: 714.0,
            end_x: 400.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let blocks = vec![
            ClassifiedElement::Text(b1, BlockType::Paragraph),
            ClassifiedElement::Text(b2, BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages[0].elements.len(), 1);
        if let Element::Paragraph { text } = &doc.pages[0].elements[0] {
            assert!(text.segments[0]
                .text
                .contains("project name and the project ID"));
        } else {
            panic!("Expected merged Paragraph");
        }
    }

    #[test]
    fn test_separate_sentences_not_merged_at_different_x() {
        // Paragraphs at different X positions should not merge
        let b1 = RawTextBlock {
            text: "First sentence".to_string(),
            x: 72.0,
            y: 700.0,
            end_x: 200.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let b2 = RawTextBlock {
            text: "Different column".to_string(),
            x: 300.0,
            y: 714.0,
            end_x: 450.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            has_bold: false,
            has_italic: false,
        };
        let blocks = vec![
            ClassifiedElement::Text(b1, BlockType::Paragraph),
            ClassifiedElement::Text(b2, BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        assert_eq!(doc.pages[0].elements.len(), 2);
    }

    #[test]
    fn test_noise_filtering_page_numbers() {
        let blocks = vec![
            ce(make_block("Real content"), BlockType::Paragraph),
            ce(make_block("1/3"), BlockType::Paragraph),
            ce(make_block("More content"), BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        // "1/3" should be filtered out
        let texts: Vec<&str> = doc.pages[0]
            .elements
            .iter()
            .filter_map(|el| match el {
                Element::Paragraph { text } => Some(text.segments[0].text.as_str()),
                _ => None,
            })
            .collect();
        assert!(!texts.contains(&"1/3"));
    }

    #[test]
    fn test_noise_filtering_onenote() {
        let blocks = vec![
            ce(make_block("Real content"), BlockType::Paragraph),
            ce(make_block("OneNote"), BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        let texts: Vec<&str> = doc.pages[0]
            .elements
            .iter()
            .filter_map(|el| match el {
                Element::Paragraph { text } => Some(text.segments[0].text.as_str()),
                _ => None,
            })
            .collect();
        assert!(!texts.contains(&"OneNote"));
    }

    #[test]
    fn test_noise_filtering_timestamp() {
        let blocks = vec![
            ce(make_block("Real content"), BlockType::Paragraph),
            ce(make_block("12/03/2026, 12:41"), BlockType::Paragraph),
        ];
        let doc = Assembler::assemble(vec![blocks], empty_metadata(), &test_metrics());
        let texts: Vec<&str> = doc.pages[0]
            .elements
            .iter()
            .filter_map(|el| match el {
                Element::Paragraph { text } => Some(text.segments[0].text.as_str()),
                _ => None,
            })
            .collect();
        assert!(!texts.contains(&"12/03/2026, 12:41"));
    }

    #[test]
    fn test_append_continuation_hyphen() {
        let mut text = "Précédent/Suivan-".to_string();
        Assembler::append_continuation(&mut text, "t");
        assert_eq!(text, "Précédent/Suivant");
    }

    #[test]
    fn test_append_continuation_space() {
        let mut text = "some text that".to_string();
        Assembler::append_continuation(&mut text, "continues here");
        assert_eq!(text, "some text that continues here");
    }

    #[test]
    fn test_merge_cross_page_tables_same_columns() {
        let page1 = Page {
            elements: vec![
                Element::Heading {
                    level: 1,
                    text: "Title".to_string(),
                },
                Element::Table {
                    headers: vec!["A".to_string(), "B".to_string()],
                    rows: vec![vec!["1".to_string(), "2".to_string()]],
                },
            ],
        };
        let page2 = Page {
            elements: vec![
                Element::Table {
                    headers: vec!["C".to_string(), "D".to_string()],
                    rows: vec![vec!["5".to_string(), "6".to_string()]],
                },
                Element::Paragraph {
                    text: RichText {
                        segments: vec![TextSegment {
                            text: "After table".to_string(),
                            bold: false,
                            italic: false,
                            code: false,
                            link: None,
                        }],
                    },
                },
            ],
        };

        let result = Assembler::merge_cross_page_tables(vec![page1, page2]);

        // Page 1 table should now include page 2's table rows
        if let Element::Table { headers, rows } = &result[0].elements[1] {
            assert_eq!(headers, &["A", "B"]);
            assert_eq!(rows.len(), 3); // original 1 row + header row + 1 data row from page2
            assert_eq!(rows[0], vec!["1", "2"]);
            assert_eq!(rows[1], vec!["C", "D"]); // page2 header becomes data row
            assert_eq!(rows[2], vec!["5", "6"]);
        } else {
            panic!("Expected Table");
        }

        // Remaining content from page 2 should still exist
        assert_eq!(result.len(), 2);
        assert!(matches!(&result[1].elements[0], Element::Paragraph { .. }));
    }

    #[test]
    fn test_merge_cross_page_tables_different_columns_not_merged() {
        let page1 = Page {
            elements: vec![Element::Table {
                headers: vec!["A".to_string(), "B".to_string()],
                rows: vec![vec!["1".to_string(), "2".to_string()]],
            }],
        };
        let page2 = Page {
            elements: vec![Element::Table {
                headers: vec!["X".to_string(), "Y".to_string(), "Z".to_string()],
                rows: vec![],
            }],
        };

        let result = Assembler::merge_cross_page_tables(vec![page1, page2]);
        assert_eq!(result.len(), 2); // Not merged, different column count
    }

    #[test]
    fn test_merge_cross_page_tables_no_table() {
        let page1 = Page {
            elements: vec![Element::Heading {
                level: 1,
                text: "Title".to_string(),
            }],
        };
        let page2 = Page {
            elements: vec![Element::Table {
                headers: vec!["A".to_string()],
                rows: vec![],
            }],
        };

        let result = Assembler::merge_cross_page_tables(vec![page1, page2]);
        assert_eq!(result.len(), 2); // Not merged, page1 doesn't end with table
    }
}
