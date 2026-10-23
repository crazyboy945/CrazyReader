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

mod css;
mod hilite;
mod loaders;
mod model;
mod storage;
mod style;

use css::Sheet;
use loaders::load as load_doc;
use model::{Block, DocData, Document, Inline, Style, TocEntry};
use storage::{Library, Note, Settings};
use style::FontManager;

const APP_TITLE: &str = "CrazyReader";
/// The size the reader's own A+/A− control is centred on: a stylesheet that
/// declares `font-size` is scaled by that control rather than ignoring it.
const BASE_FONT_SIZE: f32 = 17.0;
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
    /// parsed user stylesheet, applied to Markdown documents
    css: Option<Rc<Sheet>>,
    css_path: Option<PathBuf>,
    /// mtime the stylesheet was last read at, so edits reload it
    css_mtime: Option<std::time::SystemTime>,
    css_check: Instant,
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
        let css_path = settings.css.as_ref().map(PathBuf::from);
        let mut fonts = FontManager::new();
        let css = css_path.as_ref().and_then(|p| load_sheet(p));
        let font_key = fonts
            .apply(ctx, &settings.font, &sheet_fonts(css.as_ref()))
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
            css: css.map(Rc::new),
            css_path,
            css_mtime: None,
            css_check: Instant::now(),
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
        css: st
            .css_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
    };
    Library::save(&st.lib, &st.config_file);
    st.last_save = Instant::now();
}

/// Read and parse the stylesheet at `path`; unreadable files are simply absent.
fn load_sheet(path: &Path) -> Option<Sheet> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(css::parse(&text))
}

fn sheet_fonts(css: Option<&Sheet>) -> Vec<String> {
    css.map(|s| s.fonts()).unwrap_or_default()
}

