use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Txt,
    Markdown,
    Epub,
    Pdf,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Txt => "TXT",
            Format::Markdown => "Markdown",
            Format::Epub => "EPUB",
            Format::Pdf => "PDF",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Normal,
    Bold,
    Italic,
    BoldItalic,
    Code,
}

#[derive(Debug, Clone)]
pub struct Inline {
    pub text: String,
    pub style: Style,
    /// In-document anchor id (from <a id="..."> or heading id).
    pub anchor: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
        anchor: Option<String>,
    },
    Para {
        inlines: Vec<Inline>,
    },
    ListItem {
        inlines: Vec<Inline>,
        bullet: bool,
    },
    Quote {
        inlines: Vec<Inline>,
    },
    Code {
        text: String,
        /// Fence info string from markdown (`rust`); empty when unknown.
        lang: String,
    },
    /// EPUB: path inside the zip. Markdown/TXT: absolute file path.
    Image {
        src: String,
        anchor: Option<String>,
    },
    /// PDF page index, rendered lazily.
    PdfPage {
        page: usize,
    },
    Divider,
}

impl Block {
    pub fn anchor(&self) -> Option<&str> {
        match self {
            Block::Heading { anchor, .. } => anchor.as_deref(),
            Block::Image { anchor, .. } => anchor.as_deref(),
            _ => None,
        }
    }

    pub fn plain_text(&self) -> String {
        let inlines = match self {
            Block::Heading { inlines, .. } | Block::Para { inlines } | Block::Quote { inlines } => {
                inlines
            }
            Block::ListItem { inlines, .. } => inlines,
            Block::Code { text, .. } => {
                return text.clone();
            }
            Block::PdfPage { page } => return format!("PDF 第 {} 页", page + 1),
            Block::Image { .. } => return "[图片]".to_string(),
            Block::Divider => return String::new(),
        };
        inlines.iter().map(|i| i.text.as_str()).collect::<String>()
    }
}

#[derive(Debug, Clone)]
pub struct Page {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
pub struct TocEntry {
    pub title: String,
    pub depth: u16,
    pub page: usize,
    /// Byte-ish/point estimate of scroll offset within the page; refined by anchor.
    pub anchor: Option<String>,
}

pub struct Document {
    pub format: Format,
    pub title: String,
    pub path: PathBuf,
    pub pages: Vec<Page>,
    pub toc: Vec<TocEntry>,
    pub data: DocData,
}

pub enum DocData {
    Text,
    Epub(EpubData),
    Pdf(PdfData),
}

pub struct EpubData {
    pub zip_bytes: std::sync::Arc<Vec<u8>>,
}

pub struct PdfData {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RenderedImage {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}
