use eframe::egui::{
    self, Align, Color32, FontId, Frame, Key, Label, Layout, RichText, ScrollArea, Vec2,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;
use zip::ZipArchive;

mod hilite;
mod loaders;
mod model;
mod storage;
mod style;

use loaders::load as load_doc;
use model::{Block, DocData, Document, Inline, Style, TocEntry};
use storage::{Library, Note, Settings};
use style::FontManager;

const APP_TITLE: &str = "CrazyReader";
const ICON_RGBA: &[u8] = include_bytes!("../assets/icon_64.rgba");

struct BookHandle {
    doc: Rc<Document>,
    #[allow(dead_code)]
    zip: Option<ZipArchive<Cursor<Vec<u8>>>>,
}

struct AppState {
    lib: Library,
    config_file: PathBuf,
    books: Vec<BookHandle>,
    cur: usize,
    page: usize,
    show_toc: bool,
    show_notes: bool,
    note_input: String,
    font_size: f32,
    dark: bool,
    double_page: bool,
    bg: Option<u32>,
    fonts: FontManager,
    font_key: String,
    /// directory shown in the left panel, if the user opened one
    folder: Option<PathBuf>,
    folder_cache: HashMap<PathBuf, Vec<FsEntry>>,
    /// expanded branches of the folder tree
    folder_open: HashSet<PathBuf>,
    status: String,
    error: Option<String>,
    open_paths: Vec<PathBuf>,
    /// (book, page, block) -> measured height
    block_h: HashMap<(usize, usize, usize), f32>,
    /// (book, unit-base-page) -> measured height of one rendered unit
    page_h: HashMap<(usize, usize), f32>,
    tex_pdf: HashMap<(usize, usize), egui::TextureHandle>,
    tex_img: HashMap<String, egui::TextureHandle>,
    pdf_failed: HashSet<(usize, usize)>,
    pdf_queued: Vec<(usize, usize)>,
    /// absolute target scroll offset to apply, if any
    pending_target: Option<f32>,
    /// current live scroll offset of reading area
    live_offset: f32,
    /// scroll offset observed on the previous frame (for detecting user drag direction)
    prev_offset: f32,
    /// true while a programmatic jump is still settling (suppresses flow auto-advance)
    programmatic: bool,
    last_save: Instant,
    zoom: f32,
    /// last text the reader selected in the reading area
    selection: Option<String>,
    /// top-left corner of the floating selection toolbar, `None` while hidden
    sel_tools_pos: Option<egui::Pos2>,
    /// excerpt the note being written refers to
    note_quote: Option<String>,
    /// filled by the [`SelectionProbe`] plugin, which is the only place the
    /// settled selection can still be read from
    sel_sink: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

struct ReaderApp {
    st: RefCell<AppState>,
}

impl ReaderApp {
    fn new(ctx: &egui::Context) -> Self {
        let (lib, config_file) = Library::load();
        let settings = lib.settings.clone();
        let mut fonts = FontManager::new();
        let font_key = fonts
            .apply(ctx, &settings.font)
            .unwrap_or_else(|| settings.font.clone());
        let mut st = AppState {
            lib,
            config_file,
            books: Vec::new(),
            cur: 0,
            page: 0,
            show_toc: true,
            show_notes: false,
            note_input: String::new(),
            font_size: settings.font_size,
            dark: settings.dark,
            double_page: settings.double_page,
            bg: settings.bg,
            fonts,
            font_key,
            folder: settings
                .folder
                .as_ref()
                .map(PathBuf::from)
                .filter(|p| p.is_dir()),
            folder_cache: HashMap::new(),
            folder_open: settings
                .folder_open
                .iter()
                .map(PathBuf::from)
                .collect(),
            status: String::new(),
            error: None,
            open_paths: Vec::new(),
            block_h: HashMap::new(),
            page_h: HashMap::new(),
            tex_pdf: HashMap::new(),
            tex_img: HashMap::new(),
            pdf_failed: HashSet::new(),
            pdf_queued: Vec::new(),
            pending_target: None,
            live_offset: 0.0,
            prev_offset: 0.0,
            programmatic: false,
            last_save: Instant::now(),
            zoom: 1.0,
            selection: None,
            sel_tools_pos: None,
            note_quote: None,
            sel_sink: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        if st.folder_open.is_empty() {
            // A freshly restored folder starts with its root branch open.
            if let Some(f) = st.folder.clone() {
                st.folder_open.insert(f);
            }
        }
        let last = st
            .lib
            .books
            .values()
            .max_by(|a, b| a.last_opened.cmp(&b.last_opened))
            .map(|e| e.path.clone());
        if let Some(p) = last {
            let path = PathBuf::from(&p);
            if path.exists() {
                open_path(&mut st, &path);
            }
        }
        let app = Self {
            st: RefCell::new(st),
        };
        ctx.add_plugin(SelectionProbe {
            sink: app.st.borrow().sel_sink.clone(),
        });
        app
    }
}

fn open_path(st: &mut AppState, path: &std::path::Path) {
    match load_doc(path) {
        Ok(doc) => {
            let key = path.to_string_lossy().to_string();
            let (restore_page, restore_scroll) = st
                .lib
                .books
                .get(&key)
                .map(|e| (e.page, e.scroll))
                .unwrap_or((0, 0.0));
            let zip: Option<ZipArchive<Cursor<Vec<u8>>>> = None;
            if let Some(i) = st.books.iter().position(|b| b.doc.path == path) {
                st.books[i] = BookHandle {
                    doc: Rc::new(doc),
                    zip,
                };
                st.cur = i;
            } else {
                st.books.push(BookHandle {
                    doc: Rc::new(doc),
                    zip,
                });
                st.cur = st.books.len() - 1;
            }
            st.page = restore_page;
            st.pending_target = Some(restore_scroll);
            st.prev_offset = restore_scroll;
            st.programmatic = true;
            st.error = None;
            st.selection = None;
            st.status = format!(
                "已打开 {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            ensure_library_entry(st);
            save_now(st);
        }
        Err(e) => {
            st.error = Some(e);
        }
    }
}

fn switch_book(st: &mut AppState, i: usize) {
    if i >= st.books.len() || i == st.cur {
        return;
    }
    save_now(st);
    st.cur = i;
    st.block_h.retain(|k, _| k.0 != i);
    st.selection = None;
    st.sel_tools_pos = None;
    st.note_quote = None;
    let key = st.books[i].doc.path.to_string_lossy().to_string();
    let (page, scroll) = st
        .lib
        .books
        .get(&key)
        .map(|e| (e.page, e.scroll))
        .unwrap_or((0, 0.0));
    st.page = page;
    st.pending_target = Some(scroll);
    st.prev_offset = scroll;
    st.programmatic = true;
}

/// Close a tab. Cached measurements are keyed by book index, so they are
/// dropped rather than shifted down.
fn close_book(st: &mut AppState, i: usize) {
    if i >= st.books.len() {
        return;
    }
    save_now(st);
    st.books.remove(i);
    st.selection = None;
    st.sel_tools_pos = None;
    st.note_quote = None;
    st.block_h.clear();
    st.page_h.clear();
    st.tex_pdf.clear();
    st.pdf_failed.clear();
    st.pdf_queued.clear();
    if st.books.is_empty() {
        st.cur = 0;
        st.page = 0;
        st.live_offset = 0.0;
        st.prev_offset = 0.0;
        st.pending_target = None;
        st.status = String::new();
        save_now(st);
        return;
    }
    if i < st.cur {
        st.cur -= 1;
    } else {
        st.cur = st.cur.min(st.books.len() - 1);
    }
    let key = st.books[st.cur].doc.path.to_string_lossy().to_string();
    let (page, scroll) = st
        .lib
        .books
        .get(&key)
        .map(|e| (e.page, e.scroll))
        .unwrap_or((0, 0.0));
    st.page = page;
    st.pending_target = Some(scroll);
    st.prev_offset = scroll;
    st.programmatic = true;
    save_now(st);
}

fn reset_page_mode(st: &mut AppState) {
    let unit = if st.double_page { 2 } else { 1 };
    st.page = (st.page / unit) * unit;
    st.block_h.retain(|k, _| k.0 != st.cur);
    st.page_h.retain(|k, _| k.0 != st.cur);
    st.pending_target = Some(0.0);
    st.prev_offset = 0.0;
    st.programmatic = true;
}

fn ensure_library_entry(st: &mut AppState) {
    if st.books.is_empty() {
        return;
    }
    let doc = &st.books[st.cur.min(st.books.len() - 1)].doc;
    let key = doc.path.to_string_lossy().to_string();
    let entry = st.lib.books.entry(key.clone()).or_insert_with(|| storage::BookEntry {
        path: key.clone(),
        title: doc.title.clone(),
        format: doc.format.label().to_string(),
        last_opened: now_str(),
        page: 0,
        scroll: 0.0,
        notes: Vec::new(),
        marks: Vec::new(),
    });
    entry.title = doc.title.clone();
    entry.format = doc.format.label().to_string();
    entry.last_opened = now_str();
}

fn save_now(st: &mut AppState) {
    if let Some(b) = st.books.get(st.cur) {
        let key = b.doc.path.to_string_lossy().to_string();
        if let Some(e) = st.lib.books.get_mut(&key) {
            e.page = st.page;
            e.scroll = st.live_offset;
        }
    }
    st.lib.settings = Settings {
        font_size: st.font_size,
        dark: st.dark,
        double_page: st.double_page,
        bg: st.bg,
        font: st.font_key.clone(),
        folder: st
            .folder
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        folder_open: st
            .folder_open
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    };
    Library::save(&st.lib, &st.config_file);
    st.last_save = Instant::now();
}

/// egui only publishes a settled label selection as a platform command, and the
/// backend takes the output as soon as the pass ends, so a plugin output hook is
/// the only place left where it can still be read.
struct SelectionProbe {
    sink: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl egui::Plugin for SelectionProbe {
    fn debug_name(&self) -> &'static str {
        "crazy-reader-selection"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let text = output.platform_output.commands.iter().rev().find_map(|c| {
            match c {
                egui::OutputCommand::TextSelectionSettled(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
            }
        });
        if let Some(text) = text {
            if let Ok(mut g) = self.sink.lock() {
                *g = Some(text);
            }
            // Nothing requests a frame once the drag has ended, so without this
            // the 「标记」 button would keep its pre-selection state.
            ctx.request_repaint();
        }
    }
}

impl eframe::App for ReaderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let st = &mut *self.st.borrow_mut();
        if let Ok(mut g) = st.sel_sink.lock() {
            if g.is_some() {
                st.selection = g.take();
                if let Some(sel) = &st.selection {
                    st.status = format!(
                        "已选中 {} 字 · 用悬浮工具窗操作",
                        sel.chars().count()
                    );
                    // Every sink write is one real drag-end, so the toolbar always
                    // follows the selection to wherever the drag finished.
                    let p = ctx
                        .input(|i| i.pointer.interact_pos())
                        .unwrap_or_else(|| ctx.viewport_rect().center());
                    st.sel_tools_pos = Some(clamp_tools(p + egui::vec2(6.0, 16.0), &ctx));
                }
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::M)) && !st.books.is_empty() {
            apply_mark(st);
        }
        if !st.open_paths.is_empty() {
            let paths = std::mem::take(&mut st.open_paths);
            for p in paths {
                open_path(st, &p);
            }
        }
        if st.last_save.elapsed().as_secs() > 20 {
            save_now(st);
        }
        // PDF keyboard paging
        let page_down =
            ctx.input(|i| i.key_pressed(Key::PageDown) || i.key_pressed(Key::Space));
        let page_up = ctx.input(|i| i.key_pressed(Key::PageUp));
        let next = ctx.input(|i| i.key_pressed(Key::ArrowRight));
        let prev = ctx.input(|i| i.key_pressed(Key::ArrowLeft));
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::O)) {
            let files = rfd::FileDialog::new()
                .add_filter("电子书", &["epub", "md", "markdown", "pdf", "txt", "text"])
                .pick_files();
            if let Some(files) = files {
                st.open_paths = files;
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::D)) {
            pick_folder(st);
        }
        if page_down || next || page_up || prev {
            let dir = if page_down || next { 1.0 } else { -1.0 };
            let view_h = ctx
                .input(|i| i.viewport().inner_rect.map(|r| r.height()).unwrap_or(700.0))
                - 160.0;
            if dir > 0.0 {
                st.pending_target = Some(st.live_offset + view_h);
            } else {
                st.pending_target = Some((st.live_offset - view_h).max(0.0));
            }
        }
        // Render at most one queued PDF page per frame to keep UI responsive.
        if let Some((book_idx, page)) = st.pdf_queued.first().copied() {
            if st.tex_pdf.contains_key(&(book_idx, page)) {
                st.pdf_queued.remove(0);
                return;
            }
            let bytes: Option<Vec<u8>> = st
                .books
                .get(book_idx)
                .filter(|b| matches!(b.doc.data, DocData::Pdf(_)))
                .map(|b| match &b.doc.data {
                    DocData::Pdf(p) => p.bytes.clone(),
                    _ => Vec::new(),
                });
            if let Some(bytes) = bytes {
                let result = loaders::pdf::render_page(&bytes, page, 2.0);
                st.pdf_queued.remove(0);
                match result {
                    Ok(img) => {
                        let ci = egui::ColorImage::from_rgba_unmultiplied(
                            [img.width as usize, img.height as usize],
                            &img.rgba,
                        );
                        let tex = ctx.load_texture(
                            format!("pdf-{book_idx}-{page}"),
                            ci,
                            egui::TextureOptions::LINEAR,
                        );
                        st.tex_pdf.insert((book_idx, page), tex);
                        ctx.request_repaint();
                    }
                    Err(_) => {
                        st.pdf_failed.insert((book_idx, page));
                    }
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let st = &mut *self.st.borrow_mut();
        let ctx = ui.ctx().clone();
        build_style(&ctx, st.dark);
        if let Some(k) = st.fonts.apply(&ctx, &st.font_key) {
            if k != st.font_key {
                st.font_key = k;
            }
        }
        let has_books = !st.books.is_empty();

        // ---- top bar ----
        let mut open_clicked = false;
        let mut open_dir_clicked = false;
        let mut tab_click: Option<usize> = None;
        let mut tab_close: Option<usize> = None;
        let cur = st.cur;
        egui::Panel::top("topbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("打开文件 (Ctrl+O)").strong()).fill(ui.style().visuals.widgets.noninteractive.bg_fill)).clicked() {
                    open_clicked = true;
                }
                if ui.add(egui::Button::new(RichText::new("打开目录").strong()).fill(ui.style().visuals.widgets.noninteractive.bg_fill)).clicked() {
                    open_dir_clicked = true;
                }
                ui.separator();
                if ui.selectable_label(st.show_toc, "侧栏").clicked() {
                    st.show_toc = !st.show_toc;
                }
                if has_books && ui.selectable_label(st.show_notes, "笔记").clicked() {
                    st.show_notes = !st.show_notes;
                }
                if has_books {
                    let tools_open = st.sel_tools_pos.is_some();
                    if ui
                        .selectable_label(tools_open, "工具窗")
                        .on_hover_text("选中文字的悬浮窗：标记 / 摘录 / 笔记 / 复制")
                        .clicked()
                    {
                        st.sel_tools_pos = if tools_open {
                            None
                        } else {
                            let p = ctx
                                .input(|i| i.pointer.interact_pos())
                                .unwrap_or_else(|| ctx.viewport_rect().center());
                            Some(clamp_tools(p, &ctx))
                        };
                    }
                }
                if !st.status.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(&st.status).weak().size(12.5));
                }
                if has_books {
                    ui.separator();
                    for (i, b) in st.books.iter().enumerate() {
                        let title = shorten(&b.doc.title, 18);
                        if i == cur {
                            ui.colored_label(
                                Color32::from_rgb(0x39, 0x55, 0xCF),
                                RichText::new(format!("■ {title}")).strong(),
                            );
                        } else if ui.button(format!("□ {title}")).clicked() {
                            tab_click = Some(i);
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("关闭此页签")
                            .clicked()
                        {
                            tab_close = Some(i);
                        }
                    }
                }
            });
        });
        if open_clicked {
            let files = rfd::FileDialog::new()
                .add_filter("电子书", &["epub", "md", "markdown", "pdf", "txt", "text"])
                .pick_files();
            if let Some(files) = files {
                for f in files {
                    st.open_paths.push(f);
                }
            }
        }
        if open_dir_clicked {
            pick_folder(st);
        }
        if let Some(i) = tab_click {
            switch_book(st, i);
        }
        if let Some(i) = tab_close {
            close_book(st, i);
        }

        // ---- left panel: folder tree + opened books + TOC ----
        let folder = st.folder.clone();
        let mut tree_open = std::mem::take(&mut st.folder_open);
        let mut tree_cache = std::mem::take(&mut st.folder_cache);
        let doc = has_books.then(|| st.books[st.cur].doc.clone());

        if !has_books {
            let mut ev = FolderEv::default();
            if st.show_toc {
                egui::Panel::left("browse")
                    .resizable(true)
                    .default_size(260.0)
                    .min_size(180.0)
                    .max_size(480.0)
                    .show(ui, |ui| {
                        folder_section(ui, folder.as_ref(), &mut tree_cache, &mut tree_open, &mut ev);
                        ui.label(RichText::new("还没有打开的书").weak().size(12.5));
                    });
            }
            st.folder_open = tree_open;
            st.folder_cache = tree_cache;
            apply_folder_events(st, ev);
            let bg = st.bg;
            egui::CentralPanel::default()
                .frame(reading_frame(ui, bg))
                .show(ui, |ui| {
                    apply_reading_colors(ui, bg);
                    empty_view(st, ui);
                });
            return;
        }
        let doc = doc.unwrap();

        if st.show_toc {
            let mut jump: Option<TocEntry> = None;
            let mut ev = FolderEv::default();
            let page = st.page;
            egui::Panel::left("toc")
                .resizable(true)
                .default_size(250.0)
                .min_size(160.0)
                .max_size(480.0)
                .show(ui, |ui| {
                    folder_section(ui, folder.as_ref(), &mut tree_cache, &mut tree_open, &mut ev);
                    ui.separator();
                    ui.strong("章节目录");
                    ui.separator();
                    ScrollArea::vertical()
                        .id_salt("toc")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if doc.toc.is_empty() {
                                ui.label(RichText::new("（此文档没有目录）").weak());
                            }
                            for e in doc.toc.iter() {
                                let indent = (e.depth as f32) * 12.0;
                                ui.add_space(1.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(indent);
                                    let is_cur = e.page == page;
                                    let rt = RichText::new(shorten(&e.title, 30)).size(13.5);
                                    let rt = if is_cur {
                                        rt.color(Color32::from_rgb(0x39, 0x55, 0xCF)).strong()
                                    } else {
                                        rt
                                    };
                                    if list_row(ui, rt, 22.0) {
                                        jump = Some(e.clone());
                                    }
                                });
                            }
                        });
                });
            st.folder_open = tree_open;
            st.folder_cache = tree_cache;
            apply_folder_events(st, ev);
            if let Some(e) = jump {
                st.page = e.page.min(doc.pages.len().saturating_sub(1));
                let target = heading_offset(st, doc.as_ref(), st.cur, st.page, &e);
                st.pending_target = Some(target);
                st.prev_offset = target;
                st.programmatic = true;
            }
        } else {
            st.folder_open = tree_open;
            st.folder_cache = tree_cache;
        }

        // ---- right panel: notes ----
        if st.show_notes {
            let key = doc.path.to_string_lossy().to_string();
            let mut add_clicked = false;
            let mut clear_clicked = false;
            let mut delete_note: Option<u64> = None;
            let mut jump_note: Option<usize> = None;
            let page = st.page;
            let total = doc.pages.len();
            let notes: Vec<Note> = st
                .lib
                .books
                .get(&key)
                .map(|e| e.notes.clone())
                .unwrap_or_default();
            // An excerpt is a note that carries only the quoted source text.
            let excerpts: Vec<Note> = notes.iter().filter(|n| n.text.is_empty()).cloned().collect();
            let note_list: Vec<Note> = notes.iter().filter(|n| !n.text.is_empty()).cloned().collect();
            let marks: Vec<String> = st
                .lib
                .books
                .get(&key)
                .map(|e| e.marks.clone())
                .unwrap_or_default();
            let mut delete_mark: Option<usize> = None;
            let mut cancel_quote = false;
            let mark_bg = mark_color(st.dark);
            egui::Panel::right("notes")
                .resizable(true)
                .default_size(280.0)
                .min_size(200.0)
                .max_size(480.0)
                .show(ui, |ui| {
                    ui.strong("笔记");
                    ui.separator();
                    ui.label(RichText::new(format!("当前：第 {} / {total} 页", page + 1)).size(12.5).weak());
                    if let Some(q) = st.note_quote.clone() {
                        Frame::new()
                            .fill(ui.style().visuals.faint_bg_color)
                            .corner_radius(6.0)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("引用").size(11.0).weak());
                                    ui.label(RichText::new(shorten(&q, 60)).size(12.0).italics());
                                    if ui.small_button("取消").clicked() {
                                        cancel_quote = true;
                                    }
                                });
                            });
                        ui.add_space(4.0);
                    }
                    ui.add(
                        egui::TextEdit::multiline(&mut st.note_input)
                            .hint_text("输入笔记内容…")
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("添加笔记").strong())).clicked() {
                            add_clicked = true;
                        }
                        if ui.button("清空").clicked() {
                            clear_clicked = true;
                        }
                    });
                    ui.separator();
                    // Everything below the editor shares one scroll area. Three stacked
                    // areas overflow a short window: once the last one was left with zero
                    // height, egui laid its content out at unbounded width and the whole
                    // panel slid off the right edge of the screen.
                    ScrollArea::vertical()
                        .id_salt("notes_body")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if note_list.is_empty() {
                                ui.label(RichText::new("（暂无笔记）").weak());
                            }
                            for n in note_list.iter().rev() {
                                Frame::new()
                                    .fill(ui.style().visuals.faint_bg_color)
                                    .corner_radius(6.0)
                                    .inner_margin(10.0)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(&n.text).size(13.5));
                                        ui.label(
                                            RichText::new(format!(
                                                "第 {} 页 · {}",
                                                n.page + 1,
                                                n.created
                                            ))
                                            .size(11.0)
                                            .weak(),
                                        );
                                        ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                                            if ui.small_button("删除").clicked() {
                                                delete_note = Some(n.id);
                                            }
                                            if ui.small_button("跳转").clicked() {
                                                jump_note = Some(n.page.min(total.saturating_sub(1)));
                                            }
                                        });
                                    });
                                ui.add_space(6.0);
                            }
                            ui.separator();
                            ui.strong(format!("摘录 ({})", excerpts.len()));
                            ui.label(
                                RichText::new("用悬浮工具窗的「摘录」保存选中的原文")
                                    .size(11.5)
                                    .weak(),
                            );
                            ui.add_space(4.0);
                            if excerpts.is_empty() {
                                ui.label(RichText::new("（暂无摘录）").weak());
                            }
                            for n in excerpts.iter().rev() {
                                let Some(q) = n.quote.as_deref() else { continue };
                                Frame::new()
                                    .fill(ui.style().visuals.faint_bg_color)
                                    .corner_radius(6.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(q).size(12.5).italics());
                                        ui.label(
                                            RichText::new(format!(
                                                "第 {} 页 · {}",
                                                n.page + 1,
                                                n.created
                                            ))
                                            .size(11.0)
                                            .weak(),
                                        );
                                        ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                                            if ui.small_button("删除").clicked() {
                                                delete_note = Some(n.id);
                                            }
                                            if ui.small_button("跳转").clicked() {
                                                jump_note = Some(n.page.min(total.saturating_sub(1)));
                                            }
                                        });
                                    });
                                ui.add_space(5.0);
                            }
                            ui.separator();
                            ui.strong("标记");
                            ui.label(
                                RichText::new("选中文字后，用悬浮工具窗或阅读区底部的「标记」")
                                    .size(11.5)
                                    .weak(),
                            );
                            ui.add_space(4.0);
                            if marks.is_empty() {
                                ui.label(RichText::new("（暂无标记）").weak());
                            }
                            for (i, m) in marks.iter().enumerate().rev() {
                                Frame::new()
                                    .fill(mark_bg)
                                    .corner_radius(4.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(m).size(12.5));
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Max),
                                            |ui| {
                                                if ui.small_button("删除").clicked() {
                                                    delete_mark = Some(i);
                                                }
                                            },
                                        );
                                    });
                                ui.add_space(5.0);
                            }
                        });
                });
            if add_clicked {
                let text = st.note_input.trim().to_string();
                if !text.is_empty() {
                    let note = Note {
                        id: now_millis(),
                        page,
                        anchor: None,
                        quote: st.note_quote.take(),
                        text,
                        created: now_str(),
                    };
                    if let Some(e) = st.lib.books.get_mut(&key) {
                        e.notes.push(note);
                    }
                    st.note_input.clear();
                    st.status = "笔记已保存".to_string();
                    save_now(st);
                }
            }
            if clear_clicked {
                st.note_input.clear();
            }
            if cancel_quote {
                st.note_quote = None;
            }
            if let Some(id) = delete_note {
                if let Some(e) = st.lib.books.get_mut(&key) {
                    e.notes.retain(|n| n.id != id);
                }
                save_now(st);
            }
            if let Some(i) = delete_mark {
                if let Some(e) = st.lib.books.get_mut(&key) {
                    e.marks.remove(i);
                }
                save_now(st);
            }
            if let Some(p) = jump_note {
                st.page = p;
                st.pending_target = Some(0.0);
                st.prev_offset = 0.0;
                st.programmatic = true;
            }
        }

        // ---- bottom bar ----
        let mut font_minus = false;
        let mut font_plus = false;
        let mut theme_toggle = false;
        let mut set_single = false;
        let mut set_double = false;
        let mut zoom_minus = false;
        let mut zoom_plus = false;
        let st_font = st.font_size;
        let st_dark = st.dark;
        let st_double = st.double_page;
        let st_zoom = st.zoom;
        let page_now = st.page;
        let total = doc.pages.len();
        let font_items: Vec<(String, String)> = st.fonts.available.clone();
        let font_key_now = st.font_key.clone();
        let bg_now = st.bg;
        let font_label = font_items
            .iter()
            .find(|(k, _)| k == &font_key_now)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| "系统字体".to_string());
        let bg_label = bg_label(bg_now);
        let mut new_font: Option<String> = None;
        let mut new_bg: Option<Option<u32>> = None;
        let sel_now = st.selection.clone();
        let mut mark_clicked = false;
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} · 第 {} / {total} 页 · {:.0}%",
                    doc.format.label(),
                    page_now + 1,
                    (page_now + 1) as f32 * 100.0 / total.max(1) as f32
                ));
                ui.separator();
                if ui.button("A−").clicked() {
                    font_minus = true;
                }
                ui.label(format!("{:.0}", st_font));
                if ui.button("A+").clicked() {
                    font_plus = true;
                }
                if matches!(doc.format, model::Format::Pdf) {
                    ui.separator();
                    if ui.button("−").clicked() {
                        zoom_minus = true;
                    }
                    ui.label(format!("缩放 {:.0}%", st_zoom * 100.0));
                    if ui.button("+").clicked() {
                        zoom_plus = true;
                    }
                }
                ui.separator();
                if ui.selectable_label(!st_double, "单页").clicked() {
                    set_single = true;
                }
                if ui.selectable_label(st_double, "双页").clicked() {
                    set_double = true;
                }
                ui.separator();
                if ui.button(if st_dark { "深色主题" } else { "浅色主题" }).clicked() {
                    theme_toggle = true;
                }
                ui.separator();
                if ui
                    .add_enabled(
                        sel_now.is_some(),
                        egui::Button::new(RichText::new("标记").weak()),
                    )
                    .on_hover_text("为选中的文字加上背景色（Ctrl+M）")
                    .clicked()
                {
                    mark_clicked = true;
                }
                ui.separator();
                ui.label(RichText::new("字体").size(12.5).weak());
                egui::ComboBox::from_id_salt("readfont")
                    .selected_text(font_label)
                    .show_ui(ui, |ui| {
                        if font_items.is_empty() {
                            ui.label(RichText::new("（未找到中文字体）").weak());
                        }
                        for (k, l) in &font_items {
                            if ui
                                .selectable_label(k == &font_key_now, RichText::new(l).size(13.5))
                                .clicked()
                            {
                                new_font = Some(k.clone());
                            }
                        }
                    });
                ui.label(RichText::new("背景").size(12.5).weak());
                egui::ComboBox::from_id_salt("readbg")
                    .selected_text(bg_label)
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for (name, v) in style::BG_PRESETS {
                            let sel = *v == bg_now;
                            let sw = swatch_color(*v);
                            if ui
                                .selectable_label(
                                    sel,
                                    RichText::new(format!("■ {name}")).color(sw).size(13.5),
                                )
                                .clicked()
                            {
                                new_bg = Some(*v);
                            }
                        }
                        ui.separator();
                        let mut custom = packed_color(bg_now.unwrap_or(0xF9F6EE));
                        if ui
                            .horizontal(|ui| {
                                ui.label(RichText::new("自定义").size(13.5));
                                egui::color_picker::color_picker_color32(
                                    ui,
                                    &mut custom,
                                    egui::color_picker::Alpha::Opaque,
                                )
                            })
                            .inner
                        {
                            new_bg = Some(Some(packed_of(custom)));
                        }
                    });
            });
        });
        if mark_clicked {
            apply_mark(st);
            ui.ctx().request_repaint();
        }
        if let Some(k) = new_font {
            if k != st.font_key {
                st.font_key = k;
                st.block_h.retain(|k, _| k.0 != st.cur);
                st.page_h.retain(|k, _| k.0 != st.cur);
                save_now(st);
                ui.ctx().request_repaint();
            }
        }
        if let Some(v) = new_bg {
            if v != st.bg {
                st.bg = v;
                save_now(st);
                ui.ctx().request_repaint();
            }
        }
        if font_minus {
            st.font_size = (st.font_size - 1.0).max(11.0);
            st.block_h.retain(|k, _| k.0 != st.cur);
        }
        if font_plus {
            st.font_size = (st.font_size + 1.0).min(34.0);
            st.block_h.retain(|k, _| k.0 != st.cur);
        }
        if zoom_minus {
            st.zoom = (st.zoom - 0.25).max(0.5);
        }
        if zoom_plus {
            st.zoom = (st.zoom + 0.25).min(3.0);
        }
        if theme_toggle {
            st.dark = !st.dark;
            save_now(st);
        }
        if set_single && st.double_page {
            st.double_page = false;
            reset_page_mode(st);
            save_now(st);
        }
        if set_double && !st.double_page {
            st.double_page = true;
            reset_page_mode(st);
            save_now(st);
        }

        // ---- central reading area ----
        let jump_target = st.pending_target.take();
        let cur = st.cur;
        let start = st.page;
        let unit = if st.double_page { 2 } else { 1 };
        let total = doc.pages.len();
        let bg = st.bg;
        let marks = current_marks(st);
        let mark_bg = mark_color(st.dark);
        let mut c = Ctx {
            st,
            doc: &doc,
            zoom: st_zoom,
            marks,
            mark_bg,
        };
        let mut area = ScrollArea::vertical()
            .id_salt(("read", c.st.cur))
            .auto_shrink([false, false]);
        if let Some(off) = jump_target {
            area = area.vertical_scroll_offset(off.max(0.0));
        }
        let out = egui::CentralPanel::default()
            .frame(reading_frame(ui, bg))
            .show(ui, |ui| {
                apply_reading_colors(ui, bg);
                area.show(ui, |ui| {
                    render_page(&mut c, ui);
                })
            });
        let out = out.inner;
        let offset_now = out.state.clamped_offset().y;
        // The offset requested before layout may have been clamped against a
        // stale content height; re-apply the corrected value next frame.
        let mut settled = true;
        if let Some(target) = jump_target {
            let max_scroll = (out.content_size.y - out.inner_rect.height()).max(0.0);
            let clamped = target.clamp(0.0, max_scroll);
            if (clamped - offset_now).abs() > 2.0 {
                c.st.pending_target = Some(clamped);
                c.st.programmatic = true;
                ui.ctx().request_repaint();
                settled = false;
            }
        }
        // Continuous flow: scroll past the first unit to enter the next chapter,
        // scroll back to the top to re-enter the previous one.
        let first_h = c.st.page_h.get(&(cur, start)).copied().unwrap_or(0.0);
        if jump_target.is_some() && settled {
            c.st.programmatic = false;
        }
        let down = offset_now > c.st.prev_offset + 0.5;
        let up = offset_now < c.st.prev_offset - 0.5;
        let can_flow = settled && !c.st.programmatic && jump_target.is_none();
        if can_flow && down && first_h > 0.0 && offset_now > first_h + 80.0 && start + unit < total {
            let new_off = (offset_now - first_h).max(0.0);
            c.st.page = start + unit;
            c.st.pending_target = Some(new_off);
            c.st.prev_offset = new_off;
            c.st.programmatic = true;
            ui.ctx().request_repaint();
        } else if can_flow && up && offset_now < 80.0 && start >= unit {
            let new_base = start - unit;
            let ph = c.st.page_h.get(&(cur, new_base)).copied().unwrap_or(0.0);
            c.st.page = new_base;
            c.st.pending_target = Some(if ph > 0.0 { ph } else { f32::MAX });
            c.st.prev_offset = if ph > 0.0 { ph } else { offset_now };
            c.st.programmatic = true;
            ui.ctx().request_repaint();
        } else {
            c.st.prev_offset = offset_now;
        }
        let st = c.st;
        st.live_offset = offset_now;
        if let Some(b) = st.books.get(st.cur) {
            let key = b.doc.path.to_string_lossy().to_string();
            if let Some(e) = st.lib.books.get_mut(&key) {
                e.page = st.page;
                e.scroll = st.live_offset;
            }
        }
        selection_tools(st, &ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let st = &mut *self.st.borrow_mut();
        save_now(st);
    }
}

