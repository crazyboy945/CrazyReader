use crate::loaders::common::paginate;
use crate::loaders::md::resolve_toc_pages;
use crate::model::{Block, DocData, Document, EpubData, Format, Inline, Style, TocEntry};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::Path;
use zip::ZipArchive;

#[derive(Default)]
struct Opf {
    manifest: HashMap<String, (String, String)>, // id -> (href, media-type)
    spine: Vec<String>,
    title: Option<String>,
    toc_href: Option<String>,
}

struct ZipSource(std::sync::Arc<Vec<u8>>, u64);
impl Read for ZipSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let pos = self.1 as usize;
        let src = &self.0;
        let n = (src.len().saturating_sub(pos)).min(buf.len());
        buf[..n].copy_from_slice(&src[pos..pos + n]);
        self.1 = (pos + n) as u64;
        Ok(n)
    }
}
impl Seek for ZipSource {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::io::SeekFrom;
        self.1 = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::End(p) => (self.0.len() as i64 + p).max(0) as u64,
            SeekFrom::Current(p) => (self.1 as i64 + p).max(0) as u64,
        };
        Ok(self.1)
    }
}

fn open_zip(bytes: &std::sync::Arc<Vec<u8>>) -> Result<ZipArchive<ZipSource>, String> {
    ZipArchive::new(ZipSource(bytes.clone(), 0)).map_err(|e| format!("EPUB 解压失败: {e}"))
}

pub fn load(path: &Path) -> Result<Document, String> {
    let zip_bytes =
        std::sync::Arc::new(std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?);
    let mut zip = open_zip(&zip_bytes)?;

    let container = read_entry(&mut zip, "META-INF/container.xml")
        .ok_or("缺少 container.xml，不是有效的 EPUB")?;
    let opf_path = find_opf_path(&container)?;
    let opf_data = read_entry(&mut zip, &opf_path).ok_or("缺少 OPF 文件")?;
    let opf = parse_opf(&opf_data, &opf_path)?;

    let mut toc: Vec<TocEntry> = Vec::new();

    let mut chapter_files: Vec<String> = Vec::new();
    for idref in &opf.spine {
        if let Some((href, mt)) = opf.manifest.get(idref) {
            if mt.contains("html") || mt.contains("xml") {
                let full = resolve_opf_path(&opf_path, href);
                if !chapter_files.contains(&full) {
                    chapter_files.push(full);
                }
            }
        }
    }

    if chapter_files.is_empty() {
        return Err("EPUB 中没有可用的章节".to_string());
    }

    let mut file_to_page: HashMap<String, usize> = HashMap::new();
    for (i, f) in chapter_files.iter().enumerate() {
        let name = f.rsplit('/').next().unwrap_or(f).to_string();
        file_to_page.insert(name, i);
        file_to_page.insert(f.clone(), i);
    }

    if let Some(ncx) = &opf.toc_href {
        if let Some(data) = read_entry(&mut zip, ncx) {
            toc = parse_ncx(&data, &file_to_page);
        }
    }
    if toc.is_empty() {
        if let Some(nav) = find_nav_doc(&opf) {
            if let Some(data) = read_entry(&mut zip, &nav) {
                toc = parse_nav(&data, &file_to_page);
            }
        }
    }

    let mut chapter_pages: Vec<Vec<crate::model::Page>> = Vec::new();
    for f in &chapter_files {
        let Some(data) = read_entry(&mut zip, f) else {
            chapter_pages.push(paginate(vec![Block::Para {
                inlines: vec![Inline {
                    text: format!("[无法读取章节 {f}]"),
                    style: Style::Normal,
                    anchor: None,
                }],
            }]));
            continue;
        };
        let cb = html_to_blocks(&data, f);
        chapter_pages.push(paginate(cb));
    }

    let mut pages: Vec<crate::model::Page> = Vec::new();
    let mut chapter_start_page: Vec<usize> = Vec::new();
    for cp in chapter_pages {
        chapter_start_page.push(pages.len());
        pages.extend(cp);
    }

    if !toc.is_empty() {
        for entry in toc.iter_mut() {
            entry.page = *chapter_start_page
                .get(entry.page.min(chapter_start_page.len() - 1))
                .unwrap_or(&0);
        }
        resolve_toc_pages(toc.as_mut_slice(), &pages);
    } else {
        for (i, f) in chapter_files.iter().enumerate() {
            let name = f.rsplit('/').next().unwrap_or(f);
            let title = name
                .strip_suffix(".xhtml")
                .or_else(|| name.strip_suffix(".html"))
                .unwrap_or(name);
            toc.push(TocEntry {
                title: title.to_string(),
                depth: 0,
                page: chapter_start_page[i],
                anchor: None,
            });
        }
        let generic = toc.iter().all(|t| {
            t.title.ends_with(".xhtml")
                || t.title.ends_with(".html")
                || t.title.ends_with(".htm")
        });
        if generic {
            toc.clear();
            for (pi, page) in pages.iter().enumerate() {
                for b in &page.blocks {
                    if let Block::Heading {
                        level: lvl @ (1 | 2),
                        inlines,
                        anchor,
                    } = b
                    {
                        toc.push(TocEntry {
                            title: inlines.iter().map(|i| i.text.as_str()).collect(),
                            depth: (*lvl as u16) - 1,
                            page: pi,
                            anchor: anchor.clone(),
                        });
                    }
                }
            }
        }
    }

    let title = opf.title.unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "未命名".to_string())
    });

    Ok(Document {
        format: Format::Epub,
        title,
        path: path.to_path_buf(),
        pages,
        toc,
        data: DocData::Epub(EpubData { zip_bytes }),
    })
}

