use crate::loaders::common::{decode_text, paginate};
use crate::model::{
    Block, DocData, Document, Format, Inline, Style, TocEntry,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use std::path::Path;

fn slug(text: &str) -> String {
    let mut s = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            s.push(c.to_lowercase().next().unwrap_or(c));
        } else if c.is_whitespace() {
            s.push('-');
        }
    }
    s
}

pub fn load(path: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let text = decode_text(&bytes);

    let mut blocks: Vec<Block> = Vec::new();
    let mut toc: Vec<TocEntry> = Vec::new();

    let mut cur_inlines: Vec<Inline> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut in_code = false;
    let mut in_quote = 0usize;
    let mut code_buf = String::new();
    let mut code_lang = String::new();
    let mut pending_img: Option<String> = None;
    let mut heading: Option<(u8, Vec<Inline>, bool)> = None; // (level, inlines, captured_id)
    let mut pending_anchor: Option<String> = None;

    let parser = Parser::new_ext(&text, pulldown_cmark::Options::all());
    for ev in parser {
        match ev {
            Event::Start(Tag::Heading { level, id, .. }) => {
                style_stack.clear();
                heading = Some((level_num(level), Vec::new(), id.is_some()));
                pending_anchor = id.map(|c| c.into_string());
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((lvl, _, _)) = heading.take() {
                    let inlines = std::mem::take(&mut cur_inlines);
                    let anchor = pending_anchor
                        .take()
                        .or_else(|| inlines.first().map(|i| slug(&i.text)));
                    toc.push(TocEntry {
                        title: inlines.iter().map(|i| i.text.as_str()).collect::<String>(),
                        depth: (lvl - 1) as u16,
                        page: 0,
                        anchor: anchor.clone(),
                    });
                    blocks.push(Block::Heading {
                        level: lvl,
                        inlines,
                        anchor,
                    });
                }
                cur_inlines.clear();
            }
            Event::Start(Tag::Paragraph) => {
                style_stack.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                if in_quote == 0 && !cur_inlines.is_empty() {
                    let anchor = pending_anchor.take();
                    let inlines = std::mem::take(&mut cur_inlines);
                    let _ = anchor;
                    blocks.push(Block::Para { inlines });
                }
            }
            Event::Start(Tag::Strong) => style_stack.push(Style::Bold),
            Event::End(TagEnd::Strong) => {
                style_stack.pop();
            }
            Event::Start(Tag::Emphasis) => style_stack.push(Style::Italic),
            Event::End(TagEnd::Emphasis) => {
                style_stack.pop();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                code_buf.clear();
                code_lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split([',', ' ', ';', ':'])
                        .next()
                        .unwrap_or_default()
                        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '#')
                        .to_ascii_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                blocks.push(Block::Code {
                    text: code_buf.trim_end().to_string(),
                    lang: std::mem::take(&mut code_lang),
                });
            }
            Event::Start(Tag::List(_)) => {
                // A nested list interrupts the current item: emit its text first.
                if !cur_inlines.is_empty() {
                    blocks.push(Block::ListItem {
                        inlines: std::mem::take(&mut cur_inlines),
                        bullet: true,
                    });
                }
            }
            Event::End(TagEnd::List(_)) => {}
            Event::Start(Tag::Item) => {
                style_stack.clear();
            }
            Event::End(TagEnd::Item) => {
                if !cur_inlines.is_empty() {
                    blocks.push(Block::ListItem {
                        inlines: std::mem::take(&mut cur_inlines),
                        bullet: true,
                    });
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                style_stack.clear();
                in_quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                in_quote = in_quote.saturating_sub(1);
                blocks.push(Block::Quote {
                    inlines: std::mem::take(&mut cur_inlines),
                });
            }
            Event::InlineHtml(_) | Event::Html(_) => {}
            Event::Start(Tag::Image { dest_url, .. }) => {
                pending_img = Some(dest_url.to_string());
            }
            Event::End(TagEnd::Image) => {
                if let Some(src) = pending_img.take() {
                    blocks.push(Block::Image {
                        src: resolve_media(path, &src),
                        anchor: None,
                    });
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                push_inline(&mut cur_inlines, &style_stack, "\n");
            }
            Event::Rule => blocks.push(Block::Divider),
            Event::Text(t) => {
                if in_code {
                    code_buf.push_str(&t);
                } else {
                    push_inline(&mut cur_inlines, &style_stack, &t);
                }
            }
            Event::Code(c) => {
                if in_code {
                    code_buf.push_str(&c);
                } else {
                    cur_inlines.push(Inline {
                        text: c.to_string(),
                        style: Style::Code,
                        anchor: None,
                    });
                }
            }
            Event::TaskListMarker(_b) => {}
            Event::FootnoteReference(_) => {}
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
            _ => {}
        }
    }

    let pages = paginate(blocks);
    resolve_toc_pages(&mut toc, &pages);

    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名".to_string());

    Ok(Document {
        format: Format::Markdown,
        title,
        path: path.to_path_buf(),
        pages,
        toc,
        data: DocData::Text,
    })
}

/// Resolve an image target to something renderable: a local file path next to
/// the markdown document, or the untouched URL for remote images.
fn resolve_media(md_path: &Path, dest: &str) -> String {
    let dest = percent_decode(dest.trim());
    if dest.starts_with("http://")
        || dest.starts_with("https://")
        || dest.starts_with("data:")
        || dest.starts_with('#')
    {
        return dest;
    }
    let dest = dest.strip_prefix("file://").unwrap_or(&dest).to_string();
    let body = dest.strip_prefix('/').unwrap_or(&dest);
    let drive_anchored = body.len() >= 3
        && body.as_bytes()[1] == b':'
        && matches!(body.as_bytes()[2], b'\\' | b'/');
    let p = if drive_anchored {
        Path::new(body)
    } else {
        Path::new(&dest)
    };
    if p.is_absolute() {
        return p.to_string_lossy().to_string();
    }
    md_path
        .parent()
        .map(|dir| dir.join(p).to_string_lossy().to_string())
        .unwrap_or(dest)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn level_num(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn push_inline(inlines: &mut Vec<Inline>, stack: &[Style], text: &str) {
    if text.is_empty() {
        return;
    }
    let style = crate::loaders::common::combine_style(stack);
    if let Some(last) = inlines.last_mut() {
        if last.style == style && last.anchor.is_none() {
            last.text.push_str(text);
            return;
        }
    }
    inlines.push(Inline {
        text: text.to_string(),
        style,
        anchor: None,
    });
}

pub fn resolve_toc_pages(toc: &mut [TocEntry], pages: &[crate::model::Page]) {
    for entry in toc.iter_mut() {
        let Some(anchor) = &entry.anchor else { continue };
        'outer: for (pi, page) in pages.iter().enumerate() {
            for b in &page.blocks {
                if b.anchor() == Some(anchor.as_str()) {
                    entry.page = pi;
                    break 'outer;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fence_language_is_kept() {
        let dir = std::env::temp_dir().join("crazy-reader-md-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fences.md");
        std::fs::write(
            &path,
            "# t\n\n```rs\nfn a() {}\n```\n\n    indented\n\n```\nplain\n```\n",
        )
        .unwrap();
        let doc = super::load(&path).unwrap();
        let codes: Vec<(String, String)> = doc
            .pages
            .iter()
            .flat_map(|p| p.blocks.iter())
            .filter_map(|b| match b {
                super::Block::Code { text, lang } => Some((text.clone(), lang.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            codes,
            vec![
                ("fn a() {}".to_string(), "rs".to_string()),
                ("indented".to_string(), String::new()),
                ("plain".to_string(), String::new()),
            ]
        );
    }
}
