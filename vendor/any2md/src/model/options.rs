use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub image_mode: ImageMode,
    pub page_mode: PageMode,
    pub image_output_dir: PathBuf,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            image_mode: ImageMode::Extract,
            page_mode: PageMode::SingleFile,
            image_output_dir: PathBuf::from("images"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ImageMode {
    Extract,
    Inline,
}

#[derive(Debug, Clone)]
pub enum PageMode {
    SingleFile,
    SplitPages,
}