fn find_nav_doc(opf: &Opf) -> Option<String> {
    for (_, (href, mt)) in &opf.manifest {
        if mt.contains("html") {
            let lower = href.to_lowercase();
            if lower.contains("nav") {
                return Some(resolve_opf_name(href));
            }
        }
    }
    None
}

fn resolve_opf_name(href: &str) -> String {
    href.to_string()
}

fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let actual = zip
        .file_names()
        .filter_map(|n| n.ok())
        .find(|n| n.eq_ignore_ascii_case(name))
        .map(|s| s.to_string())?;
    let mut f = zip.by_name(&actual).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn find_opf_path(xml: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(xml).to_string();
    let s = text.as_str();
    let i = s.find("full-path=\"").ok_or("container.xml 中未找到 OPF 路径")?;
    let rest = &s[i + 11..];
    let j = rest.find('"').ok_or("OPF 路径格式错误")?;
    Ok(rest[..j].to_string())
}

fn parse_opf(data: &[u8], opf_path: &str) -> Result<Opf, String> {
    let text = String::from_utf8_lossy(data).to_string();
    let mut reader = Reader::from_str(&text);
    let mut opf = Opf::default();
    let mut in_manifest = false;
    let mut in_spine = false;
    let mut in_title = false;
    let mut toc_id: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "manifest" => in_manifest = true,
                    "spine" => {
                        in_spine = true;
                        if let Some(v) = attr(&e, "toc") {
                            toc_id = toc_id.or(Some(v));
                        }
                    }
                    "title" => {
                        in_title = true;
                        if opf.title.is_none() {
                            opf.title = Some(String::new());
                        }
                    }
                    "item" => {
                        if in_manifest {
                            let id = attr(&e, "id").unwrap_or_default();
                            let href = attr(&e, "href").unwrap_or_default();
                            let mt = attr(&e, "media-type").unwrap_or_default();
                            if !id.is_empty() {
                                opf.manifest.insert(id.clone(), (href, mt.clone()));
                                if mt.contains("ncx") {
                                    toc_id = Some(id);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "item" => {
                        if in_manifest {
                            let id = attr(&e, "id").unwrap_or_default();
                            let href = attr(&e, "href").unwrap_or_default();
                            let mt = attr(&e, "media-type").unwrap_or_default();
                            if !id.is_empty() {
                                opf.manifest.insert(id.clone(), (href, mt.clone()));
                                if mt.contains("ncx") {
                                    toc_id = Some(id);
                                }
                            }
                        }
                    }
                    "itemref" => {
                        if in_spine {
                            if let Some(v) = attr(&e, "idref") {
                                opf.spine.push(v);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_title {
                    if let Ok(v) = quick_xml::escape::unescape(t.as_ref()) {
                        if let Some(slot) = opf.title.as_mut() {
                            slot.push_str(&v);
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "manifest" => in_manifest = false,
                    "spine" => in_spine = false,
                    "title" => in_title = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    if let Some(id) = toc_id {
        if let Some((href, _)) = opf.manifest.get(&id) {
            opf.toc_href = Some(resolve_opf_path(opf_path, href));
        }
    }
    if opf.toc_href.is_none() {
        opf.toc_href = Some(resolve_opf_path(opf_path, "toc.ncx"));
    }
    Ok(opf)
}

fn resolve_opf_path(base_file: &str, href: &str) -> String {
    let base_dir = match base_file.rfind('/') {
        Some(i) => &base_file[..i + 1],
        None => "",
    };
    if href.starts_with('/') {
        return href.trim_start_matches('/').to_string();
    }
    let joined = format!("{base_dir}{href}");
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(seg),
        }
    }
    parts.join("/")
}

fn parse_ncx(data: &[u8], file_to_page: &HashMap<String, usize>) -> Vec<TocEntry> {
    let text = String::from_utf8_lossy(data).to_string();
    let mut reader = Reader::from_str(&text);
    let mut toc = Vec::new();
    let mut depth: u16 = 0;
    let mut cur_label = String::new();
    let mut cur_src = String::new();
    let mut in_label = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "navPoint" => depth += 1,
                    "navLabel" => {
                        cur_label.clear();
                        cur_src.clear();
                    }
                    "text" => in_label = true,
                    "content" => {
                        if let Some(v) = attr(&e, "src") {
                            cur_src = v;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == "content" {
                    if let Some(v) = attr(&e, "src") {
                        cur_src = v;
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if in_label {
                    if let Ok(v) = quick_xml::escape::unescape(t.as_ref()) {
                        cur_label.push_str(&v);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "text" => in_label = false,
                    "navPoint" => {
                        let file = cur_src.split('#').next().unwrap_or("").to_string();
                        let anchor = cur_src.split_once('#').map(|(_, a)| a.to_string());
                        let page = lookup_page(&file, file_to_page).unwrap_or(0);
                        let title = cur_label.trim().to_string();
                        if !title.is_empty() {
                            toc.push(TocEntry {
                                title,
                                depth: depth.saturating_sub(1),
                                page,
                                anchor,
                            });
                        }
                        depth = depth.saturating_sub(1);
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    toc
}

fn parse_nav(data: &[u8], file_to_page: &HashMap<String, usize>) -> Vec<TocEntry> {
    let text = String::from_utf8_lossy(data).to_string();
    let mut reader = Reader::from_str(&text);
    let mut toc = Vec::new();
    let mut li_depth = 0usize;
    let mut cur_href: Option<String> = None;
    let mut cur_text = String::new();
    let mut in_a = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "a" => {
                        cur_href = attr(&e, "href");
                        cur_text.clear();
                        in_a = true;
                    }
                    "li" => li_depth += 1,
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_a {
                    if let Ok(v) = quick_xml::escape::unescape(t.as_ref()) {
                        cur_text.push_str(&v);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "a" {
                    in_a = false;
                    if let Some(href) = cur_href.take() {
                        let file = href.split('#').next().unwrap_or("").to_string();
                        let anchor = href.split_once('#').map(|(_, a)| a.to_string());
                        let page = lookup_page(&file, file_to_page).unwrap_or(0);
                        let title = cur_text.trim().to_string();
                        if !title.is_empty() {
                            toc.push(TocEntry {
                                title,
                                depth: (li_depth.saturating_sub(1)) as u16,
                                page,
                                anchor,
                            });
                        }
                        cur_text.clear();
                    }
                } else if name == "li" {
                    li_depth = li_depth.saturating_sub(1);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    toc
}

fn lookup_page(file: &str, map: &HashMap<String, usize>) -> Option<usize> {
    let name = file.rsplit('/').next().unwrap_or(file);
    if let Some(v) = map.get(file) {
        return Some(*v);
    }
    if let Some(v) = map.get(name) {
        return Some(*v);
    }
    map.iter()
        .find(|(k, _)| {
            let kn = k.rsplit('/').next().unwrap_or(k);
            kn == name
        })
        .map(|(_, v)| *v)
}

pub(crate) fn load_epub_bytes(
    bytes: &std::sync::Arc<Vec<u8>>,
    zip_path: &str,
) -> Option<Vec<u8>> {
    let mut zip = ZipArchive::new(ZipSource(bytes.clone(), 0)).ok()?;
    let mut f = zip.by_name(zip_path).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

pub(crate) fn html_to_blocks(data: &[u8], chapter_file: &str) -> Vec<Block> {
    let text = String::from_utf8_lossy(data).to_string();
    let mut reader = Reader::from_str(&text);
    let mut blocks = Vec::new();

    let mut cur_inlines: Vec<Inline> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut heading_level: Option<u8> = None;
    let mut heading_anchor: Option<String> = None;
    let mut in_body = false;
    let mut skip_depth = 0usize;
    let mut list_stack: Vec<bool> = Vec::new();
    let mut quote_depth = 0usize;
    let mut pending_anchor: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                let id = attr(&e, "id");
                if name == "body" {
                    in_body = true;
                    continue;
                }
                if matches!(name.as_str(), "style" | "script" | "head") {
                    skip_depth += 1;
                    continue;
                }
                if skip_depth > 0 || !in_body {
                    continue;
                }
                match name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        flush_para(&mut blocks, &mut cur_inlines);
                        heading_level = Some(name.as_bytes()[1] - b'0');
                        heading_anchor = id.clone();
                        style_stack.clear();
                    }
                    "p" | "div" | "section" | "article" => {
                        flush_para(&mut blocks, &mut cur_inlines);
                        if pending_anchor.is_none() {
                            pending_anchor = id.clone();
                        }
                        style_stack.clear();
                    }
                    "li" => {
                        flush_para(&mut blocks, &mut cur_inlines);
                        style_stack.clear();
                    }
                    "ul" => list_stack.push(true),
                    "ol" => list_stack.push(false),
                    "blockquote" => {
                        flush_para(&mut blocks, &mut cur_inlines);
                        style_stack.clear();
                        quote_depth += 1;
                    }
                    "strong" | "b" => style_stack.push(Style::Bold),
                    "em" | "i" => style_stack.push(Style::Italic),
                    "code" | "tt" | "pre" => style_stack.push(Style::Code),
                    "a" => {
                        if let Some(id) = id {
                            pending_anchor = Some(id);
                        }
                    }
                    "br" => push_inline(&mut cur_inlines, &style_stack, "\n"),
                    "img" => {
                        if let Some(src) = attr(&e, "src") {
                            flush_para(&mut blocks, &mut cur_inlines);
                            let full = resolve_opf_path(chapter_file, &src);
                            blocks.push(Block::Image {
                                src: full,
                                anchor: std::mem::take(&mut pending_anchor),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                if !in_body || skip_depth > 0 {
                    continue;
                }
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "br" => push_inline(&mut cur_inlines, &style_stack, "\n"),
                    "hr" => {
                        flush_para(&mut blocks, &mut cur_inlines);
                        blocks.push(Block::Divider);
                    }
                    "img" => {
                        if let Some(src) = attr(&e, "src") {
                            flush_para(&mut blocks, &mut cur_inlines);
                            let full = resolve_opf_path(chapter_file, &src);
                            blocks.push(Block::Image {
                                src: full,
                                anchor: std::mem::take(&mut pending_anchor),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if skip_depth > 0 || !in_body {
                    continue;
                }
                let raw = quick_xml::escape::unescape(t.as_ref()).unwrap_or_default().to_string();
                let cleaned = collapse_ws(&raw);
                if !cleaned.is_empty() {
                    push_inline(&mut cur_inlines, &style_stack, &cleaned);
                }
            }
            Ok(Event::CData(t)) => {
                if skip_depth > 0 || !in_body {
                    continue;
                }
                let cleaned = collapse_ws(t.as_ref());
                if !cleaned.is_empty() {
                    push_inline(&mut cur_inlines, &style_stack, &cleaned);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                if matches!(name.as_str(), "style" | "script" | "head") {
                    skip_depth = skip_depth.saturating_sub(1);
                    continue;
                }
                if skip_depth > 0 {
                    continue;
                }
                match name.as_str() {
                    "body" => in_body = false,
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        if let Some(lvl) = heading_level.take() {
                            let inlines = std::mem::take(&mut cur_inlines);
                            if !inlines.is_empty() {
                                let anchor = heading_anchor.take().or_else(|| pending_anchor.take());
                                blocks.push(Block::Heading {
                                    level: lvl,
                                    inlines,
                                    anchor,
                                });
                            } else {
                                heading_anchor.take();
                            }
                        }
                    }
                    "p" | "div" | "section" | "article" => {
                        if quote_depth == 0 {
                            let mut inlines = std::mem::take(&mut cur_inlines);
                            if !inlines.is_empty() {
                                if let Some(anchor) = pending_anchor.take() {
                                    if let Some(first) = inlines.first_mut() {
                                        first.anchor = Some(anchor);
                                    }
                                }
                                blocks.push(Block::Para { inlines });
                            }
                        }
                    }
                    "li" => {
                        flush_para(&mut blocks, &mut cur_inlines);
                        let bullet = *list_stack.last().unwrap_or(&true);
                        if let Some(last) = blocks.last_mut() {
                            if let Block::Para { inlines } = last {
                                *last = Block::ListItem {
                                    inlines: std::mem::take(inlines),
                                    bullet,
                                };
                            }
                        }
                    }
                    "ul" | "ol" => {
                        list_stack.pop();
                    }
                    "blockquote" => {
                        quote_depth = quote_depth.saturating_sub(1);
                        let inlines = std::mem::take(&mut cur_inlines);
                        if !inlines.is_empty() {
                            blocks.push(Block::Quote { inlines });
                        }
                    }
                    "strong" | "b" | "em" | "i" | "code" | "tt" | "pre" => {
                        style_stack.pop();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    flush_para(&mut blocks, &mut cur_inlines);
    blocks
}

fn flush_para(blocks: &mut Vec<Block>, cur: &mut Vec<Inline>) {
    if cur.is_empty() {
        return;
    }
    let inlines = std::mem::take(cur);
    blocks.push(Block::Para { inlines });
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

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            continue;
        }
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    out
}

fn attr(e: &quick_xml::events::BytesStart<'_>, key: &str) -> Option<String> {
    for a in e.attributes() {
        if let Ok(a) = a {
            if a.key.local_name().as_ref() == key {
                return Some(
                    a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .unwrap_or_default()
                        .to_string(),
                );
            }
        }
    }
    None
}