struct Ctx<'a> {
    st: &'a mut AppState,
    doc: &'a Rc<Document>,
    zoom: f32,
    /// Excerpt strings the reader marked in the current book.
    marks: Vec<String>,
    mark_bg: Color32,
}

fn current_marks(st: &AppState) -> Vec<String> {
    let Some(b) = st.books.get(st.cur) else {
        return Vec::new();
    };
    let key = b.doc.path.to_string_lossy().to_string();
    st.lib
        .books
        .get(&key)
        .map(|e| e.marks.clone())
        .unwrap_or_default()
}

/// Keep the floating toolbar inside the window.
fn clamp_tools(pos: egui::Pos2, ctx: &egui::Context) -> egui::Pos2 {
    let r = ctx.viewport_rect();
    let (w, h) = (232.0, 62.0);
    egui::pos2(
        pos.x.clamp(r.min.x + 6.0, (r.max.x - w).max(r.min.x + 6.0)),
        pos.y.clamp(r.min.y + 6.0, (r.max.y - h).max(r.min.y + 6.0)),
    )
}

/// The floating selection toolbar: 标记 / 摘录 / 笔记 / 复制.
///
/// It opens where the drag ended, moves by its header and closes with the ×
/// button; the next selection brings it back. The glyph is U+00D7 — CJK
/// reading fonts have it but not U+2715, which renders as a box.
fn selection_tools(st: &mut AppState, ctx: &egui::Context) {
    let Some(pos) = st.sel_tools_pos else {
        return;
    };
    let sel = st.selection.clone().filter(|s| !s.trim().is_empty());
    let mut close = false;
    let mut act: Option<&str> = None;
    egui::Area::new(egui::Id::unique("sel_tools"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            let (fill, stroke) = (
                ui.visuals().window_fill,
                ui.visuals().window_stroke(),
            );
            Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(8.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let head = ui.add(
                            Label::new(
                                RichText::new(format!(
                                    "选中 · {}",
                                    shorten(sel.as_deref().unwrap_or("无"), 14)
                                ))
                                .size(11.5)
                                .weak(),
                            )
                            .sense(egui::Sense::drag()),
                        );
                        if head.dragged() {
                            st.sel_tools_pos = Some(clamp_tools(pos + head.drag_delta(), ctx));
                        }
                        head.on_hover_text("按住此处可拖动悬浮窗");
                        ui.separator();
                        if ui
                            .small_button("×")
                            .on_hover_text("关闭悬浮窗（重新选中文字会再次出现）")
                            .clicked()
                        {
                            close = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        for (label, key, tip) in [
                            ("标记", "mark", "为选中的文字加上背景色（Ctrl+M）"),
                            ("摘录", "quote", "把选中的原文存入摘录列表"),
                            ("笔记", "note", "针对选中的文字写笔记"),
                            ("复制", "copy", "复制选中的文字到剪贴板"),
                        ] {
                            let b = ui.add_enabled(
                                sel.is_some(),
                                egui::Button::new(RichText::new(label).size(13.0)),
                            );
                            if b.on_hover_text(tip).clicked() {
                                act = Some(key);
                            }
                        }
                    });
                });
        });

    if close {
        st.sel_tools_pos = None;
        ctx.request_repaint();
        return;
    }
    let Some(act) = act else { return };
    let Some(text) = sel else { return };
    let excerpt = text.trim().to_string();
    match act {
        "mark" => {
            apply_mark(st);
            st.sel_tools_pos = None;
        }
        "quote" => {
            ensure_library_entry(st);
            let Some(key) = st
                .books
                .get(st.cur)
                .map(|b| b.doc.path.to_string_lossy().to_string())
            else {
                return;
            };
            let Some(entry) = st.lib.books.get_mut(&key) else {
                return;
            };
            if entry.notes.iter().any(|n| n.quote.as_deref() == Some(excerpt.as_str())) {
                st.status = "这段文字已经摘录过了".to_string();
            } else {
                entry.notes.push(Note {
                    id: now_millis(),
                    page: st.page,
                    anchor: None,
                    quote: Some(excerpt),
                    text: String::new(),
                    created: now_str(),
                });
                st.status = format!(
                    "摘录已保存 · 共 {} 条",
                    entry.notes.iter().filter(|n| n.text.is_empty()).count()
                );
                save_now(st);
            }
        }
        "note" => {
            st.show_notes = true;
            st.note_quote = Some(excerpt);
            st.sel_tools_pos = None;
        }
        _ => {
            ctx.copy_text(excerpt);
            st.status = "已复制到剪贴板".to_string();
        }
    }
    // The toolbar runs after the panels have already been laid out this frame,
    // so nothing it changes would be visible without another pass.
    ctx.request_repaint();
}

