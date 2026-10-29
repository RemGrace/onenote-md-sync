#[derive(Debug, Clone)]
pub struct Document {
    pub metadata: Metadata,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone)]
pub enum Element {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: RichText,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    List {
        ordered: bool,
        items: Vec<ListItem>,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Image {
        data: Vec<u8>,
        alt: Option<String>,
    },
    HorizontalRule,
    BlockQuote {
        text: RichText,
    },
}

#[derive(Debug, Clone)]
pub struct RichText {
    pub segments: Vec<TextSegment>,
}

#[derive(Debug, Clone)]
pub struct TextSegment {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ListItem {
    pub text: RichText,
    pub children: Vec<ListItem>,
}