/// (Re)read the user stylesheet and install any face it names.
fn reload_css(st: &mut AppState, ctx: &egui::Context) {
    let sheet = st.css_path.as_deref().and_then(load_sheet);
    st.css_mtime = st
        .css_path
        .as_deref()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());
    let extras = sheet_fonts(sheet.as_ref());
    st.css = sheet.map(Rc::new).filter(|s| !s.is_empty());
    if let Some(k) = st.fonts.apply(ctx, &st.font_key, &extras) {
        st.font_key = k;
    }
    // Glyph metrics change with a stylesheet, so every cached height is stale.
    st.block_h.clear();
    st.page_h.clear();
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
        // A stylesheet is a text file the reader edits elsewhere, so pick up
        // saves without a restart.
        if st.css_check.elapsed().as_secs_f32() > 1.5 {
            st.css_check = Instant::now();
            let changed = st
                .css_path
                .as_deref()
                .and_then(|p| std::fs::metadata(p).ok())
                .and_then(|m| m.modified().ok())
                .is_some_and(|t| Some(t) != st.css_mtime);
            if changed {
                reload_css(st, ctx);
                st.status = "样式表已重新载入".to_string();
                ctx.request_repaint();
            }
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
        if let Some(k) = st.fonts.apply(&ctx, &st.font_key, &sheet_fonts(st.css.as_deref())) {
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
        let mut t_toc = false;
        let mut t_notes = false;
        let mut t_tools = false;
        let cur = st.cur;
        let show_toc = st.show_toc;
        let show_notes = st.show_notes;
        let tools_open = st.sel_tools_pos.is_some();
        egui::Panel::top("topbar")
            .frame(
                Frame::new()
                    .fill(ui.style().visuals.panel_fill)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 5,
                        bottom: 5,
                    }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if icon_button(ui, Icon::OpenFile, "打开文件", "打开电子书文件（Ctrl+O）", false)
                    {
                        open_clicked = true;
                    }
                    if icon_button(ui, Icon::OpenDir, "打开目录", "浏览一个文件夹（Ctrl+D）", false)
                    {
                        open_dir_clicked = true;
                    }
                    ui.add_space(4.0);
                    group(ui, |ui| {
                        if icon_button_at(ui, Icon::Panel, "侧栏", "显示或隐藏左侧栏", show_toc, true)
                        {
                            t_toc = true;
                        }
                        if has_books
                            && icon_button_at(
                                ui,
                                Icon::Pen,
                                "笔记",
                                "显示或隐藏笔记面板",
                                show_notes,
                                true,
                            )
                        {
                            t_notes = true;
                        }
                        if has_books
                            && icon_button_at(
                                ui,
                                Icon::Tools,
                                "工具窗",
                                "选中文字的悬浮窗：标记 / 摘录 / 笔记 / 复制",
                                tools_open,
                                true,
                            )
                        {
                            t_tools = true;
                        }
                    });
                    if has_books {
                        // The active tab is painted in the reading paper colour so it
                        // looks like the top edge of the page it opens onto.
                        let theme = style::reading_theme(st.bg);
                        let page_fill = theme
                            .map(|t| t.fill)
                            .unwrap_or(ui.style().visuals.window_fill);
                        let page_text = theme
                            .map(|t| t.text)
                            .unwrap_or_else(|| ui.visuals().text_color());
                        let acc = accent(ui);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            for (i, b) in st.books.iter().enumerate().rev() {
                                let title = shorten(&b.doc.title, 14);
                                let active = i == cur;
                                let kind = kind_of(&b.doc.path, false);
                                let font = FontId::proportional(12.5);
                                let tw = ui
                                    .fonts_mut(|f| {
                                        f.layout_no_wrap(title.clone(), font.clone(), Color32::WHITE)
                                            .size()
                                            .x
                                    });
                                let h = 26.0;
                                let close_w = 18.0;
                                // Right-to-left: the close box is allocated first so it
                                // ends up on the right of the tab body.
                                let (xr, xresp) = ui.allocate_exact_size(
                                    Vec2::new(close_w, h),
                                    egui::Sense::click(),
                                );
                                let (br, bresp) = ui.allocate_exact_size(
                                    Vec2::new(10.0 + 15.0 + 6.0 + tw + 6.0, h),
                                    egui::Sense::click(),
                                );
                                let tab = br.union(xr);
                                let hovered = bresp.hovered() || xresp.hovered();
                                let fill = if active {
                                    page_fill
                                } else if hovered {
                                    ui.style().visuals.faint_bg_color
                                } else {
                                    Color32::TRANSPARENT
                                };
                                ui.painter().rect(
                                    tab,
                                    egui::CornerRadius::from(7.0),
                                    fill,
                                    egui::Stroke::NONE,
                                    egui::StrokeKind::Inside,
                                );
                                if active {
                                    ui.painter().rect_filled(
                                        egui::Rect::from_min_max(
                                            tab.min,
                                            egui::pos2(tab.max.x, tab.min.y + 2.5),
                                        ),
                                        egui::CornerRadius {
                                            nw: 7,
                                            ne: 7,
                                            sw: 0,
                                            se: 0,
                                        },
                                        acc,
                                    );
                                }
                                ui.painter().rect_stroke(
                                    tab,
                                    7.0,
                                    ui.style().visuals.widgets.noninteractive.bg_stroke,
                                    egui::StrokeKind::Inside,
                                );
                                let fg = if active {
                                    page_text
                                } else {
                                    ui.visuals().text_color()
                                };
                                kind_icon_at(
                                    ui.painter(),
                                    egui::Rect::from_min_size(
                                        egui::pos2(br.min.x + 8.0, br.center().y - 7.5),
                                        Vec2::splat(15.0),
                                    ),
                                    &kind,
                                );
                                ui.painter().text(
                                    egui::pos2(br.min.x + 29.0, br.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &title,
                                    font,
                                    fg,
                                );
                                if xresp.hovered() {
                                    ui.painter().circle_filled(
                                        xr.center(),
                                        8.0,
                                        style::accent_soft(ui.visuals().dark_mode, 60),
                                    );
                                }
                                Icon::Close.paint(
                                    ui.painter(),
                                    egui::Rect::from_min_size(
                                        xr.center() - egui::vec2(5.0, 5.0),
                                        Vec2::splat(10.0),
                                    ),
                                    if active || hovered {
                                        fg
                                    } else {
                                        ui.visuals().weak_text_color()
                                    },
                                    fill,
                                );
                                bresp.clone().on_hover_text(&b.doc.title);
                                xresp.clone().on_hover_text("关闭此页签");
                                if bresp.clicked() && !active {
                                    tab_click = Some(i);
                                }
                                if xresp.clicked() {
                                    tab_close = Some(i);
                                }
                                ui.add_space(6.0);
                            }
                        });
                    }
                });
            });
        if t_toc {
            st.show_toc = !st.show_toc;
        }
        if t_notes {
            st.show_notes = !st.show_notes;
        }
        if t_tools {
            st.sel_tools_pos = if st.sel_tools_pos.is_some() {
                None
            } else {
                let p = ctx
                    .input(|i| i.pointer.interact_pos())
                    .unwrap_or_else(|| ctx.viewport_rect().center());
                Some(clamp_tools(p, &ctx))
            };
        }
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
                    panel_header(ui, "章节目录", 0.0, |_| {});
                    ScrollArea::vertical()
                        .id_salt("toc")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if doc.toc.is_empty() {
                                ui.label(RichText::new("（此文档没有目录）").weak());
                            }
                            // Several entries can share a page; the one being read is
                            // the last of them, not all of them.
                            let active = doc.toc.iter().rposition(|e| e.page <= page);
                            for (idx, e) in doc.toc.iter().enumerate() {
                                let indent = (e.depth as f32) * 12.0;
                                ui.add_space(1.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(indent);
                                    let is_cur = active == Some(idx);
                                    let rt = RichText::new(shorten(&e.title, 30)).size(13.5);
                                    let rt = if is_cur {
                                        rt.color(accent(ui)).strong()
                                    } else {
                                        rt
                                    };
                                    if list_row_sel(ui, rt, 22.0, is_cur) {
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
                    panel_header(ui, "笔记", 0.0, |_| {});
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
                                    if icon_button_at(
                                        ui,
                                        Icon::Close,
                                        "",
                                        "取消引用",
                                        false,
                                        true,
                                    ) {
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
                        if icon_button(ui, Icon::Plus, "添加笔记", "把输入框里的内容存为一条笔记", false) {
                            add_clicked = true;
                        }
                        if icon_button_at(ui, Icon::Trash, "清空", "清空输入框", false, true) {
                            clear_clicked = true;
                        }
                    });
                    ui.separator();
                    // Everything below the editor shares one scroll area. Three stacked
                    // areas overflow a short window: once the last one was left with zero
                    // height, egui laid its content out at unbounded width and the whole
                    // panel slid off the right edge of the screen.
                    let card_fill = ui.style().visuals.faint_bg_color;
                    ScrollArea::vertical()
                        .id_salt("notes_body")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            section_head(ui, "笔记", note_list.len());
                            if note_list.is_empty() {
                                ui.label(RichText::new("（暂无笔记）").weak().size(12.5));
                            }
                            for n in note_list.iter().rev() {
                                card(ui, card_fill, |ui| {
                                    ui.label(RichText::new(&n.text).size(13.5));
                                    ui.horizontal(|ui| {
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if icon_button_at(
                                                    ui,
                                                    Icon::Jump,
                                                    "",
                                                    "跳到原文位置",
                                                    false,
                                                    true,
                                                ) {
                                                    jump_note =
                                                        Some(n.page.min(total.saturating_sub(1)));
                                                }
                                                if icon_button_at(
                                                    ui,
                                                    Icon::Trash,
                                                    "",
                                                    "删除",
                                                    false,
                                                    true,
                                                ) {
                                                    delete_note = Some(n.id);
                                                }
                                            },
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "第 {} 页 · {}",
                                                n.page + 1,
                                                n.created
                                            ))
                                            .size(11.0)
                                            .weak(),
                                        );
                                    });
                                });
                                ui.add_space(6.0);
                            }
                            ui.add_space(6.0);
                            section_head(ui, "摘录", excerpts.len());
                            ui.label(
                                RichText::new("用悬浮工具窗的「摘录」保存选中的原文")
                                    .size(11.5)
                                    .weak(),
                            );
                            ui.add_space(4.0);
                            if excerpts.is_empty() {
                                ui.label(RichText::new("（暂无摘录）").weak().size(12.5));
                            }
                            for n in excerpts.iter().rev() {
                                let Some(q) = n.quote.as_deref() else { continue };
                                card(ui, card_fill, |ui| {
                                    ui.label(RichText::new(q).size(12.5).italics());
                                    ui.horizontal(|ui| {
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if icon_button_at(
                                                    ui,
                                                    Icon::Jump,
                                                    "",
                                                    "跳到原文位置",
                                                    false,
                                                    true,
                                                ) {
                                                    jump_note =
                                                        Some(n.page.min(total.saturating_sub(1)));
                                                }
                                                if icon_button_at(
                                                    ui,
                                                    Icon::Trash,
                                                    "",
                                                    "删除",
                                                    false,
                                                    true,
                                                ) {
                                                    delete_note = Some(n.id);
                                                }
                                            },
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "第 {} 页 · {}",
                                                n.page + 1,
                                                n.created
                                            ))
                                            .size(11.0)
                                            .weak(),
                                        );
                                    });
                                });
                                ui.add_space(6.0);
                            }
                            ui.add_space(6.0);
                            section_head(ui, "标记", marks.len());
                            ui.label(
                                RichText::new("选中文字后，用悬浮工具窗或阅读区底部的「标记」")
                                    .size(11.5)
                                    .weak(),
                            );
                            ui.add_space(4.0);
                            if marks.is_empty() {
                                ui.label(RichText::new("（暂无标记）").weak().size(12.5));
                            }
                            for (i, m) in marks.iter().enumerate().rev() {
                                card(ui, mark_bg, |ui| {
                                    ui.label(RichText::new(m).size(12.5));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if icon_button_at(
                                            ui,
                                            Icon::Trash,
                                            "",
                                            "删除标记",
                                            false,
                                            true,
                                        ) {
                                            delete_mark = Some(i);
                                        }
                                    });
                                });
                                ui.add_space(6.0);
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
        let css_now = st.css_path.clone();
        let css_label = css_now
            .as_ref()
            .and_then(|p| {
                p.file_name()
                    .map(|n| shorten(&n.to_string_lossy(), 18))
            })
            .unwrap_or_else(|| "无（内置样式）".to_string());
        let mut css_pick = false;
        let mut css_clear = false;
        let mut css_reload = false;
        egui::Panel::bottom("status")
            .frame(
                Frame::new()
                    .fill(ui.style().visuals.panel_fill)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 5,
                        bottom: 5,
                    }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} · 第 {} / {total} 页 · {:.0}%",
                            doc.format.label(),
                            page_now + 1,
                            (page_now + 1) as f32 * 100.0 / total.max(1) as f32
                        ))
                        .size(12.5)
                        .strong(),
                    );
                    ui.add_space(4.0);
                    group(ui, |ui| {
                        if ui.button("A−").on_hover_text("减小字号").clicked() {
                            font_minus = true;
                        }
                        ui.label(RichText::new(format!("{:.0}", st_font)).size(12.5));
                        if ui.button("A+").on_hover_text("增大字号").clicked() {
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
                    });
                    group(ui, |ui| {
                        if icon_button_at(ui, Icon::PageSingle, "单页", "单栏阅读", !st_double, true)
                        {
                            set_single = true;
                        }
                        if icon_button_at(ui, Icon::PageDouble, "双页", "双栏阅读", st_double, true)
                        {
                            set_double = true;
                        }
                        ui.separator();
                        if icon_button_at(
                            ui,
                            if st_dark { Icon::Sun } else { Icon::Moon },
                            if st_dark { "浅色主题" } else { "深色主题" },
                            "切换浅色 / 深色主题",
                            false,
                            true,
                        ) {
                            theme_toggle = true;
                        }
                        ui.separator();
                        ui.add_enabled_ui(sel_now.is_some(), |ui| {
                            if icon_button_at(
                                ui,
                                Icon::Mark,
                                "标记",
                                "为选中的文字加上背景色（Ctrl+M）",
                                false,
                                true,
                            ) {
                                mark_clicked = true;
                            }
                        });
                    });
                    group(ui, |ui| {
                        ui.label(RichText::new("字体").size(12.5).weak());
                        egui::ComboBox::from_id_salt("readfont")
                            .selected_text(font_label)
                            .show_ui(ui, |ui| {
                                if font_items.is_empty() {
                                    ui.label(RichText::new("（未找到中文字体）").weak());
                                }
                                for (k, l) in &font_items {
                                    if ui
                                        .selectable_label(
                                            k == &font_key_now,
                                            RichText::new(l).size(13.5),
                                        )
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
                                            RichText::new(format!("■ {name}"))
                                                .color(sw)
                                                .size(13.5),
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
                        if matches!(doc.format, model::Format::Markdown) {
                            ui.label(RichText::new("样式").size(12.5).weak());
                            egui::ComboBox::from_id_salt("readcss")
                                .selected_text(css_label)
                                .width(150.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            css_now.is_none(),
                                            RichText::new("无（内置样式）").size(13.5),
                                        )
                                        .clicked()
                                    {
                                        css_clear = true;
                                    }
                                    if ui
                                        .selectable_label(
                                            false,
                                            RichText::new("选择 CSS 文件…").size(13.5),
                                        )
                                        .clicked()
                                    {
                                        css_pick = true;
                                    }
                                    if css_now.is_some() {
                                        if ui
                                            .selectable_label(
                                                false,
                                                RichText::new("重新载入").size(13.5),
                                            )
                                            .clicked()
                                        {
                                            css_reload = true;
                                        }
                                        if let Some(s) = st.css.as_deref() {
                                            for e in s.errors.iter().take(4) {
                                                ui.label(
                                                    RichText::new(e)
                                                        .size(11.5)
                                                        .color(Color32::from_rgb(0xC0, 0x30, 0x3C)),
                                                );
                                            }
                                        }
                                    }
                                });
                        }
                    });
                    // Status lives here, not in the top bar: the top bar is for
                    // navigation, and a transient message should not shift the tabs.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !st.status.is_empty() {
                            ui.add(
                                Label::new(RichText::new(&st.status).weak().size(12.0)).truncate(),
                            );
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
        if css_pick {
            if let Some(file) = rfd::FileDialog::new()
                .add_filter("CSS 样式表", &["css"])
                .pick_file()
            {
                st.css_path = Some(file.clone());
                reload_css(st, &ctx);
                save_now(st);
                st.status = match &file.file_name() {
                    Some(n) => format!("已应用样式表 {}", n.to_string_lossy()),
                    None => "已应用样式表".to_string(),
                };
                ui.ctx().request_repaint();
            }
        }
        if css_clear {
            st.css_path = None;
            st.css = None;
            st.css_mtime = None;
            st.status = "已改用内置样式".to_string();
            save_now(st);
            ui.ctx().request_repaint();
        }
        if css_reload {
            reload_css(st, &ctx);
            st.status = "样式表已重新载入".to_string();
            save_now(st);
            ui.ctx().request_repaint();
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
        // Only Markdown takes a stylesheet; the other formats bring their own
        // layout and would only be distorted by it.
        let css = if matches!(doc.format, model::Format::Markdown) {
            st.css.clone()
        } else {
            None
        };
        let scale = st.font_size / BASE_FONT_SIZE;
        let body = css
            .as_ref()
            .and_then(|s| s.own("body").font_size)
            .map(|l| l.px(BASE_FONT_SIZE, 0.0))
            .unwrap_or(BASE_FONT_SIZE);
        let mut c = Ctx {
            st,
            doc: &doc,
            zoom: st_zoom,
            marks,
            mark_bg,
            css,
            body,
            scale,
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
    /// User stylesheet, only set for Markdown documents.
    css: Option<Rc<Sheet>>,
    /// `body { font-size }` as authored, before the reader's own zoom.
    body: f32,
    /// The reader's A+/A− control expressed as a multiplier.
    scale: f32,
}

/// How one block should look once the stylesheet has been applied.
#[derive(Clone)]
struct Fmt {
    size: f32,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: Option<Color32>,
    bg: Option<Color32>,
    line_h: Option<f32>,
    family: Option<egui::FontFamily>,
    align: Option<Align>,
    indent: f32,
    before: f32,
    after: f32,
    pad: Option<[f32; 4]>,
    border: Option<(f32, Color32)>,
}

impl Ctx<'_> {
    /// Resolve the look of `sel`, falling back to the reader's own defaults.
    ///
    /// `before`/`after` are the spacing the reader uses when the sheet stays
    /// silent about margins, so a stylesheet can override either side alone.
    fn style(&self, sel: &str, size: f32, bold: bool, before: f32, after: f32, ref_w: f32) -> Fmt {
        let mut f = Fmt {
            size,
            bold,
            italic: false,
            underline: false,
            strike: false,
            color: None,
            bg: None,
            line_h: None,
            family: None,
            align: None,
            indent: 0.0,
            before,
            after,
            pad: None,
            border: None,
        };
        let Some(sheet) = self.css.as_deref() else {
            return f;
        };
        let own = sheet.own(sel);
        let inh = sheet.rule(&[sel]);
        if let Some(l) = own.font_size {
            f.size = l.px(self.body, self.body) * self.scale;
        }
        if let Some(b) = own.bold {
            f.bold = b;
        }
        f.italic = inh.italic.unwrap_or(false);
        f.underline = inh.underline.unwrap_or(false);
        f.strike = inh.strike.unwrap_or(false);
        f.color = inh.color;
        f.bg = own.background;
        if let Some(l) = inh.line_height {
            f.line_h = Some(l.px(f.size, f.size).clamp(f.size, f.size * 4.0));
        }
        f.align = own.text_align.map(|a| match a {
            css::TextAlign::Center => Align::Center,
            css::TextAlign::Right => Align::Max,
            css::TextAlign::Left => Align::Min,
        });
        f.indent = own.text_indent.map(|l| l.px(f.size, ref_w)).unwrap_or(0.0);
        f.before = own.margin_top.map(|l| l.px(f.size, ref_w)).unwrap_or(before);
        f.after = own
            .margin_bottom
            .map(|l| l.px(f.size, ref_w))
            .unwrap_or(after);
        f.pad = own.padding.map(|p| {
            [
                p[0].px(f.size, ref_w),
                p[1].px(f.size, ref_w),
                p[2].px(f.size, ref_w),
                p[3].px(f.size, ref_w),
            ]
        });
        f.border = own.border_left.map(|(w, c)| (w * self.scale, c));
        f.family = inh.font_family.as_deref().and_then(|name| {
            let key = css::font_key(name)?;
            if key == "mono" {
                Some(egui::FontFamily::Monospace)
            } else if self.st.fonts.has(key) {
                Some(egui::FontFamily::Name(key.into()))
            } else {
                None
            }
        });
        f
    }

    /// Inline code and `<pre>` blocks want a monospace face unless the sheet says
    /// otherwise.
    fn mono(&self, mut f: Fmt) -> Fmt {
        if self.css.is_some() {
            f.family = Some(egui::FontFamily::Monospace);
        }
        f
    }
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

/// The floating selection toolbar: 标记 / 摘录 / 笔记 / 复制, all on one row
/// with the close button at the right end. It opens where the drag ended and
/// moves by its header; the next selection brings it back.
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
                .shadow(egui::epaint::Shadow {
                    offset: [0, 5],
                    blur: 16,
                    spread: 0,
                    color: Color32::from_black_alpha(28),
                })
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
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
                        for (icon, label, key, tip) in [
                            (
                                Icon::Mark,
                                "标记",
                                "mark",
                                "为选中的文字加上背景色（Ctrl+M）",
                            ),
                            (Icon::Quote, "摘录", "quote", "把选中的原文存入摘录列表"),
                            (Icon::Pen, "笔记", "note", "针对选中的文字写笔记"),
                            (Icon::Copy, "复制", "copy", "复制选中的文字到剪贴板"),
                        ] {
                            let hit = ui
                                .add_enabled_ui(sel.is_some(), |ui| {
                                    icon_button_at(ui, icon, label, tip, false, true)
                                })
                                .inner;
                            if hit {
                                act = Some(key);
                            }
                        }
                        ui.separator();
                        if icon_button_at(
                            ui,
                            Icon::Close,
                            "",
                            "关闭悬浮窗（重新选中文字会再次出现）",
                            false,
                            true,
                        ) {
                            close = true;
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
    let title = match folder {
        Some(r) => r.display().to_string(),
        None => "文件夹".to_string(),
    };
    panel_header(ui, &title, 56.0, |ui| {
        if folder.is_some()
            && icon_button_at(ui, Icon::Refresh, "刷新", "重新读取目录", false, true)
        {
            ev.refresh = true;
        }
    });
    if let Some(root) = folder {
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
    kind_icon_at(&ui.painter_at(rect.shrink(1.0)), rect.shrink(1.0), kind);
    let clicked = resp.clicked();
    resp.on_hover_text(kind.label());
    clicked
}

/// Same glyph as [`kind_icon`], painted into a caller-owned rect (page tabs).
fn kind_icon_at(p: &egui::Painter, rect: egui::Rect, kind: &Kind) {
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
///
/// The highlight is painted from `rect_contains_pointer` before the label,
/// because egui draws shapes in submission order and the row's own `Response`
/// only exists after the text has already been painted.
fn list_row(ui: &mut egui::Ui, rt: RichText, h: f32) -> bool {
    list_row_sel(ui, rt, h, false)
}

fn list_row_sel(ui: &mut egui::Ui, rt: RichText, h: f32, selected: bool) -> bool {
    let w = ui.available_width().max(60.0);
    let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(w, h));
    if selected {
        let a = accent(ui);
        ui.painter()
            .rect_filled(rect.shrink(1.0), 6.0, a.linear_multiply(0.22));
        let line = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + 1.0, rect.min.y + 4.0),
            egui::pos2(rect.min.x + 3.5, rect.max.y - 4.0),
        );
        ui.painter().rect_filled(line, 1.5, a);
    } else if ui.rect_contains_pointer(rect) {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            6.0,
            ui.style().visuals.faint_bg_color,
        );
    }
    ui.allocate_ui_with_layout(Vec2::new(w, h), Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(if selected { 6.0 } else { 2.0 });
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
    let ref_w = ui.available_width();
    match block {
        Block::Heading { level, inlines, .. } => {
            let sel = match level {
                1 => "h1",
                2 => "h2",
                3 => "h3",
                4 => "h4",
                5 => "h5",
                _ => "h6",
            };
            let size = match level {
                1 => font_size * 1.7,
                2 => font_size * 1.4,
                3 => font_size * 1.2,
                _ => font_size * 1.08,
            };
            let f = c.style(sel, size, true, 10.0, 6.0, ref_w);
            ui.add_space(f.before);
            show_block_frame(c, ui, &f, |ui| {
                render_inlines(ui, inlines, &f, &c.marks, c.mark_bg);
            });
            ui.add_space(f.after);
        }
        Block::Para { inlines } => {
            let f = c.style("p", font_size, false, 0.0, 8.0, ref_w);
            ui.add_space(f.before);
            if f.indent != 0.0 {
                ui.indent("css-indent", |ui| ui.add_space(f.indent));
            }
            show_block_frame(c, ui, &f, |ui| {
                render_inlines(ui, inlines, &f, &c.marks, c.mark_bg);
            });
            ui.add_space(f.after);
        }
        Block::ListItem { inlines, bullet } => {
            let f = c.style("li", font_size, false, 0.0, 2.0, ref_w);
            ui.horizontal_wrapped(|ui| {
                if f.indent != 0.0 {
                    ui.add_space(f.indent);
                }
                let prefix = if *bullet { "• " } else { "‣ " };
                ui.label(RichText::new(prefix).size(f.size));
                render_inlines(ui, inlines, &f, &c.marks, c.mark_bg);
            });
            ui.add_space(f.after);
        }
        Block::Quote { inlines } => {
            let f = c.style("blockquote", font_size, false, 0.0, 8.0, ref_w);
            ui.add_space(f.before);
            let pad = f.pad.unwrap_or([12.0, 12.0, 12.0, 12.0]);
            block_frame(&f, pad, ui, |ui| {
                render_inlines(ui, inlines, &f, &c.marks, c.mark_bg);
            });
            ui.add_space(f.after);
        }
        Block::Code { text, lang } => {
            let f = c.style("pre", font_size * 0.92, false, 0.0, 8.0, ref_w);
            let f = c.mono(f);
            ui.add_space(f.before);
            let pad = f.pad.unwrap_or([10.0, 10.0, 10.0, 10.0]);
            block_frame(&f, pad, ui, |ui| {
                // The panel spans the whole column so its width does not
                // follow the length of the longest line.
                ui.set_width(ui.available_width());
                let mut job = hilite::code_job(
                    text,
                    lang,
                    f.size,
                    ui.style().visuals.dark_mode,
                );
                if let Some(col) = f.color {
                    for s in job.sections.iter_mut() {
                        s.format.color = col;
                    }
                }
                ui.add(Label::new(job).selectable(true));
            });
            ui.add_space(f.after);
        }
        Block::Image { src, .. } => {
            if let Some(tex) = load_block_image(c, ui, src) {
                let size = tex.size_vec2();
                let cap = c
                    .css
                    .as_ref()
                    .and_then(|s| s.own("img").max_width)
                    .map(|l| l.px(font_size, ref_w - 24.0));
                let max_w = match cap {
                    Some(v) => v.min(ref_w - 24.0).max(60.0),
                    None => (ref_w - 24.0).max(120.0),
                };
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
            let f = c.style("hr", font_size, false, 6.0, 6.0, ref_w);
            ui.add_space(f.before);
            ui.separator();
            ui.add_space(f.after);
        }
    }
}

/// Draw a block, wrapped in the frame its stylesheet asked for.
///
/// Without a stylesheet only the existing quote/code cards get a frame, so the
/// reader's default look is untouched.
fn show_block_frame(
    c: &Ctx,
    ui: &mut egui::Ui,
    f: &Fmt,
    content: impl FnOnce(&mut egui::Ui),
) {
    if c.css.is_none() || (f.bg.is_none() && f.border.is_none() && f.pad.is_none()) {
        content(ui);
        return;
    }
    let pad = f.pad.unwrap_or([0.0, 0.0, 0.0, 0.0]);
    block_frame(f, pad, ui, content);
}

/// A rounded panel carrying the CSS background, padding and left border.
fn block_frame(
    f: &Fmt,
    pad: [f32; 4],
    ui: &mut egui::Ui,
    content: impl FnOnce(&mut egui::Ui),
) {
    let fill = f
        .bg
        .unwrap_or_else(|| ui.style().visuals.faint_bg_color);
    let margin = egui::Margin {
        left: pad[3].clamp(0.0, 96.0).round() as i8,
        right: pad[1].clamp(0.0, 96.0).round() as i8,
        top: pad[0].clamp(0.0, 96.0).round() as i8,
        bottom: pad[2].clamp(0.0, 96.0).round() as i8,
    };
    let frame = Frame::new().fill(fill).corner_radius(6.0).inner_margin(margin);
    // A CSS background or border belongs to the block, not to the glyphs: the
    // panel has to span the text column instead of hugging the longest line.
    let inner = |ui: &mut egui::Ui| {
        ui.set_width(ui.available_width());
        content(ui);
    };
    match f.border {
        Some((w, col)) => {
            let resp = frame.show(ui, inner);
            let rect = resp.response.rect;
            let r = rect.expand(0.5);
            ui.painter().rect(
                egui::Rect::from_min_max(
                    egui::pos2(r.min.x, r.min.y),
                    egui::pos2(r.min.x + w, r.max.y),
                ),
                0.0,
                col,
                egui::Stroke::NONE,
                egui::StrokeKind::Inside,
            );
        }
        None => {
            frame.show(ui, inner);
        }
    }
}

/// Render one block's inline runs as a single galley so that reader marks can
/// paint a background behind an arbitrary byte range, including ranges that
/// cross bold/italic boundaries.
fn render_inlines(
    ui: &mut egui::Ui,
    inlines: &[Inline],
    f: &Fmt,
    marks: &[String],
    mark_bg: Color32,
) {
    let text: String = inlines.iter().map(|i| i.text.as_str()).collect();
    let ranges = if marks.is_empty() {
        Vec::new()
    } else {
        mark_ranges(&text, marks)
    };
    let mut job = egui::text::LayoutJob::default();
    job.halign = f.align.unwrap_or(Align::Min);
    if f.indent > 0.0 {
        // CSS `text-indent` only shifts the first line. Ideographic spaces are
        // the unit Chinese typesetting uses for it, so a whole number of them
        // reproduces `2em` without a custom layout pass.
        let n = (f.indent / f.size.max(1.0)).round() as usize;
        RichText::new("　".repeat(n.clamp(1, 8)))
            .size(f.size)
            .append_to(&mut job, ui.style(), egui::FontSelection::Default, Align::Min);
    }
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
            let mut rt = inline_richtext(&inl.text[off..cut], &inl.style, f);
            if hit.is_some() {
                rt = rt.background_color(mark_bg);
            }
            let from = job.sections.len();
            rt.append_to(
                &mut job,
                ui.style(),
                egui::FontSelection::Default,
                Align::Max,
            );
            // A stylesheet colour has to beat the reading theme's own override,
            // which RichText applies while appending.
            if let Some(col) = f.color {
                for s in job.sections[from..].iter_mut() {
                    s.format.color = col;
                }
            }
            off = cut;
        }
        base += inl.text.len();
    }
    if job.is_empty() {
        return;
    }
    ui.add(Label::new(job).selectable(true));
}

fn inline_richtext(text: &str, style: &Style, f: &Fmt) -> RichText {
    let mut rt = RichText::new(text).size(f.size);
    if let Some(fam) = &f.family {
        rt = rt.font(FontId::new(f.size, fam.clone()));
    }
    if f.line_h.is_some() {
        rt = rt.line_height(f.line_h);
    }
    if f.italic {
        rt = rt.italics();
    }
    if f.underline {
        rt = rt.underline();
    }
    if f.strike {
        rt = rt.strikethrough();
    }
    match style {
        Style::Normal => {}
        Style::Bold => {
            rt = rt.strong();
        }
        Style::Italic => {
            rt = rt.italics();
        }
        Style::BoldItalic => {
            rt = rt.strong().italics();
        }
        Style::Code => {
            rt = rt
                .font(FontId::monospace(f.size))
                .color(Color32::from_rgb(0xB0, 0x30, 0x60));
        }
    }
    if f.bold {
        rt = rt.strong()
    }
    rt
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
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let accent = style::accent(dark);

    // Paper-tinted surfaces keep the reading area and the chrome distinguishable
    // without a second theme switch. The chrome is a cool neutral so the warm
    // reading paper stands out as the page, not as another panel.
    if dark {
        v.panel_fill = Color32::from_rgb(0x1C, 0x1E, 0x22);
        v.window_fill = Color32::from_rgb(0x16, 0x17, 0x1A);
        v.faint_bg_color = Color32::from_rgb(0x26, 0x2A, 0x30);
        v.extreme_bg_color = Color32::from_rgb(0x0F, 0x10, 0x13);
    } else {
        v.panel_fill = Color32::from_rgb(0xF2, 0xF3, 0xF5);
        v.window_fill = Color32::from_rgb(0xFA, 0xFB, 0xFC);
        v.faint_bg_color = Color32::from_rgb(0xE8, 0xEA, 0xEE);
        v.extreme_bg_color = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    }
    v.override_text_color = None;
    v.selection.bg_fill = style::accent_soft(dark, 46);
    v.selection.stroke = egui::Stroke::new(1.0, accent);
    v.hyperlink_color = accent;
    v.widgets.noninteractive.bg_fill = v.panel_fill;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(
        1.0,
        if dark {
            Color32::from_rgb(0x33, 0x38, 0x3F)
        } else {
            Color32::from_rgb(0xDD, 0xE1, 0xE7)
        },
    );
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, v.text_color());
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = 7.0.into();
        w.bg_stroke.width = 0.0;
    }
    v.widgets.inactive.weak_bg_fill = if dark {
        Color32::from_rgb(0x2E, 0x32, 0x38)
    } else {
        Color32::from_rgb(0xEC, 0xE7, 0xDC)
    };
    v.widgets.hovered.weak_bg_fill = style::accent_soft(dark, 34);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, accent);
    v.widgets.active.weak_bg_fill = accent;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, if dark {
        Color32::from_rgb(0x14, 0x16, 0x1A)
    } else {
        Color32::WHITE
    });
    style.visuals = v;

    style.spacing.item_spacing = Vec2::new(7.0, 4.0);
    style.spacing.button_padding = Vec2::new(9.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(0);
    style.spacing.slider_width = 120.0;
    for (name, size) in [
        (egui::TextStyle::Small, 12.0),
        (egui::TextStyle::Body, 14.0),
        (egui::TextStyle::Button, 13.5),
        (egui::TextStyle::Heading, 19.0),
        (egui::TextStyle::Monospace, 13.0),
    ] {
        if let Some(id) = style.text_styles.get_mut(&name) {
            id.size = size;
        }
    }

    ctx.set_style_of(theme, style);
    // Activating the theme is what actually repaints the UI; set_style_of alone
    // only edits the (possibly inactive) theme's style.
    ctx.set_theme(theme);
}

