use crate::model::{Block, DocData, Document, Format, Page, PdfData, RenderedImage, TocEntry};
use std::path::Path;

pub fn load(path: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;

    let num_pages = hayro::hayro_syntax::Pdf::new(bytes.clone())
        .map(|pdf| pdf.pages().len())
        .map_err(|e| format!("PDF 解析失败: {e:?}"))?;

    let toc = extract_outline(&bytes);

    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名".to_string());

    let pages: Vec<Page> = (0..num_pages)
        .map(|i| Page {
            blocks: vec![Block::PdfPage { page: i }],
        })
        .collect();

    Ok(Document {
        format: Format::Pdf,
        title,
        path: path.to_path_buf(),
        pages,
        toc,
        data: DocData::Pdf(PdfData { bytes }),
    })
}

/// Render a PDF page into an RGBA pixmap (premultiplied) at the given scale.
pub fn render_page(doc_bytes: &[u8], page: usize, scale: f32) -> Result<RenderedImage, String> {
    use hayro::hayro_syntax::Pdf;
    use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

    let pdf = Pdf::new(doc_bytes.to_vec()).map_err(|e| format!("PDF 重载失败: {e:?}"))?;
    let pages = pdf.pages();
    let p = pages.get(page).ok_or("页码超出范围")?;
    let cache = RenderCache::new();
    let pixmap = render(
        p,
        &cache,
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    let width = pixmap.width();
    let height = pixmap.height();
    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for px in pixmap.data() {
        rgba.extend_from_slice(&[px.r, px.g, px.b, px.a]);
    }
    Ok(RenderedImage { width, height, rgba })
}

fn extract_outline(bytes: &[u8]) -> Vec<TocEntry> {
    let Ok(doc) = lopdf::Document::load_mem(bytes) else {
        return Vec::new();
    };
    let Ok(Some(outline)) = doc.get_outlines(None, None, &mut Default::default()) else {
        return Vec::new();
    };

    let page_order: Vec<lopdf::ObjectId> = doc.page_iter().collect();

    let mut entries = Vec::new();
    walk_outline(&doc, &outline, 0, &page_order, &mut entries);
    entries.sort_by_key(|e| e.page);
    entries
}

fn walk_outline(
    doc: &lopdf::Document,
    items: &[lopdf::Outline],
    depth: u16,
    page_order: &[lopdf::ObjectId],
    out: &mut Vec<TocEntry>,
) {
    for item in items {
        match item {
            lopdf::Outline::Destination(dest) => {
                let title = dest
                    .title()
                    .ok()
                    .and_then(|o| lopdf::decode_text_string(&o).ok())
                    .unwrap_or_default();
                let page = dest
                    .page()
                    .ok()
                    .and_then(|o| match o {
                        lopdf::Object::Reference(id) => {
                            page_order.iter().position(|p| p == id).map(|i| i as u32)
                        }
                        lopdf::Object::Integer(n) => Some((*n - 1).max(0) as u32),
                        _ => None,
                    })
                    .unwrap_or(0);
                if !title.is_empty() {
                    out.push(TocEntry {
                        title,
                        depth,
                        page: page as usize,
                        anchor: None,
                    });
                }
            }
            lopdf::Outline::SubOutlines(sub) => {
                walk_outline(doc, sub, depth + 1, page_order, out);
            }
        }
    }
}
