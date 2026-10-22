use crate::loaders::common::{decode_text, paginate};
use crate::model::{Block, Document, DocData, Format, Inline, Style};
use std::path::Path;

pub fn load(path: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let text = decode_text(&bytes);
    let mut blocks = Vec::new();
    for para in text.split("\n\n") {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            continue;
        }
        let inlines = vec![Inline {
            text: trimmed.to_string(),
            style: Style::Normal,
            anchor: None,
        }];
        blocks.push(Block::Para { inlines });
    }
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名".to_string());
    Ok(Document {
        format: Format::Txt,
        title,
        path: path.to_path_buf(),
        pages: paginate(blocks),
        toc: Vec::new(),
        data: DocData::Text,
    })
}
