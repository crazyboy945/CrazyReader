use crate::model::{Block, Inline, Page, Style};

pub const TARGET_PAGE_H: f32 = 900.0;

pub fn est_block_height(b: &Block, chars_per_line: usize) -> f32 {
    let text = b.plain_text();
    let lines = if text.is_empty() {
        1
    } else {
        text.chars().count().div_ceil(chars_per_line).max(1)
    };
    match b {
        Block::Heading { level, .. } => match *level {
            1 => (lines * 42).max(46) as f32,
            2 => (lines * 36).max(40) as f32,
            3 => (lines * 30).max(34) as f32,
            _ => (lines * 26).max(30) as f32,
        },
        Block::Code { text, .. } => {
            let n = text.lines().count().max(1);
            (n * 22 + 24) as f32
        }
        Block::Image { .. } => 260.0,
        Block::PdfPage { .. } => 760.0,
        Block::Divider => 24.0,
        Block::Quote { .. } => (lines * 26 + 18) as f32,
        _ => (lines * 26) as f32,
    }
}

pub fn paginate(blocks: Vec<Block>) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut cur: Vec<Block> = Vec::new();
    let mut h = 0.0f32;
    for b in blocks {
        let bh = est_block_height(&b, 48);
        let heading_break = matches!(b, Block::Heading { level: 1 | 2, .. });
        let force_new = (bh > TARGET_PAGE_H * 0.9 && !cur.is_empty())
            || (heading_break && h > TARGET_PAGE_H * 0.55 && !cur.is_empty());
        if force_new && !cur.is_empty() {
            pages.push(make_page(std::mem::take(&mut cur)));
            h = 0.0;
        }
        h += bh;
        cur.push(b);
    }
    if !cur.is_empty() {
        pages.push(make_page(cur));
    }
    if pages.is_empty() {
        pages.push(Page {
            blocks: vec![Block::Para {
                inlines: vec![Inline {
                    text: "(空文档)".to_string(),
                    style: Style::Normal,
                    anchor: None,
                }],
            }],
        });
    }
    pages
}

fn make_page(blocks: Vec<Block>) -> Page {
    Page { blocks }
}

pub fn decode_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let (cow, _, _) = encoding_rs::GBK.decode(bytes);
    cow.into_owned()
}

/// Resolve the effective style from a nesting stack (e.g. bold inside italic).
pub fn combine_style(stack: &[crate::model::Style]) -> crate::model::Style {
    use crate::model::Style;
    let (mut bold, mut italic) = (false, false);
    for s in stack {
        match s {
            Style::BoldItalic => {
                bold = true;
                italic = true;
            }
            Style::Bold => bold = true,
            Style::Italic => italic = true,
            Style::Code => return Style::Code,
            Style::Normal => {}
        }
    }
    match (bold, italic) {
        (true, true) => Style::BoldItalic,
        (true, false) => Style::Bold,
        (false, true) => Style::Italic,
        (false, false) => Style::Normal,
    }
}