/// Highlight colour used for reader marks; also painted on the mark list.
fn mark_color(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(0x6B, 0x5A, 0x1E)
    } else {
        Color32::from_rgb(0xFF, 0xE9, 0x8A)
    }
}

/// Turn the current text selection into a permanent mark on the open book.
fn apply_mark(st: &mut AppState) {
    let Some(raw) = st.selection.take() else {
        st.status = "请先在阅读区选中一段文字".to_string();
        return;
    };
    let sel = raw.trim().to_string();
    if sel.is_empty() {
        st.status = "请先在阅读区选中一段文字".to_string();
        return;
    }
    let Some(doc) = st.books.get(st.cur).map(|b| b.doc.clone()) else {
        return;
    };
    if !doc_has_text(&doc, &sel) {
        st.status = "标记失败：请在同一段落内选中文字".to_string();
        return;
    }
    let key = doc.path.to_string_lossy().to_string();
    let excerpt = shorten(&sel, 160);
    ensure_library_entry(st);
    let Some(entry) = st.lib.books.get_mut(&key) else {
        return;
    };
    if entry.marks.iter().any(|m| m == &excerpt) {
        st.status = "这段文字已经标记过了".to_string();
        return;
    }
    entry.marks.push(excerpt);
    st.status = "已标记".to_string();
    save_now(st);
}

