pub mod common;
pub mod epub;
pub mod md;
pub mod pdf;
pub mod txt;

use crate::model::{Document, Format};
use std::path::Path;

pub fn format_of(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "epub" => Some(Format::Epub),
        "md" | "markdown" | "mdown" => Some(Format::Markdown),
        "pdf" => Some(Format::Pdf),
        "txt" | "text" | "log" => Some(Format::Txt),
        _ => None,
    }
}

pub fn load(path: &Path) -> Result<Document, String> {
    match format_of(path) {
        Some(Format::Epub) => epub::load(path),
        Some(Format::Markdown) => md::load(path),
        Some(Format::Pdf) => pdf::load(path),
        Some(Format::Txt) => txt::load(path),
        None => Err("不支持的文件格式（支持 epub / md / pdf / txt）".to_string()),
    }
}
