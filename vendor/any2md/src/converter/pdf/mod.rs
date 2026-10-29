pub mod assembler;
pub mod classifier;
pub mod extractor;
pub mod table_detector;

use crate::converter::Converter;
use crate::error::ConvertError;
use crate::model::document::{Document, Metadata};
use crate::model::options::ConvertOptions;
use assembler::Assembler;
use classifier::{ClassifiedElement, Classifier};
use extractor::{PageMetrics, PdfExtractor, RawPage};
use std::path::Path;
use table_detector::TableDetector;
use tracing::{debug, info};

pub struct PdfConverter;

impl Converter for PdfConverter {
    fn name(&self) -> &str {
        "pdf"
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pdf"]
    }

    fn convert(&self, input: &Path, _options: &ConvertOptions) -> Result<Document, ConvertError> {
        info!(input = %input.display(), "PDF conversion started");

        debug!("Phase 1: Extracting raw elements and metadata");
        let (raw_pages, pdf_meta) = PdfExtractor::extract_with_metadata(input)?;
        debug!(pages = raw_pages.len(), "Extraction complete");

        let metrics = PageMetrics::from_pages(&raw_pages);

        debug!("Phase 2: Table detection + classification");
        let mut all_classified: Vec<Vec<ClassifiedElement>> = Vec::new();

        for raw_page in &raw_pages {
            let detection = TableDetector::detect(raw_page, &metrics);

            // Phase 2 merge: assemble non-table blocks into full text lines
            let assembled = PdfExtractor::assemble_lines(detection.remaining_elements);
            let remaining_page = RawPage {
                elements: assembled,
            };
            let mut classified = Classifier::classify(&[remaining_page])
                .into_iter()
                .next()
                .unwrap_or_default();

            // Interleave tables back at their Y positions
            for table in detection.tables {
                // Find insertion point: insert before the first classified element
                // with a Y position greater than the table's Y position
                let insert_pos = classified
                    .iter()
                    .position(|el| match el {
                        ClassifiedElement::Text(b, _) => b.y > table.y_position,
                        _ => false,
                    })
                    .unwrap_or(classified.len());
                classified.insert(insert_pos, ClassifiedElement::PreBuilt(table.element));
            }

            all_classified.push(classified);
        }

        debug!("Phase 3: Building metadata");
        let metadata = Metadata {
            title: pdf_meta.title,
            author: pdf_meta.author,
            date: pdf_meta.date,
        };

        debug!("Phase 4: Assembling document");
        let doc = Assembler::assemble(all_classified, metadata, &metrics);
        info!(
            pages = doc.pages.len(),
            elements = doc.pages.iter().map(|p| p.elements.len()).sum::<usize>(),
            "PDF conversion complete"
        );
        Ok(doc)
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