fn heading_offset(st: &AppState, doc: &Document, cur: usize, page: usize, entry: &TocEntry) -> f32 {
    // Offset of the named heading within the page, using heights measured on
    // previous frames (falling back to a rough estimate for unseen blocks).
    let Some(p) = doc.pages.get(page) else {
        return 0.0;
    };
    let mut y = 0.0f32;
    for (bi, b) in p.blocks.iter().enumerate() {
        if matches!(b, Block::Heading { .. }) && b.plain_text() == entry.title {
            return y;
        }
        y += st
            .block_h
            .get(&(cur, page, bi))
            .copied()
            .unwrap_or(24.0 + 18.0 * (b.plain_text().chars().count() as f32 / 60.0));
    }
    0.0
}

#[derive(Clone)]
struct FsEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

#[derive(Default)]
struct FolderEv {
    refresh: bool,
    /// a branch was expanded or collapsed
    toggled: bool,
    open_file: Option<PathBuf>,
}

fn is_book_ext(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            matches!(e.as_str(), "epub" | "md" | "markdown" | "pdf" | "txt" | "text")
        })
        .unwrap_or(false)
}

fn list_books(dir: &Path) -> Vec<FsEntry> {
    let mut out: Vec<FsEntry> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        return None;
                    }
                    let path = e.path();
                    let is_dir = path.is_dir();
                    if is_dir || is_book_ext(&path) {
                        Some(FsEntry { name, path, is_dir })
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out.truncate(500);
    out
}

