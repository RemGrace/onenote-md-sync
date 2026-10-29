use any2md::model::document::*;
use any2md::model::options::*;
use std::path::PathBuf;

#[test]
fn test_document_creation() {
    let doc = Document {
        metadata: Metadata {
            title: Some("Test".to_string()),
            author: None,
            date: None,
        },
        pages: vec![Page {
            elements: vec![
                Element::Heading {
                    level: 1,
                    text: "Hello".to_string(),
                },
                Element::Paragraph {
                    text: RichText {
                        segments: vec![TextSegment {
                            text: "World".to_string(),
                            bold: false,
                            italic: false,
                            code: false,
                            link: None,
                        }],
                    },
                },
            ],
        }],
    };
    assert_eq!(doc.metadata.title, Some("Test".to_string()));
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].elements.len(), 2);
}

#[test]
fn test_rich_text_display() {
    let rt = RichText {
        segments: vec![
            TextSegment {
                text: "hello ".to_string(),
                bold: true,
                italic: false,
                code: false,
                link: None,
            },
            TextSegment {
                text: "world".to_string(),
                bold: false,
                italic: true,
                code: false,
                link: None,
            },
        ],
    };
    assert_eq!(rt.segments.len(), 2);
    assert!(rt.segments[0].bold);
    assert!(rt.segments[1].italic);
}

#[test]
fn test_all_element_variants() {
    let elements: Vec<Element> = vec![
        Element::Heading {
            level: 1,
            text: "H1".to_string(),
        },
        Element::Paragraph {
            text: RichText { segments: vec![] },
        },
        Element::CodeBlock {
            language: Some("rust".to_string()),
            code: "let x = 1;".to_string(),
        },
        Element::List {
            ordered: false,
            items: vec![ListItem {
                text: RichText {
                    segments: vec![TextSegment {
                        text: "item".to_string(),
                        bold: false,
                        italic: false,
                        code: false,
                        link: None,
                    }],
                },
                children: vec![],
            }],
        },
        Element::Table {
            headers: vec!["A".to_string()],
            rows: vec![vec!["1".to_string()]],
        },
        Element::Image {
            data: vec![0u8],
            alt: Some("img".to_string()),
        },
        Element::HorizontalRule,
        Element::BlockQuote {
            text: RichText { segments: vec![] },
        },
    ];
    assert_eq!(elements.len(), 8);
}

#[test]
fn test_convert_options_defaults() {
    let opts = ConvertOptions::default();
    assert!(matches!(opts.image_mode, ImageMode::Extract));
    assert!(matches!(opts.page_mode, PageMode::SingleFile));
    assert_eq!(opts.image_output_dir, PathBuf::from("images"));
}

#[test]
fn test_image_mode_variants() {
    let extract = ImageMode::Extract;
    let inline = ImageMode::Inline;
    assert!(matches!(extract, ImageMode::Extract));
    assert!(matches!(inline, ImageMode::Inline));
}