/// The blue the reader points with, for the current frame's theme.
fn accent(ui: &egui::Ui) -> Color32 {
    style::accent(ui.visuals().dark_mode)
}

/// Toolbar glyphs, drawn as vectors.
///
/// egui bundles no icon font and the CJK reading fonts turn most pictographs
/// into tofu boxes, so every glyph here is a handful of strokes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    OpenFile,
    OpenDir,
    Refresh,
    Panel,
    Pen,
    Tools,
    Sun,
    Moon,
    PageSingle,
    PageDouble,
    Mark,
    Trash,
    Jump,
    Copy,
    Quote,
    Close,
    Plus,
}

impl Icon {
    fn paint(self, p: &egui::Painter, r: egui::Rect, col: Color32, bg: Color32) {
        let s = egui::Stroke::new(1.4, col);
        let (x0, y0, x1, y1) = (r.min.x, r.min.y, r.max.x, r.max.y);
        let c = r.center();
        let line = |a: egui::Pos2, b: egui::Pos2| p.line_segment([a, b], s);
        let poly = |pts: Vec<egui::Pos2>, fill: Color32| {
            p.add(egui::Shape::convex_polygon(pts, fill, s));
        };
        match self {
            Icon::OpenFile => {
                poly(
                    vec![
                        egui::pos2(x0 + 3.0, y0 + 0.5),
                        egui::pos2(x1 - 4.0, y0 + 0.5),
                        egui::pos2(x1 - 1.0, y0 + 3.5),
                        egui::pos2(x1 - 1.0, y1 - 0.5),
                        egui::pos2(x0 + 3.0, y1 - 0.5),
                    ],
                    Color32::TRANSPARENT,
                );
                line(egui::pos2(x1 - 4.0, y0 + 0.5), egui::pos2(x1 - 4.0, y0 + 3.5));
                line(egui::pos2(x1 - 4.0, y0 + 3.5), egui::pos2(x1 - 1.0, y0 + 3.5));
                line(egui::pos2(x0 + 5.0, c.y + 1.5), egui::pos2(x0 + 9.5, c.y + 1.5));
                line(egui::pos2(x0 + 7.25, c.y - 0.75), egui::pos2(x0 + 7.25, c.y + 3.75));
            }
            Icon::OpenDir => {
                poly(
                    vec![
                        egui::pos2(x0 + 1.0, y0 + 3.0),
                        egui::pos2(x0 + 5.5, y0 + 3.0),
                        egui::pos2(x0 + 7.0, y0 + 5.0),
                        egui::pos2(x1 - 1.0, y0 + 5.0),
                        egui::pos2(x1 - 1.0, y1 - 2.0),
                        egui::pos2(x0 + 1.0, y1 - 2.0),
                    ],
                    Color32::TRANSPARENT,
                );
                line(egui::pos2(x0 + 1.0, y0 + 3.0), egui::pos2(x0 + 1.0, y1 - 2.0));
            }
            Icon::Refresh => {
                p.add(egui::Shape::circle_stroke(c, r.width() * 0.34, s));
                // Break the ring and add an arrowhead so it reads as "reload".
                p.line_segment(
                    [
                        egui::pos2(c.x + r.width() * 0.34, c.y),
                        egui::pos2(c.x + r.width() * 0.34 - 3.0, c.y - 2.5),
                    ],
                    s,
                );
                p.line_segment(
                    [
                        egui::pos2(c.x + r.width() * 0.34, c.y),
                        egui::pos2(c.x + r.width() * 0.34 - 3.0, c.y + 2.5),
                    ],
                    s,
                );
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(c.x + r.width() * 0.34 - 1.0, c.y - 1.6),
                        egui::pos2(c.x + r.width() * 0.34 + 1.6, c.y + 1.6),
                    ),
                    0.0,
                    col,
                );
            }
            Icon::Panel => {
                p.rect(r.shrink(1.0), 2.0, Color32::TRANSPARENT, s, egui::StrokeKind::Inside);
                line(
                    egui::pos2(x0 + 1.0 + r.width() * 0.36, y0 + 1.0),
                    egui::pos2(x0 + 1.0 + r.width() * 0.36, y1 - 1.0),
                );
            }
            Icon::Pen => {
                poly(
                    vec![
                        egui::pos2(x0 + 2.0, y1 - 2.0),
                        egui::pos2(x0 + 3.0, y1 - 5.5),
                        egui::pos2(x1 - 3.0, y0 + 1.0),
                        egui::pos2(x1 - 1.0, y0 + 3.0),
                        egui::pos2(x0 + 5.5, y1 - 3.0),
                    ],
                    Color32::TRANSPARENT,
                );
            }
            Icon::Tools => {
                // Mouse pointer.
                poly(
                    vec![
                        egui::pos2(x0 + 3.0, y0 + 1.0),
                        egui::pos2(x0 + 3.0, y1 - 3.0),
                        egui::pos2(x0 + 6.0, y1 - 5.5),
                        egui::pos2(x0 + 8.0, y1 - 1.0),
                        egui::pos2(x0 + 10.0, y1 - 2.0),
                        egui::pos2(x0 + 8.0, y1 - 6.0),
                        egui::pos2(x1 - 1.5, y1 - 6.5),
                    ],
                    Color32::TRANSPARENT,
                );
            }
            Icon::Sun => {
                p.add(egui::Shape::circle_filled(c, r.width() * 0.22, col));
                for i in 0..8 {
                    let a = i as f32 * std::f32::consts::TAU / 8.0;
                    let (dx, dy) = (a.cos(), a.sin());
                    line(
                        egui::pos2(c.x + dx * r.width() * 0.32, c.y + dy * r.width() * 0.32),
                        egui::pos2(c.x + dx * r.width() * 0.46, c.y + dy * r.width() * 0.46),
                    );
                }
            }
            Icon::Moon => {
                let mut pts = Vec::new();
                let ro = r.width() * 0.44;
                for i in 0..=12 {
                    let a = std::f32::consts::FRAC_PI_2 * 0.7 + i as f32 / 12.0 * std::f32::consts::TAU * 0.72;
                    pts.push(egui::pos2(c.x + a.cos() * ro, c.y + a.sin() * ro));
                }
                for i in (0..=10).rev() {
                    let a = std::f32::consts::FRAC_PI_2 * 0.7 + i as f32 / 10.0 * std::f32::consts::TAU * 0.72;
                    pts.push(egui::pos2(
                        c.x + 2.2 + a.cos() * ro * 0.72,
                        c.y + a.sin() * ro * 0.72,
                    ));
                }
                p.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
            }
            Icon::PageSingle => {
                p.rect(r.shrink(2.0), 1.5, Color32::TRANSPARENT, s, egui::StrokeKind::Inside);
            }
            Icon::PageDouble => {
                let half = r.width() * 0.42;
                p.rect(
                    egui::Rect::from_min_size(r.min + egui::vec2(1.0, 2.0), Vec2::new(half, r.height() - 4.0)),
                    1.5,
                    Color32::TRANSPARENT,
                    s,
                    egui::StrokeKind::Inside,
                );
                p.rect(
                    egui::Rect::from_min_size(
                        egui::pos2(r.max.x - 1.0 - half, r.min.y + 2.0),
                        Vec2::new(half, r.height() - 4.0),
                    ),
                    1.5,
                    Color32::TRANSPARENT,
                    s,
                    egui::StrokeKind::Inside,
                );
            }
            Icon::Mark => {
                // Highlighter swipe: a thick slanted tip over an underline.
                p.line_segment(
                    [egui::pos2(x0 + 3.0, y1 - 5.0), egui::pos2(x1 - 4.0, y0 + 1.5)],
                    egui::Stroke::new(3.4, col),
                );
                line(egui::pos2(x0 + 1.5, y1 - 1.5), egui::pos2(x1 - 1.5, y1 - 1.5));
            }
            Icon::Trash => {
                line(egui::pos2(x0 + 2.0, y0 + 3.0), egui::pos2(x1 - 2.0, y0 + 3.0));
                line(egui::pos2(c.x - 2.0, y0 + 3.0), egui::pos2(c.x - 2.0, y0 + 1.5));
                line(egui::pos2(c.x - 2.0, y0 + 1.5), egui::pos2(c.x + 2.0, y0 + 1.5));
                line(egui::pos2(c.x + 2.0, y0 + 1.5), egui::pos2(c.x + 2.0, y0 + 3.0));
                poly(
                    vec![
                        egui::pos2(x0 + 3.5, y0 + 3.0),
                        egui::pos2(x1 - 3.5, y0 + 3.0),
                        egui::pos2(x1 - 4.5, y1 - 1.0),
                        egui::pos2(x0 + 4.5, y1 - 1.0),
                    ],
                    Color32::TRANSPARENT,
                );
            }
            Icon::Jump => {
                line(egui::pos2(x0 + 2.0, c.y), egui::pos2(x1 - 4.0, c.y));
                line(egui::pos2(x1 - 4.0, c.y), egui::pos2(x1 - 7.0, c.y - 3.0));
                line(egui::pos2(x1 - 4.0, c.y), egui::pos2(x1 - 7.0, c.y + 3.0));
                line(egui::pos2(x1 - 1.5, y0 + 2.0), egui::pos2(x1 - 1.5, y1 - 2.0));
            }
            Icon::Copy => {
                p.rect(
                    egui::Rect::from_min_size(r.min + egui::vec2(1.0, 1.0), Vec2::new(r.width() - 5.0, r.height() - 5.0)),
                    1.5,
                    Color32::TRANSPARENT,
                    s,
                    egui::StrokeKind::Inside,
                );
                p.rect(
                    egui::Rect::from_min_size(r.min + egui::vec2(4.0, 4.0), Vec2::new(r.width() - 5.0, r.height() - 5.0)),
                    1.5,
                    bg,
                    s,
                    egui::StrokeKind::Inside,
                );
            }
            Icon::Quote => {
                for dx in [0.0, r.width() * 0.42] {
                    p.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(x0 + 3.0 + dx, y0 + 2.0),
                            egui::pos2(x0 + 6.5 + dx, y0 + 2.0),
                            egui::pos2(x0 + 5.0 + dx, y1 - 2.0),
                            egui::pos2(x0 + 2.5 + dx, y1 - 2.0),
                        ],
                        col,
                        egui::Stroke::NONE,
                    ));
                }
            }
            Icon::Close => {
                line(egui::pos2(x0 + 3.0, y0 + 3.0), egui::pos2(x1 - 3.0, y1 - 3.0));
                line(egui::pos2(x1 - 3.0, y0 + 3.0), egui::pos2(x0 + 3.0, y1 - 3.0));
            }
            Icon::Plus => {
                line(egui::pos2(c.x - 4.0, c.y), egui::pos2(c.x + 4.0, c.y));
                line(egui::pos2(c.x, c.y - 4.0), egui::pos2(c.x, c.y + 4.0));
            }
        }
    }
}

