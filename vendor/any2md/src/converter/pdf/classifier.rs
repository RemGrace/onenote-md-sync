use crate::converter::pdf::extractor::{RawElement, RawImage, RawPage, RawTextBlock};
use crate::model::document::Element;
use tracing::{debug, trace};

#[derive(Debug, Clone, PartialEq)]
pub enum BlockType {
    Heading(u8),
    CodeBlock,
    ListItem,
    Paragraph,
}

#[derive(Debug, Clone)]
pub enum ClassifiedElement {
    Text(RawTextBlock, BlockType),
    Image(RawImage),
    PreBuilt(Element),
}

pub struct Classifier;

impl Classifier {
    pub fn classify(pages: &[RawPage]) -> Vec<Vec<ClassifiedElement>> {
        let avg_font_size = Self::average_font_size(pages);
        debug!(
            avg_font_size,
            page_count = pages.len(),
            "Starting classification"
        );

        pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .map(|el| match el {
                        RawElement::Text(block) => {
                            let block_type = Self::classify_block(block, avg_font_size);
                            ClassifiedElement::Text(block.clone(), block_type)
                        }
                        RawElement::Image(img) => ClassifiedElement::Image(img.clone()),
                    })
                    .collect()
            })
            .collect()
    }

    fn classify_block(block: &RawTextBlock, avg_font_size: f64) -> BlockType {
        let result = if Self::is_code(block) {
            BlockType::CodeBlock
        } else if Self::is_heading(block, avg_font_size) {
            let level = Self::heading_level(block.font_size, avg_font_size);
            BlockType::Heading(level)
        } else if Self::is_list_item(block) {
            BlockType::ListItem
        } else {
            BlockType::Paragraph
        };
        trace!(
            text = %block.text.chars().take(50).collect::<String>(),
            font = %block.font_name,
            font_size = block.font_size,
            x = block.x,
            y = block.y,
            block_type = ?result,
            "Classified block"
        );
        result
    }

    fn is_code(block: &RawTextBlock) -> bool {
        let mono_fonts = [
            "courier",
            "consolas",
            "menlo",
            "monaco",
            "monospace",
            "source code",
            "fira code",
        ];
        let font_lower = block.font_name.to_lowercase();
        mono_fonts.iter().any(|f| font_lower.contains(f))
    }

    fn is_heading(block: &RawTextBlock, avg_font_size: f64) -> bool {
        block.font_size > avg_font_size * 1.2
    }

    fn heading_level(font_size: f64, avg_font_size: f64) -> u8 {
        let ratio = font_size / avg_font_size;
        if ratio >= 2.0 {
            1
        } else if ratio >= 1.5 {
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

    pub fn is_bold(font_name: &str) -> bool {
        font_name.to_lowercase().contains("bold")
    }

    pub fn is_italic(font_name: &str) -> bool {
        let lower = font_name.to_lowercase();
        lower.contains("italic") || lower.contains("oblique")
    }

    fn average_font_size(pages: &[RawPage]) -> f64 {
        // Use the most common (mode) font size as the baseline,
        // so that headings don't skew the average upward.
        let mut freq: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for page in pages {
            for el in &page.elements {
                if let RawElement::Text(block) = el {
                    let key = (block.font_size * 100.0) as u64;
                    *freq.entry(key).or_insert(0) += 1;
                }
            }
        }
        if freq.is_empty() {
            12.0
        } else {
            let mode_key = freq.into_iter().max_by_key(|&(_, count)| count).unwrap().0;
            mode_key as f64 / 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_block(text: &str, font_size: f64, font_name: &str) -> RawTextBlock {
        let end_x = 72.0 + text.chars().count() as f64 * font_size * 0.5;
        RawTextBlock {
            text: text.to_string(),
            x: 72.0,
            y: 700.0,
            end_x,
            font_size,
            font_name: font_name.to_string(),
            has_bold: false,
            has_italic: false,
        }
    }

    fn make_page(blocks: Vec<RawTextBlock>) -> RawPage {
        RawPage {
            elements: blocks.into_iter().map(RawElement::Text).collect(),
        }
    }

    fn get_block_type(el: &ClassifiedElement) -> &BlockType {
        match el {
            ClassifiedElement::Text(_, bt) => bt,
            _ => panic!("Expected Text element"),
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
        assert_eq!(*get_block_type(&result[0][0]), BlockType::Heading(1));
        assert_eq!(*get_block_type(&result[0][1]), BlockType::Paragraph);
    }

    #[test]
    fn test_classify_code_by_font() {
        let pages = vec![make_page(vec![
            make_block("let x = 1;", 12.0, "Courier"),
            make_block("normal text", 12.0, "Helvetica"),
        ])];
        let result = Classifier::classify(&pages);
        assert_eq!(*get_block_type(&result[0][0]), BlockType::CodeBlock);
        assert_eq!(*get_block_type(&result[0][1]), BlockType::Paragraph);
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
        assert_eq!(*get_block_type(&result[0][0]), BlockType::ListItem);
        assert_eq!(*get_block_type(&result[0][1]), BlockType::ListItem);
        assert_eq!(*get_block_type(&result[0][2]), BlockType::ListItem);
        assert_eq!(*get_block_type(&result[0][3]), BlockType::Paragraph);
    }

    #[test]
    fn test_code_has_priority_over_heading() {
        let pages = vec![make_page(vec![
            make_block("fn main()", 24.0, "Courier"),
            make_block("normal", 12.0, "Helvetica"),
        ])];
        let result = Classifier::classify(&pages);
        assert_eq!(*get_block_type(&result[0][0]), BlockType::CodeBlock);
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
        assert_eq!(*get_block_type(&result[0][0]), BlockType::Heading(1));
        assert_eq!(*get_block_type(&result[0][1]), BlockType::Heading(2));
        assert_eq!(*get_block_type(&result[0][2]), BlockType::Heading(3));
    }

    #[test]
    fn test_classify_passes_through_images() {
        use crate::converter::pdf::extractor::RawImage;
        let page = RawPage {
            elements: vec![
                RawElement::Text(make_block("Hello", 12.0, "Helvetica")),
                RawElement::Image(RawImage {
                    data: vec![0xFF, 0xD8],
                    width: 100,
                    height: 50,
                }),
                RawElement::Text(make_block("World", 12.0, "Helvetica")),
            ],
        };
        let result = Classifier::classify(&[page]);
        assert_eq!(result[0].len(), 3);
        assert!(matches!(
            &result[0][0],
            ClassifiedElement::Text(_, BlockType::Paragraph)
        ));
        assert!(matches!(&result[0][1], ClassifiedElement::Image(_)));
        assert!(matches!(
            &result[0][2],
            ClassifiedElement::Text(_, BlockType::Paragraph)
        ));
    }
}
