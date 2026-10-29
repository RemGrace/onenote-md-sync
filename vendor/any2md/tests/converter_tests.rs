use any2md::converter::{Converter, ConverterRegistry};
use any2md::error::ConvertError;
use any2md::model::document::*;
use any2md::model::options::ConvertOptions;
use std::path::Path;

struct MockConverter;

impl Converter for MockConverter {
    fn name(&self) -> &str {
        "mock"
    }

    fn supported_extensions(&self) -> &[&str] {
        &["mock", "mck"]
    }

    fn convert(&self, _input: &Path, _options: &ConvertOptions) -> Result<Document, ConvertError> {
        Ok(Document {
            metadata: Metadata {
                title: Some("Mock".to_string()),
                author: None,
                date: None,
            },
            pages: vec![],
        })
    }
}

#[test]
fn test_registry_find_by_extension() {
    let mut registry = ConverterRegistry::new();
    registry.register(Box::new(MockConverter));
    assert!(registry.find_by_extension("mock").is_some());
    assert!(registry.find_by_extension("mck").is_some());
    assert!(registry.find_by_extension("unknown").is_none());
}

#[test]
fn test_converter_trait() {
    let converter = MockConverter;
    assert_eq!(converter.name(), "mock");
    assert_eq!(converter.supported_extensions(), &["mock", "mck"]);

    let opts = ConvertOptions::default();
    let doc = converter.convert(Path::new("test.mock"), &opts).unwrap();
    assert_eq!(doc.metadata.title, Some("Mock".to_string()));
}
