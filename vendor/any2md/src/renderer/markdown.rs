use crate::error::ConvertError;
use crate::model::document::*;
use crate::model::options::{ConvertOptions, ImageMode};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::fs;
use tracing::debug;

pub struct MarkdownRenderer;

impl MarkdownRenderer {
    pub fn render(doc: &Document, opts: &ConvertOptions) -> Result<String, ConvertError> {
        let mut out = String::new();
        let mut image_counter: usize = 0;
        debug!(pages = doc.pages.len(), image_mode = ?opts.image_mode, "Rendering to markdown");

        Self::render_metadata(&doc.metadata, &mut out);

        let non_empty_pages: Vec<(usize, &Page)> = doc
            .pages
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.elements.is_empty())
            .collect();

        for (i, (page_idx, page)) in non_empty_pages.iter().enumerate() {
            if i > 0 {
                out.push_str(&format!("\n<!-- page {} -->\n\n---\n\n", page_idx + 1));
            }
            for (j, element) in page.elements.iter().enumerate() {
                if j > 0 {
                    out.push('\n');
                }
                Self::render_element(element, opts, &mut out, &mut image_counter)?;
            }
        }

        Ok(out)
    }

    fn render_metadata(meta: &Metadata, out: &mut String) {
        if let Some(title) = &meta.title {
            out.push_str(&format!("# {}\n\n", title));
        }
        let mut meta_parts = Vec::new();
        if let Some(author) = &meta.author {
            meta_parts.push(format!("**Author:** {}", author));
        }
        if let Some(date) = &meta.date {
            meta_parts.push(format!("**Date:** {}", date));
        }
        if !meta_parts.is_empty() {
            out.push_str(&meta_parts.join(" | "));
            out.push_str("\n\n");
        }
    }

    fn render_element(
        el: &Element,
        opts: &ConvertOptions,
        out: &mut String,
        image_counter: &mut usize,
    ) -> Result<(), ConvertError> {
        match el {
            Element::Heading { level, text } => {
                let hashes = "#".repeat(*level as usize);
                out.push_str(&format!("{} {}\n", hashes, text));
            }
            Element::Paragraph { text } => {
                out.push_str(&Self::render_rich_text(text));
                out.push('\n');
            }
            Element::CodeBlock { language, code } => {
                let lang = language.as_deref().unwrap_or("");
                out.push_str(&format!("```{}\n{}\n```\n", lang, code));
            }
            Element::List { ordered, items } => {
                Self::render_list_items(items, *ordered, 0, out);
            }
            Element::Table { headers, rows } => {
                Self::render_table(headers, rows, out);
            }
            Element::Image { data, alt } => {
                Self::render_image(data, alt.as_deref(), opts, out, image_counter)?;
            }
            Element::HorizontalRule => {
                out.push_str("---\n");
            }
            Element::BlockQuote { text } => {
                let rendered = Self::render_rich_text(text);
                for line in rendered.lines() {
                    out.push_str(&format!("> {}\n", line));
                }
            }
        }
        Ok(())
    }

    fn render_rich_text(rt: &RichText) -> String {
        let mut s = String::new();
        for seg in &rt.segments {
            let mut text = seg.text.clone();
            if seg.bold {
                text = format!("**{}**", text);
            }
            if seg.italic {
                text = format!("*{}*", text);
            }
            if seg.code {
                text = format!("`{}`", text);
            }
            if let Some(url) = &seg.link {
                text = format!("[{}]({})", text, url);
            }
            s.push_str(&text);
        }
        s
    }

    fn render_list_items(items: &[ListItem], ordered: bool, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);
        for (i, item) in items.iter().enumerate() {
            let marker = if ordered {
                format!("{}. ", i + 1)
            } else {
                "- ".to_string()
            };
            out.push_str(&format!(
                "{}{}{}\n",
                indent,
                marker,
                Self::render_rich_text(&item.text)
            ));
            if !item.children.is_empty() {
                Self::render_list_items(&item.children, ordered, depth + 1, out);
            }
        }
    }

    fn render_table(headers: &[String], rows: &[Vec<String>], out: &mut String) {
        out.push_str("| ");
        out.push_str(&headers.join(" | "));
        out.push_str(" |\n");

        out.push_str("| ");
        out.push_str(
            &headers
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | "),
        );
        out.push_str(" |\n");

        for row in rows {
            out.push_str("| ");
            out.push_str(&row.join(" | "));
            out.push_str(" |\n");
        }
    }

    fn render_image(
        data: &[u8],
        alt: Option<&str>,
        opts: &ConvertOptions,
        out: &mut String,
        image_counter: &mut usize,
    ) -> Result<(), ConvertError> {
        let alt_text = alt.unwrap_or("image");
        match opts.image_mode {
            ImageMode::Inline => {
                let encoded = BASE64.encode(data);
                let mime = match Self::detect_image_ext(data) {
                    "jpg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    _ => "image/png",
                };
                out.push_str(&format!(
                    "![{}](data:{};base64,{})\n",
                    alt_text, mime, encoded
                ));
            }
            ImageMode::Extract => {
                *image_counter += 1;
                // Detect image format from magic bytes
                let ext = Self::detect_image_ext(data);
                let filename = format!("img_{}.{}", image_counter, ext);
                let dir = &opts.image_output_dir;

                fs::create_dir_all(dir)?;

                let file_path = dir.join(&filename);
                debug!(path = %file_path.display(), bytes = data.len(), "Saving extracted image");
                fs::write(&file_path, data)?;

                // Use relative path with the directory name
                let dir_name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "images".to_string());
                out.push_str(&format!("![{}]({}/{})\n", alt_text, dir_name, filename));
            }
        }
        Ok(())
    }

    /// Detect image format from magic bytes and return the file extension.
    fn detect_image_ext(data: &[u8]) -> &'static str {
        if data.len() >= 4 && &data[0..4] == b"\x89PNG" {
            "png"
        } else if data.len() >= 2 && &data[0..2] == b"\xff\xd8" {
            "jpg"
        } else if data.len() >= 4 && &data[0..4] == b"GIF8" {
            "gif"
        } else if data.len() >= 4
            && &data[0..4] == b"RIFF"
            && data.len() >= 12
            && &data[8..12] == b"WEBP"
        {
            "webp"
        } else {
            "png"
        }
    }
}