fn pick_folder(st: &mut AppState) {
    let start = st.folder.clone();
    let mut dlg = rfd::FileDialog::new();
    if let Some(s) = start {
        dlg = dlg.set_directory(s);
    }
    if let Some(dir) = dlg.pick_folder() {
        st.folder = Some(dir.clone());
        st.folder_cache.clear();
        st.folder_open = [dir].into_iter().collect();
        st.show_toc = true;
        save_now(st);
    }
}

fn folder_children(
    cache: &mut HashMap<PathBuf, Vec<FsEntry>>,
    dir: &Path,
) -> Vec<FsEntry> {
    if let Some(v) = cache.get(dir) {
        return v.clone();
    }
    let v = list_books(dir);
    cache.insert(dir.to_path_buf(), v.clone());
    v
}

fn apply_folder_events(st: &mut AppState, ev: FolderEv) {
    let FolderEv {
        refresh,
        toggled,
        open_file,
    } = ev;
    let mut changed = false;
    if refresh {
        st.folder_cache.clear();
        changed = true;
    }
    if let Some(f) = open_file {
        st.open_paths.push(f);
    }
    if changed || toggled {
        save_now(st);
    }
}

/// Folder tree opened with "打开目录"; the listing lives in the left panel.
fn folder_section(
    ui: &mut egui::Ui,
    folder: Option<&PathBuf>,
    cache: &mut HashMap<PathBuf, Vec<FsEntry>>,
    open: &mut HashSet<PathBuf>,
    ev: &mut FolderEv,
) {
    ui.label(RichText::new("文件夹").strong());
    if let Some(root) = folder {
        ui.horizontal(|ui| {
            if ui.small_button("刷新").on_hover_text("重新读取目录").clicked() {
                ev.refresh = true;
            }
        });
        ui.add_space(2.0);
        ui.label(RichText::new(shorten(&root.display().to_string(), 40)).size(11.0).weak());
        ui.separator();
        ScrollArea::vertical()
            .id_salt("folder")
            .auto_shrink([false, true])
            .max_height((ui.available_height() * 0.45).clamp(180.0, 520.0))
            .show(ui, |ui| {
                tree_row(ui, root, 0, true, cache, open, ev);
                if open.contains(root) {
                    folder_tree(ui, root, 1, cache, open, ev);
                }
            });
        ui.add_space(8.0);
        ui.separator();
    }
}