/// A button with a vector glyph in front of its label.
fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, tip: &str, selected: bool) -> bool {
    icon_button_at(ui, icon, label, tip, selected, false)
}

/// `small` shrinks the control for row-embedded actions like 删除 / 跳转.
fn icon_button_at(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    tip: &str,
    selected: bool,
    small: bool,
) -> bool {
    let font = FontId::proportional(if small { 12.0 } else { 13.0 });
    let tw = if label.is_empty() {
        0.0
    } else {
        ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x)
    };
    let pad = if small { 4.0 } else { 8.0 };
    let gap = if label.is_empty() { 0.0 } else { 6.0 };
    let icon_sz = if small { 12.0 } else { 14.0 };
    let h = if small { 20.0 } else { 24.0 };
    let size = Vec2::new(pad * 2.0 + icon_sz + gap + tw, h);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());
    let enabled = ui.is_enabled();
    let w = ui.style().interact(&resp);
    let acc = accent(ui);
    let fill = if selected {
        style::accent_soft(ui.visuals().dark_mode, 40)
    } else if enabled && (resp.is_pointer_button_down_on() || resp.hovered()) {
        w.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let fg = if !enabled {
        ui.visuals().weak_text_color()
    } else if selected {
        acc
    } else {
        w.fg_stroke.color
    };
    ui.painter().rect(
        rect,
        w.corner_radius,
        fill,
        if selected {
            egui::Stroke::new(1.0, acc)
        } else {
            egui::Stroke::NONE
        },
        egui::StrokeKind::Inside,
    );
    let ir = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + pad, rect.center().y - icon_sz / 2.0),
        Vec2::splat(icon_sz),
    );
    icon.paint(ui.painter(), ir, fg, ui.visuals().window_fill);
    if !label.is_empty() {
        ui.painter().text(
            egui::pos2(ir.max.x + gap, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            fg,
        );
    }
    if !tip.is_empty() {
        resp.clone().on_hover_text(tip);
    }
    enabled && resp.clicked()
}

