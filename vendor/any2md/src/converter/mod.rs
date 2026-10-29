#[cfg(feature = "audio")]
pub mod audio;
pub mod image_ocr;
pub mod pdf;
pub mod web;

use crate::error::ConvertError;
use crate::model::document::Document;
use crate::model::options::ConvertOptions;
use std::path::Path;

pub trait Converter {
    fn name(&self) -> &str;
    fn supported_extensions(&self) -> &[&str];
    fn convert(&self, input: &Path, options: &ConvertOptions) -> Result<Document, ConvertError>;
}

pub struct ConverterRegistry {
    converters: Vec<Box<dyn Converter>>,
}

impl Default for ConverterRegistry {
    fn default() -> Self {
        Self::new()
    }
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
            .find(|c| {
                c.supported_extensions()
                    .iter()
                    .any(|e| e.to_lowercase() == ext_lower)
            })
            .map(|c| c.as_ref())
    }
}