/// Which type icon a folder-tree row shows.
enum Kind {
    Folder,
    /// `tag` is painted inside the page glyph, `desc` names the type.
    Doc {
        tag: &'static str,
        desc: &'static str,
        fill: Color32,
    },
}

impl Kind {
    fn label(&self) -> &'static str {
        match self {
            Kind::Folder => "文件夹",
            Kind::Doc { desc, .. } => desc,
        }
    }
}

fn kind_of(path: &Path, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let (tag, desc, rgb) = match ext.as_str() {
        "epub" => ("E", "epub 书籍", 0x3e7bfa),
        "md" | "markdown" => ("M", "Markdown 文档", 0x3f9a55),
        "pdf" => ("P", "PDF 文档", 0xc4453a),
        "txt" | "text" => ("T", "文本文件", 0x8a8a80),
        _ => ("F", "其他文件", 0x9a9a94),
    };
    Kind::Doc {
        tag,
        desc,
        fill: packed_color(rgb),
    }
}

fn shade(c: Color32, k: f32) -> Color32 {
    let a = c.to_srgba_unmultiplied();
    let m = |v: u8| (v as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(m(a[0]), m(a[1]), m(a[2]), a[3])
}

/// The type icon in front of a tree row.
///
/// Drawn from shapes instead of emoji: the fonts egui bundles only carry a
/// handful of pictographs, so the rest would render as tofu boxes.
/// Returns `true` when clicked, since the icon belongs to the row's hit area.
fn kind_icon(ui: &mut egui::Ui, kind: &Kind) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::click());
    let p = ui.painter_at(rect.shrink(1.0));
    let mid = egui::StrokeKind::Middle;
    match kind {
        Kind::Folder => {
            let fill = packed_color(0xd8b25c);
            let edge = egui::Stroke::new(1.0, shade(fill, 0.62));
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.min.x + 1.5, rect.min.y + 3.5),
                    egui::pos2(rect.min.x + 7.0, rect.min.y + 6.5),
                ),
                1,
                fill,
            );
            p.rect(
                egui::Rect::from_min_max(
                    egui::pos2(rect.min.x + 1.0, rect.min.y + 5.5),
                    egui::pos2(rect.max.x - 1.0, rect.max.y - 2.0),
                ),
                2,
                fill,
                edge,
                mid,
            );
        }
        Kind::Doc { tag, fill, .. } => {
            let (x0, x1) = (rect.min.x + 3.0, rect.max.x - 2.5);
            let (y0, y1) = (rect.min.y + 1.5, rect.max.y - 1.5);
            let fold = 4.0f32;
            p.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x0, y0),
                    egui::pos2(x1 - fold, y0),
                    egui::pos2(x1, y0 + fold),
                    egui::pos2(x1, y1),
                    egui::pos2(x0, y1),
                ],
                *fill,
                egui::Stroke::new(1.0, shade(*fill, 0.7)),
            ));
            p.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(x1 - fold, y0),
                    egui::pos2(x1 - fold, y0 + fold),
                    egui::pos2(x1, y0 + fold),
                ],
                egui::Stroke::new(1.0, Color32::from_white_alpha(150)),
            ));
            p.text(
                egui::pos2((x0 + x1) / 2.0, (y0 + fold + y1) / 2.0 + 1.0),
                egui::Align2::CENTER_CENTER,
                tag.to_string(),
                FontId::proportional(8.5),
                Color32::WHITE,
            );
        }
    }
    let clicked = resp.clicked();
    resp.on_hover_text(kind.label());
    clicked
}

/// One row of the folder tree. Returns nothing but records the click in `ev`.
fn tree_row(
    ui: &mut egui::Ui,
    path: &Path,
    depth: usize,
    is_dir: bool,
    cache: &mut HashMap<PathBuf, Vec<FsEntry>>,
    open: &mut HashSet<PathBuf>,
    ev: &mut FolderEv,
) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let expanded = is_dir && open.contains(path);
    let kind = kind_of(path, is_dir);
    let marker = if !is_dir {
        ""
    } else if expanded {
        "-"
    } else {
        "+"
    };
    let mut rt = RichText::new(shorten(&name, 30)).size(13.0);
    if is_dir {
        rt = rt.color(Color32::from_rgb(0x8A, 0x6D, 0x1F));
        if depth == 0 {
            rt = rt.strong();
        }
    }
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);
        // Fixed-width expand marker, so the icon column never shifts.
        ui.allocate_ui_with_layout(
            Vec2::new(11.0, 21.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                if !marker.is_empty() {
                    ui.label(RichText::new(marker).size(13.0).weak());
                }
            },
        );
        if kind_icon(ui, &kind) {
            clicked = true;
        }
        if list_row(ui, rt, 21.0) {
            clicked = true;
        }
    });
    if !clicked {
        return;
    }
    if is_dir {
        if !open.remove(path) {
            open.insert(path.to_path_buf());
        }
        ev.toggled = true;
        if !expanded {
            folder_children(cache, path);
        }
    } else {
        ev.open_file = Some(path.to_path_buf());
    }
}

fn folder_tree(
    ui: &mut egui::Ui,
    dir: &Path,
    depth: usize,
    cache: &mut HashMap<PathBuf, Vec<FsEntry>>,
    open: &mut HashSet<PathBuf>,
    ev: &mut FolderEv,
) {
    let kids = folder_children(cache, dir);
    if kids.is_empty() && depth > 1 {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 14.0);
            ui.label(RichText::new("（空）").weak().size(12.0));
        });
        return;
    }
    for e in &kids {
        tree_row(ui, &e.path, depth, e.is_dir, cache, open, ev);
        if e.is_dir && open.contains(&e.path) {
            folder_tree(ui, &e.path, depth + 1, cache, open, ev);
        }
    }
}

/// Left-aligned list row. `Ui::add_sized` centres its content, so reserve the
/// full width with a left-to-right layout instead.
fn list_row(ui: &mut egui::Ui, rt: RichText, h: f32) -> bool {
    let size = Vec2::new(ui.available_width(), h);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        ui.add(Label::new(rt).selectable(false).sense(egui::Sense::click()))
            .clicked()
    })
    .inner
}