/// A panel header: an accent bar, a bold title that truncates instead of
/// pushing the controls off-screen, and right-aligned actions.
fn panel_header(ui: &mut egui::Ui, title: &str, reserve: f32, actions: impl FnOnce(&mut egui::Ui)) {
    let row_h = 26.0;
    let full = ui.available_rect_before_wrap();
    let accent = accent(ui);
    let bar = egui::Rect::from_min_max(
        egui::pos2(full.min.x + 1.0, full.min.y + 5.0),
        egui::pos2(full.min.x + 4.0, full.min.y + row_h - 5.0),
    );
    ui.painter().rect_filled(bar, 1.5, accent);
    let title_w = (full.width() - reserve - 12.0).max(48.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.allocate_ui_with_layout(
            Vec2::new(title_w, row_h),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    Label::new(RichText::new(title).size(14.5).strong().color(accent))
                        .truncate(),
                )
                .on_hover_text(title);
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), actions);
    });
    ui.add_space(2.0);
    ui.separator();
}

/// A sub-section title inside a panel: a short accent bar, a bold name and an
/// optional count.
fn section_head(ui: &mut egui::Ui, title: &str, count: usize) {
    let accent = accent(ui);
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(full.min.x, full.min.y + 3.0),
            egui::pos2(full.min.x + 3.0, full.min.y + 17.0),
        ),
        1.5,
        accent,
    );
    ui.horizontal(|ui| {
        ui.add_space(7.0);
        ui.label(RichText::new(title).size(13.0).strong().color(accent));
        if count > 0 {
            ui.label(RichText::new(count.to_string()).size(11.5).weak());
        }
    });
    ui.add_space(3.0);
}

/// A card on the panel background: soft fill, hairline edge, rounded corners.
fn card(ui: &mut egui::Ui, fill: Color32, inner: impl FnOnce(&mut egui::Ui)) {
    let stroke = ui.style().visuals.widgets.noninteractive.bg_stroke.clone();
    Frame::new()
        .fill(fill)
        .corner_radius(8.0)
        .inner_margin(egui::Margin {
            left: 9,
            right: 9,
            top: 7,
            bottom: 7,
        })
        .stroke(stroke)
        .show(ui, inner);
}

/// A rounded tinted band around a cluster of controls, so the tool bars read as
/// groups rather than a row of loose widgets.
fn group(ui: &mut egui::Ui, inner: impl FnOnce(&mut egui::Ui)) {
    let s = ui.style();
    Frame::new()
        .fill(s.visuals.faint_bg_color)
        .stroke(s.visuals.widgets.noninteractive.bg_stroke.clone())
        .corner_radius(8.0)
        .inner_margin(egui::Margin {
            left: 6,
            right: 6,
            top: 2,
            bottom: 2,
        })
        .show(ui, inner);
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
