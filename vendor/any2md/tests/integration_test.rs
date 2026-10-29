use any2md::converter::pdf::PdfConverter;
use any2md::converter::Converter;
use any2md::model::options::ConvertOptions;
use any2md::renderer::markdown::MarkdownRenderer;
use std::path::Path;

#[test]
fn test_pdf_converter_nonexistent_file_returns_error() {
    let converter = PdfConverter;
    let opts = ConvertOptions::default();
    let result = converter.convert(Path::new("tests/fixtures/nonexistent.pdf"), &opts);
    assert!(result.is_err());
}

#[test]
fn test_full_pipeline_with_document() {
    use any2md::model::document::*;
    use any2md::model::options::*;

    let doc = Document {
        metadata: Metadata {
            title: Some("Integration Test".to_string()),
            author: Some("Test Author".to_string()),
            date: None,
        },
        pages: vec![
            Page {
                elements: vec![
                    Element::Heading {
                        level: 1,
                        text: "Chapter 1".to_string(),
                    },
                    Element::Paragraph {
                        text: RichText {
                            segments: vec![
                                TextSegment {
                                    text: "This is ".to_string(),
                                    bold: false,
                                    italic: false,
                                    code: false,
                                    link: None,
                                },
                                TextSegment {
                                    text: "bold".to_string(),
                                    bold: true,
                                    italic: false,
                                    code: false,
                                    link: None,
                                },
                                TextSegment {
                                    text: " text.".to_string(),
                                    bold: false,
                                    italic: false,
                                    code: false,
                                    link: None,
                                },
                            ],
                        },
                    },
                    Element::CodeBlock {
                        language: Some("rust".to_string()),
                        code: "fn hello() {\n    println!(\"Hello!\");\n}".to_string(),
                    },
                    Element::List {
                        ordered: false,
                        items: vec![
                            ListItem {
                                text: RichText {
                                    segments: vec![TextSegment {
                                        text: "Item A".to_string(),
                                        bold: false,
                                        italic: false,
                                        code: false,
                                        link: None,
                                    }],
                                },
                                children: vec![],
                            },
                            ListItem {
                                text: RichText {
                                    segments: vec![TextSegment {
                                        text: "Item B".to_string(),
                                        bold: false,
                                        italic: false,
                                        code: false,
                                        link: None,
                                    }],
                                },
                                children: vec![],
                            },
                        ],
                    },
                ],
            },
            Page {
                elements: vec![
                    Element::Table {
                        headers: vec!["Col1".to_string(), "Col2".to_string()],
                        rows: vec![vec!["A".to_string(), "B".to_string()]],
                    },
                    Element::HorizontalRule,
                    Element::BlockQuote {
                        text: RichText {
                            segments: vec![TextSegment {
                                text: "A wise quote".to_string(),
                                bold: false,
                                italic: false,
                                code: false,
                                link: None,
                            }],
                        },
                    },
                ],
            },
        ],
    };

    let opts = ConvertOptions::default();
    let md = MarkdownRenderer::render(&doc, &opts).unwrap();

    assert!(md.contains("# Integration Test"));
    assert!(md.contains("**Author:** Test Author"));
    assert!(md.contains("# Chapter 1"));
    assert!(md.contains("**bold**"));
    assert!(md.contains("```rust"));
    assert!(md.contains("- Item A"));
    assert!(md.contains("| Col1 | Col2 |"));
    assert!(md.contains("> A wise quote"));
    assert!(md.contains("<!-- page 2 -->"));
}