fn packed_color(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)
}

fn packed_of(c: Color32) -> u32 {
    let a = c.to_srgba_unmultiplied();
    ((a[0] as u32) << 16) | ((a[1] as u32) << 8) | a[2] as u32
}

fn swatch_color(v: Option<u32>) -> Color32 {
    v.map(packed_color).unwrap_or(Color32::from_gray(130))
}

fn bg_label(v: Option<u32>) -> String {
    match v {
        None => "跟随主题".to_string(),
        Some(x) => style::BG_PRESETS
            .iter()
            .find(|(_, p)| *p == Some(x))
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| format!("#{:06X}", x)),
    }
}

fn reading_frame(ui: &egui::Ui, bg: Option<u32>) -> Frame {
    let fill = style::reading_theme(bg)
        .map(|t| t.fill)
        .unwrap_or_else(|| ui.style().visuals.panel_fill);
    Frame::new().inner_margin(8.0).fill(fill)
}

/// A custom paper colour also needs matching text and panel tints, otherwise
/// dark backgrounds stay unreadable under the light theme.
fn apply_reading_colors(ui: &mut egui::Ui, bg: Option<u32>) {
    let Some(t) = style::reading_theme(bg) else {
        return;
    };
    let s = ui.style_mut();
    s.visuals.override_text_color = Some(t.text);
    s.visuals.faint_bg_color = t.faint;
    s.visuals.extreme_bg_color = t.extreme;
}

fn empty_view(st: &mut AppState, ui: &mut egui::Ui) {
    if let Some(err) = st.error.take() {
        ui.colored_label(Color32::from_rgb(0xC0, 0x30, 0x3C), err);
        ui.separator();
    }
    ui.vertical_centered(|ui| {
        ui.add_space(70.0);
        if ui
            .add(
                egui::Button::new(RichText::new("打开文件").size(18.0).strong())
                    .min_size(Vec2::new(180.0, 44.0)),
            )
            .clicked()
        {
            let files = rfd::FileDialog::new()
                .add_filter("电子书", &["epub", "md", "markdown", "pdf", "txt", "text"])
                .pick_files();
            if let Some(files) = files {
                for f in files {
                    st.open_paths.push(f);
                }
            }
        }
        ui.add_space(10.0);
        if ui
            .add(
                egui::Button::new(RichText::new("打开目录").size(16.0))
                    .min_size(Vec2::new(180.0, 36.0)),
            )
            .clicked()
        {
            let dir = rfd::FileDialog::new().pick_folder();
            if let Some(dir) = dir {
                st.folder = Some(dir.clone());
                st.folder_cache.clear();
                st.folder_open = [dir].into_iter().collect();
                st.show_toc = true;
                save_now(st);
            }
        }
        ui.add_space(10.0);
        ui.label(RichText::new("支持 EPUB · Markdown · PDF · TXT").weak());
        ui.add_space(6.0);
        ui.label(RichText::new("快捷键：Ctrl+O 打开 · PgDn/PgUp 翻页").weak().size(12.0));
        if !st.lib.books.is_empty() {
            ui.add_space(24.0);
            ui.separator();
            ui.strong("最近阅读");
            let mut recent: Vec<_> = st.lib.books.values().collect();
            recent.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
            for e in recent.iter().take(8) {
                let path = PathBuf::from(&e.path);
                let exists = path.exists();
                let label = format!("{}  ·  {}{}", e.title, e.format, if exists { "" } else { "（文件已移动）" });
                let resp = ui.add_enabled(exists, egui::Button::new(label).min_size(Vec2::new(360.0, 30.0)));
                if resp.clicked() {
                    st.open_paths.push(path);
                }
            }
        }
    });
}

fn render_page(c: &mut Ctx, ui: &mut egui::Ui) {
    let cur = c.st.cur;
    let start = c.st.page;
    let doc = c.doc.clone();
    let font_size = c.st.font_size;
    let zoom = c.zoom;
    let double = c.st.double_page;
    let unit = if double { 2 } else { 1 };
    let total = doc.pages.len();
    let avail = ui.available_width();

    // Render units until the content covers the viewport plus a prefetch
    // margin. A fixed lookahead is not enough when the next chapter is shorter
    // than the window: the scrollbar bottoms out before the flow threshold and
    // the reader stays trapped in the current chapter.
    ui.add_space(16.0);
    let need = c.st.live_offset + ui.available_height() + 800.0;
    let mut acc = 0.0f32;
    let mut base = start;
    let mut drawn = 0usize;
    while base < total {
        if drawn >= 2 && acc > need {
            break;
        }
        let h = c.st.page_h.get(&(cur, base)).copied().unwrap_or(0.0);
        acc += h + 28.0;
        drawn += 1;
        let before = ui.cursor().min.y;
        if double {
            // Two columns need room at the outer edges as well, otherwise the
            // pages read as glued to the window frame. Both columns get an exact
            // width so the page pair stays centred and symmetric.
            let avail = ui.available_width();
            let side = (avail * 0.07).clamp(32.0, 96.0);
            let gap = 28.0;
            let col_w = ((avail - 2.0 * side - gap) / 2.0).max(180.0);
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    render_blocks(c, ui, &doc, cur, base, font_size, zoom);
                });
                ui.add_space(gap);
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    render_blocks(c, ui, &doc, cur, base + 1, font_size, zoom);
                });
                ui.add_space(side);
            });
        } else {
            let col_w = avail.min(760.0);
            let pad = ((avail - col_w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_max_width(col_w);
                    render_blocks(c, ui, &doc, cur, base, font_size, zoom);
                });
            });
        }
        let after = ui.cursor().min.y;
        c.st.page_h.insert((cur, base), (after - before).max(1.0));
        ui.add_space(28.0);
        base += unit;
    }
    ui.add_space(40.0);
}

fn render_blocks(
    c: &mut Ctx,
    ui: &mut egui::Ui,
    doc: &Document,
    cur: usize,
    pg: usize,
    font_size: f32,
    zoom: f32,
) {
    let Some(p) = doc.pages.get(pg) else {
        return;
    };
    for (bi, block) in p.blocks.iter().enumerate() {
        let before = ui.cursor().min.y;
        let key = (cur, pg, bi);
        render_block(c, ui, block, key, font_size, zoom);
        let after = ui.cursor().min.y;
        c.st.block_h.insert(key, (after - before).max(1.0));
    }
}

fn render_block(
    c: &mut Ctx,
    ui: &mut egui::Ui,
    block: &Block,
    key: (usize, usize, usize),
    font_size: f32,
    zoom: f32,
) {
    match block {
        Block::Heading { level, inlines, .. } => {
            let size = match level {
                1 => font_size * 1.7,
                2 => font_size * 1.4,
                3 => font_size * 1.2,
                _ => font_size * 1.08,
            };
            ui.add_space(10.0);
            render_inlines(ui, inlines, size, &c.marks, c.mark_bg, true);
            ui.add_space(6.0);
        }
        Block::Para { inlines } => {
            render_inlines(ui, inlines, font_size, &c.marks, c.mark_bg, false);
            ui.add_space(8.0);
        }
        Block::ListItem { inlines, bullet } => {
            ui.horizontal_wrapped(|ui| {
                let prefix = if *bullet { "• " } else { "‣ " };
                ui.label(RichText::new(prefix).size(font_size));
                render_inlines(ui, inlines, font_size, &c.marks, c.mark_bg, false);
            });
            ui.add_space(2.0);
        }
        Block::Quote { inlines } => {
            Frame::new()
                .fill(ui.style().visuals.faint_bg_color)
                .corner_radius(6.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    render_inlines(ui, inlines, font_size, &c.marks, c.mark_bg, false);
                });
            ui.add_space(8.0);
        }
        Block::Code { text, lang } => {
            Frame::new()
                .fill(ui.style().visuals.faint_bg_color)
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    // The panel spans the whole column so its width does not
                    // follow the length of the longest line.
                    ui.set_width(ui.available_width());
                    let job = hilite::code_job(
                        text,
                        lang,
                        font_size * 0.92,
                        ui.style().visuals.dark_mode,
                    );
                    ui.add(Label::new(job).selectable(true));
                });
            ui.add_space(8.0);
        }
        Block::Image { src, .. } => {
            if let Some(tex) = load_block_image(c, ui, src) {
                let size = tex.size_vec2();
                let max_w = (ui.available_width() - 24.0).max(120.0);
                let scale = (max_w / size.x).min(1.0) * zoom;
                ui.add(egui::Image::new(&tex).max_size(size * scale));
                ui.add_space(6.0);
            } else {
                let name = Path::new(src)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.clone());
                let msg = if src.starts_with("http") {
                    format!("[网络图片 {name}]")
                } else {
                    format!("[图片未找到 {src}]")
                };
                ui.label(RichText::new(msg).weak());
                ui.add_space(6.0);
            }
        }
        Block::PdfPage { page } => {
            let book_idx = key.0;
            if let Some(tex) = c.st.tex_pdf.get(&(book_idx, *page)) {
                let size = tex.size_vec2();
                let scale = (1.0 / 2.0_f32).max(1.0) * zoom;
                ui.add(
                    egui::Image::new(tex)
                        .max_size(size * scale)
                        .sense(egui::Sense::drag()),
                );
                ui.add_space(10.0);
            } else if c.st.pdf_failed.contains(&(book_idx, *page)) {
                ui.label(
                    RichText::new(format!("第 {} 页渲染失败", page + 1))
                        .color(Color32::from_rgb(0xC0, 0x30, 0x3C)),
                );
            } else if !c.st.pdf_queued.contains(&(book_idx, *page)) {
                c.st.pdf_queued.push((book_idx, *page));
                ui.spinner();
                ui.label(format!("正在渲染第 {} 页…", page + 1));
                ui.ctx().request_repaint();
            } else {
                ui.spinner();
                ui.label(format!("正在渲染第 {} 页…", page + 1));
                ui.ctx().request_repaint();
            }
        }
        Block::Divider => {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
        }
    }
}

/// Render one block's inline runs as a single galley so that reader marks can
/// paint a background behind an arbitrary byte range, including ranges that
/// cross bold/italic boundaries.
fn render_inlines(
    ui: &mut egui::Ui,
    inlines: &[Inline],
    size: f32,
    marks: &[String],
    mark_bg: Color32,
    strong: bool,
) {
    let text: String = inlines.iter().map(|i| i.text.as_str()).collect();
    let ranges = if marks.is_empty() {
        Vec::new()
    } else {
        mark_ranges(&text, marks)
    };
    let mut job = egui::text::LayoutJob::default();
    let mut base = 0usize;
    for inl in inlines {
        let mut off = 0usize;
        while off < inl.text.len() {
            let abs = base + off;
            let hit = ranges.iter().find(|r| r.0 <= abs && r.1 > abs);
            let cut = match hit {
                // A mark covers this position: emit up to its end.
                Some(r) => (r.1 - base).min(inl.text.len()),
                // Otherwise stop at the next mark that starts inside this run.
                None => ranges
                    .iter()
                    .map(|r| r.0.saturating_sub(base))
                    .find(|&s| s > off)
                    .unwrap_or(inl.text.len())
                    .min(inl.text.len()),
            };
            let mut rt = inline_richtext(&inl.text[off..cut], &inl.style, size, strong);
            if hit.is_some() {
                rt = rt.background_color(mark_bg);
            }
            rt.append_to(
                &mut job,
                ui.style(),
                egui::FontSelection::Default,
                Align::Max,
            );
            off = cut;
        }
        base += inl.text.len();
    }
    if job.is_empty() {
        return;
    }
    ui.add(Label::new(job).selectable(true));
}

fn inline_richtext(text: &str, style: &Style, size: f32, strong: bool) -> RichText {
    let rt = match style {
        Style::Normal => RichText::new(text).size(size),
        Style::Bold => RichText::new(text).size(size).strong(),
        Style::Italic => RichText::new(text).size(size).italics(),
        Style::BoldItalic => RichText::new(text).size(size).strong().italics(),
        Style::Code => RichText::new(text)
            .size(size)
            .font(FontId::monospace(size))
            .color(Color32::from_rgb(0xB0, 0x30, 0x60)),
    };
    if strong {
        rt.strong()
    } else {
        rt
    }
}

/// Byte ranges of `text` covered by any mark. Both sides are compared with
/// whitespace discarded, because a soft line wrap injects a break anywhere in
/// CJK text while the stored excerpt has none.
fn mark_ranges(text: &str, marks: &[String]) -> Vec<(usize, usize)> {
    let mut chars: Vec<char> = Vec::with_capacity(text.len());
    // (byte start, byte end) in `text` for each entry of `chars`
    let mut bounds: Vec<(usize, usize)> = Vec::with_capacity(text.len());
    let mut off = 0usize;
    for ch in text.chars() {
        let len = ch.len_utf8();
        if !ch.is_whitespace() {
            chars.push(ch);
            bounds.push((off, off + len));
        }
        off += len;
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    for m in marks {
        let needle: Vec<char> = m.chars().filter(|c| !c.is_whitespace()).collect();
        if needle.is_empty() || chars.len() < needle.len() {
            continue;
        }
        let mut i = 0usize;
        while i + needle.len() <= chars.len() {
            if chars[i..i + needle.len()] == needle[..] {
                out.push((bounds[i].0, bounds[i + needle.len() - 1].1));
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    out.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(out.len());
    for (s, e) in out {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Whether `needle` (a reader selection) occurs in the document. Marks that
/// span two blocks cannot match, since each block is matched on its own.
fn doc_has_text(doc: &Document, needle: &str) -> bool {
    if needle.chars().all(|c| c.is_whitespace()) {
        return false;
    }
    let one = [needle.to_string()];
    doc
        .pages
        .iter()
        .flat_map(|p| p.blocks.iter())
        .map(|b| b.plain_text())
        .filter(|t| !t.is_empty())
        .any(|t| !mark_ranges(&t, &one).is_empty())
}

fn load_block_image(c: &mut Ctx, ui: &egui::Ui, src: &str) -> Option<egui::TextureHandle> {
    if let Some(t) = c.st.tex_img.get(src) {
        return Some(t.clone());
    }
    if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("data:") {
        return None;
    }
    let bytes = match &c.doc.data {
        DocData::Epub(ep) => loaders::epub::load_epub_bytes(&ep.zip_bytes, src),
        _ => std::fs::read(src).ok(),
    }?;
    let img = image::load_from_memory(&bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let data = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
    let tex = ui.ctx().load_texture(src.to_string(), data, egui::TextureOptions::LINEAR);
    c.st.tex_img.insert(src.to_string(), tex.clone());
    Some(tex)
}

fn build_style(ctx: &egui::Context, dark: bool) {
    let theme = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    let mut style = egui::Style::default();
    style.visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if !dark {
        style.visuals.extreme_bg_color = Color32::from_rgb(0xFB, 0xF8, 0xF1);
        style.visuals.panel_fill = Color32::from_rgb(0xF6, 0xF3, 0xEC);
    }
    ctx.set_style_of(theme, style);
    // Activating the theme is what actually repaints the UI; set_style_of alone
    // only edits the (possibly inactive) theme's style.
    ctx.set_theme(theme);
}

fn shorten(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let short: String = s.chars().take(n - 1).collect();
        format!("{short}…")
    }
}

fn now_str() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let local = secs + 8 * 3600; // UTC+8 display for CN users
    let (y, mo, d, h, mi, _s) = epoch_to_civil(local);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

fn epoch_to_civil(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z / 146097;
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("--smoke") {
        for p in &args[2..] {
            let path = std::path::Path::new(p);
            match load_doc(path) {
                Ok(doc) => {
                    println!(
                        "OK [{}] {} pages={} toc={}",
                        doc.format.label(),
                        doc.title,
                        doc.pages.len(),
                        doc.toc.len()
                    );
                    if let Some(b) = doc.pages.first().and_then(|p| p.blocks.first()) {
                        println!("   first: {}", shorten(&b.plain_text(), 40));
                    }
                    for t in doc.toc.iter().take(3) {
                        println!("   toc: {} -> p{}", t.title, t.page);
                    }
                    if let DocData::Pdf(pd) = &doc.data {
                        match loaders::pdf::render_page(&pd.bytes, 0, 1.0) {
                            Ok(img) => println!("   pdf page0 {}x{}", img.width, img.height),
                            Err(e) => println!("   pdf page0 ERR {e}"),
                        }
                    }
                }
                Err(e) => println!("ERR {p}: {e}"),
            }
        }
        return Ok(());
    }
    let icon = egui::IconData {
        rgba: ICON_RGBA.to_vec(),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_icon(icon)
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([920.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(ReaderApp::new(&cc.egui_ctx)))),
    )
}
