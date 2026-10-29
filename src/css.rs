//! A small subset of CSS, enough to restyle a rendered Markdown document.
//!
//! Only element selectors (`body`, `h1`…`h6`, `p`, `li`, `a`, `code`, `pre`,
//! `blockquote`, `hr`) are honoured; class, id, pseudo and descendant selectors
//! are parsed and dropped, because the document model carries no class names.
//! `@media (prefers-color-scheme: …)` blocks are understood, so one stylesheet can
//! carry both a day and a night palette.

use eframe::egui::Color32;

pub mod presets;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LenUnit {
    Px(f32),
    Em(f32),
    Percent(f32),
}

impl LenUnit {
    /// Resolve against the inherited font size (`em` base) and column width.
    pub fn px(self, em_base: f32, ref_px: f32) -> f32 {
        match self {
            LenUnit::Px(v) => v,
            LenUnit::Em(v) => v * em_base,
            LenUnit::Percent(v) => v / 100.0 * ref_px,
        }
    }
}

/// Which paper a block of declarations is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Any,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    /// `justify` — Chinese book layouts ask for it; the reader keeps the left edge.
    Justify,
}

/// A single declaration, kept as an (name, value) pair so the ordering of a
/// stylesheet survives into the merged rule instead of being flattened away.
#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    FontSize(LenUnit),
    FontFamily(String),
    Color(Color32),
    Background(Color32),
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Strike(bool),
    LineHeight(LenUnit),
    MarginTop(LenUnit),
    MarginBottom(LenUnit),
    Padding([LenUnit; 4]),
    BorderLeft(f32, Color32),
    TextAlign(TextAlign),
    TextIndent(LenUnit),
    MaxWidth(LenUnit),
}

/// Declarations for one selector, in source order; later ones win.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rule(pub Vec<Decl>);

impl Rule {
    fn push_all(&mut self, decls: Vec<Decl>) {
        for d in decls {
            self.evict(&d);
            self.0.push(d);
        }
    }

    /// Drop earlier declarations of the same property, as CSS does.
    fn evict(&mut self, d: &Decl) {
        self.0.retain(|old| key(old) != key(d));
    }
}

fn key(d: &Decl) -> u8 {
    match d {
        Decl::FontSize(_) => 0,
        Decl::FontFamily(_) => 1,
        Decl::Color(_) => 2,
        Decl::Background(_) => 3,
        Decl::Bold(_) => 4,
        Decl::Italic(_) => 5,
        Decl::Underline(_) => 6,
        Decl::Strike(_) => 7,
        Decl::LineHeight(_) => 8,
        Decl::MarginTop(_) => 9,
        Decl::MarginBottom(_) => 10,
        Decl::Padding(_) => 11,
        Decl::BorderLeft(_, _) => 12,
        Decl::TextAlign(_) => 13,
        Decl::TextIndent(_) => 14,
        Decl::MaxWidth(_) => 15,
    }
}

/// A stylesheet: declarations grouped by selector, in source order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet(pub Vec<(String, Vec<(Scheme, Rule)>)>);

impl Sheet {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Font keys the sheet names, so the reader can install them up front.
    /// `"mono"` is egui's built-in family and needs no file.
    pub fn fonts(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .iter()
            .filter_map(|r| r.font_family())
            .filter_map(|name| css_font_key(&name))
            .filter(|k| *k != "mono")
            .map(|k| k.to_string())
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    fn iter(&self) -> impl Iterator<Item = &Rule> {
        self.0.iter().flat_map(|(_, blocks)| blocks.iter().map(|(_, r)| r))
    }

    fn push(&mut self, sel: &str, scheme: Scheme, decls: &[Decl]) {
        if decls.is_empty() {
            return;
        }
        let owned: Vec<Decl> = decls.to_vec();
        let entry = self.0.iter_mut().find(|(s, _)| s == sel);
        match entry {
            Some((_, blocks)) => match blocks.iter_mut().find(|(s, _)| *s == scheme) {
                Some((_, rule)) => rule.push_all(owned),
                None => blocks.push((scheme, Rule(owned))),
            },
            None => self.0.push((sel.to_string(), vec![(scheme, Rule(owned))])),
        }
    }

    /// The sheet as it applies to one paper colour scheme: light or dark.
    pub fn with_theme(&self, dark: bool) -> Sheet {
        let want = if dark { Scheme::Dark } else { Scheme::Light };
        let mut out = Sheet::default();
        for (sel, blocks) in &self.0 {
            let mut decls = Vec::new();
            for (scheme, rule) in blocks {
                if *scheme == Scheme::Any || *scheme == want {
                    decls.extend(rule.0.iter().cloned());
                }
            }
            if !decls.is_empty() {
                out.0.push((sel.clone(), vec![(Scheme::Any, Rule(decls))]));
            }
        }
        out
    }

    /// Merged rule for `selectors`, with `body` as the inherited base.
    pub fn rule(&self, selectors: &[&str]) -> Rule {
        rule(self, selectors)
    }

    /// Declarations written for exactly this selector, with nothing inherited.
    pub fn own(&self, sel: &str) -> Rule {
        own(self, sel)
    }
}

impl Rule {
    /// The last declaration of `kind` wins, as it would in a browser.
    fn last(&self, kind: u8) -> Option<&Decl> {
        self.0.iter().rev().find(|d| key(d) == kind)
    }
}

impl Rule {
    pub fn font_size(&self) -> Option<LenUnit> {
        len(self, 0)
    }
    pub fn font_family(&self) -> Option<String> {
        match self.last(1) {
            Some(Decl::FontFamily(s)) => Some(s.clone()),
            _ => None,
        }
    }
    pub fn color(&self) -> Option<Color32> {
        col(self, 2)
    }
    pub fn background(&self) -> Option<Color32> {
        col(self, 3)
    }
    pub fn bold(&self) -> Option<bool> {
        flag(self, 4)
    }
    pub fn italic(&self) -> Option<bool> {
        flag(self, 5)
    }
    pub fn underline(&self) -> Option<bool> {
        flag(self, 6)
    }
    pub fn strike(&self) -> Option<bool> {
        flag(self, 7)
    }
    pub fn line_height(&self) -> Option<LenUnit> {
        len(self, 8)
    }
    pub fn margin_top(&self) -> Option<LenUnit> {
        len(self, 9)
    }
    pub fn margin_bottom(&self) -> Option<LenUnit> {
        len(self, 10)
    }
    pub fn padding(&self) -> Option<[LenUnit; 4]> {
        match self.last(11) {
            Some(Decl::Padding(p)) => Some(*p),
            _ => None,
        }
    }
    pub fn border_left(&self) -> Option<(f32, Color32)> {
        match self.last(12) {
            Some(Decl::BorderLeft(w, c)) => Some((*w, *c)),
            _ => None,
        }
    }
    pub fn text_align(&self) -> Option<TextAlign> {
        match self.last(13) {
            Some(Decl::TextAlign(a)) => Some(*a),
            _ => None,
        }
    }
    pub fn text_indent(&self) -> Option<LenUnit> {
        len(self, 14)
    }
    pub fn max_width(&self) -> Option<LenUnit> {
        len(self, 15)
    }
}

fn len(r: &Rule, k: u8) -> Option<LenUnit> {
    match r.last(k) {
        Some(Decl::FontSize(l))
        | Some(Decl::LineHeight(l))
        | Some(Decl::MarginTop(l))
        | Some(Decl::MarginBottom(l))
        | Some(Decl::TextIndent(l))
        | Some(Decl::MaxWidth(l)) => Some(*l),
        _ => None,
    }
}

fn col(r: &Rule, k: u8) -> Option<Color32> {
    match r.last(k) {
        Some(Decl::Color(c)) | Some(Decl::Background(c)) => Some(*c),
        _ => None,
    }
}

fn flag(r: &Rule, k: u8) -> Option<bool> {
    match r.last(k) {
        Some(Decl::Bold(b)) | Some(Decl::Italic(b)) | Some(Decl::Underline(b))
        | Some(Decl::Strike(b)) => Some(*b),
        _ => None,
    }
}

/// The merged view the reader used to get from a HashMap: `body` as the inherited
/// base, then the requested selectors on top.
pub fn rule(sheet: &Sheet, selectors: &[&str]) -> Rule {
    let mut decls: Vec<Decl> = own(sheet, "body").0;
    for sel in selectors {
        decls.extend(own(sheet, sel).0);
    }
    Rule(decls)
}

/// Declarations written for exactly this selector, with nothing inherited.
///
/// A sheet that has not been projected onto a theme yet only answers with the
/// declarations that hold for every paper; day- and night-specific blocks are
/// picked up by [`Sheet::with_theme`].
pub fn own(sheet: &Sheet, sel: &str) -> Rule {
    let needle = sel.to_ascii_lowercase();
    match sheet.0.iter().find(|(s, _)| *s == needle) {
        Some((_, blocks)) => {
            let mut out = Vec::new();
            for (scheme, rule) in blocks {
                if *scheme == Scheme::Any {
                    out.extend(rule.0.iter().cloned());
                }
            }
            Rule(out)
        }
        None => Rule::default(),
    }
}

fn is_supported(sel: &str) -> bool {
    matches!(
        sel,
        "body"
            | "html"
            | "p"
            | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "ul"
            | "ol"
            | "a"
            | "code"
            | "pre"
            | "blockquote"
            | "hr"
            | "img"
            | "strong"
            | "em"
    )
}

/// Parse a stylesheet; unsupported selectors are skipped, never fatal.
/// The second value lists the declarations that could not be understood.
pub fn parse(src: &str) -> (Sheet, Vec<String>) {
    let text = strip_comments(src);
    let mut sheet = Sheet::default();
    let mut errors = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let open = match text[i..].find('{') {
            Some(o) => i + o,
            None => break,
        };
        let header = text[i..open].trim();
        let close = match find_close(&text[open..]) {
            Some(c) => open + c,
            None => {
                errors.push(format!("选择器 “{header}” 缺少右花括号"));
                break;
            }
        };
        if header.starts_with('@') {
            let body = &text[open + 1..close];
            if let Some(scheme) = color_scheme(header) {
                // The rules inside carry their own selectors, so they parse as
                // ordinary blocks once the scheme is known.
                parse_into(body, scheme, &mut sheet, &mut errors);
            }
        } else if !header.is_empty() {
            let sels: Vec<String> = header
                .split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| is_supported(s))
                .collect();
            if !sels.is_empty() {
                let decls = parse_decls(&text[open + 1..close], header, &mut errors);
                for sel in sels {
                    sheet.push(&sel, Scheme::Any, &decls);
                }
            }
        }
        i = close + 1;
    }
    (sheet, errors)
}

fn parse_into(body: &str, scheme: Scheme, sheet: &mut Sheet, errors: &mut Vec<String>) {
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let open = match body[i..].find('{') {
            Some(o) => i + o,
            None => break,
        };
        let header = body[i..open].trim();
        let close = match find_close(&body[open..]) {
            Some(c) => open + c,
            None => {
                errors.push(format!("选择器 “{header}” 缺少右花括号"));
                break;
            }
        };
        if !header.starts_with('@') && !header.is_empty() {
            let sels: Vec<String> = header
                .split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| is_supported(s))
                .collect();
            if !sels.is_empty() {
                let decls = parse_decls(&body[open + 1..close], &sels[0], errors);
                for sel in sels {
                    sheet.push(&sel, scheme, &decls);
                }
            }
        }
        i = close + 1;
    }
}

/// Index of the `}` that closes the block whose `{` starts `text`.
fn find_close(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, b) in text.bytes().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

/// `prefers-color-scheme` inside an `@media` header, if that is what it asks for.
fn color_scheme(header: &str) -> Option<Scheme> {
    let h = header.to_ascii_lowercase();
    if !h.contains("prefers-color-scheme") {
        return None;
    }
    if h.contains("dark") {
        Some(Scheme::Dark)
    } else if h.contains("light") {
        Some(Scheme::Light)
    } else {
        None
    }
}

pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_decls(decls: &str, sel: &str, errors: &mut Vec<String>) -> Vec<Decl> {
    let mut out = Vec::new();
    for decl in decls.split(';') {
        let Some(colon) = decl.find(':') else { continue };
        let prop = decl[..colon].trim().to_ascii_lowercase();
        let value = decl[colon + 1..].trim();
        if value.is_empty() {
            continue;
        }
        let lower = value.to_ascii_lowercase();
        match prop.as_str() {
            "font-size" => out.extend(parse_len(value, sel, &prop, errors).map(Decl::FontSize)),
            "font-family" => out.push(Decl::FontFamily(first_family(value))),
            "color" => out.extend(parse_color(value).map(Decl::Color)),
            "background-color" | "background" => out.extend(parse_color(value).map(Decl::Background)),
            "font-weight" => {
                out.push(Decl::Bold(matches!(lower.as_str(), "bold" | "bolder") || num(&lower) >= 600.0))
            }
            "font-style" => out.push(Decl::Italic(matches!(lower.as_str(), "italic" | "oblique"))),
            "text-decoration" | "text-decoration-line" => {
                out.push(Decl::Underline(lower.contains("underline")));
                out.push(Decl::Strike(lower.contains("line-through")));
            }
            "line-height" => {
                // A bare number is a multiplier of the font size, not a length.
                let l = match value.trim().parse::<f32>() {
                    Ok(m) if m > 0.0 => Some(LenUnit::Em(m)),
                    _ => parse_len(value, sel, &prop, errors),
                };
                out.extend(l.map(Decl::LineHeight));
            }
            "margin-top" => out.extend(parse_len(value, sel, &prop, errors).map(Decl::MarginTop)),
            "margin-bottom" => {
                out.extend(parse_len(value, sel, &prop, errors).map(Decl::MarginBottom))
            }
            "margin" => {
                if let Some(s) = sides(&lower) {
                    out.push(Decl::MarginTop(s[0]));
                    out.push(Decl::MarginBottom(s[2]));
                }
            }
            "padding" => out.extend(sides(&lower).map(Decl::Padding)),
            "border-left" => out.extend(parse_border(value).map(|(w, c)| Decl::BorderLeft(w, c))),
            "text-align" => {
                out.push(Decl::TextAlign(match lower.as_str() {
                    "center" => TextAlign::Center,
                    "right" | "end" => TextAlign::Right,
                    "justify" => TextAlign::Justify,
                    _ => TextAlign::Left,
                }))
            }
            "text-indent" => out.extend(parse_len(value, sel, &prop, errors).map(Decl::TextIndent)),
            "max-width" => out.extend(parse_len(value, sel, &prop, errors).map(Decl::MaxWidth)),
            _ => {}
        }
    }
    out
}

/// Expand a 1-4 value CSS box shorthand into [top, right, bottom, left].
fn sides(value: &str) -> Option<[LenUnit; 4]> {
    let mut vals = Vec::new();
    for part in value.split_whitespace() {
        vals.push(parse_len_raw(part)?);
    }
    Some(match vals.len() {
        1 => [vals[0], vals[0], vals[0], vals[0]],
        2 => [vals[0], vals[1], vals[0], vals[1]],
        3 => [vals[0], vals[1], vals[2], vals[1]],
        4 => [vals[0], vals[1], vals[2], vals[3]],
        _ => return None,
    })
}

fn parse_len(value: &str, sel: &str, prop: &str, errors: &mut Vec<String>) -> Option<LenUnit> {
    match parse_len_raw(value) {
        Some(l) => Some(l),
        None => {
            errors.push(format!("“{sel} {prop}: {value}” 无法解析长度"));
            None
        }
    }
}

fn parse_len_raw(value: &str) -> Option<LenUnit> {
    let v = value.trim().to_ascii_lowercase();
    if v == "auto" || v == "normal" || v.is_empty() {
        return None;
    }
    let (num, unit) = split_number(&v)?;
    Some(match unit {
        "" | "px" => LenUnit::Px(num),
        "em" | "rem" => LenUnit::Em(num),
        "pt" => LenUnit::Px(num * 4.0 / 3.0),
        "%" => LenUnit::Percent(num),
        _ => return None,
    })
}

fn split_number(v: &str) -> Option<(f32, &str)> {
    let end = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(v.len());
    if end == 0 {
        return None;
    }
    let num: f32 = v[..end].parse().ok()?;
    Some((num, v[end..].trim()))
}

fn num(value: &str) -> f32 {
    value.trim().parse().unwrap_or(0.0)
}

/// The first named face in a `font-family` list; generic keywords are skipped,
/// because they name no file the reader could install.
fn first_family(value: &str) -> String {
    const GENERIC: &[&str] = &[
        "serif",
        "sans-serif",
        "monospace",
        "cursive",
        "fantasy",
        "system-ui",
        "ui-monospace",
        "-apple-system",
        "blinkmacsystemfont",
    ];
    for part in value.split(',') {
        let name = part.trim().trim_matches('"').trim_matches('\'').trim();
        if !name.is_empty() && !GENERIC.contains(&name.to_ascii_lowercase().as_str()) {
            return name.to_string();
        }
    }
    String::new()
}

fn parse_border(value: &str) -> Option<(f32, Color32)> {
    let mut width = 1.0f32;
    let mut color = None;
    for part in value.split_whitespace() {
        if matches!(
            part.to_ascii_lowercase().as_str(),
            "solid" | "dashed" | "dotted" | "double"
        ) {
            continue;
        }
        if let Some(l) = parse_len_raw(part) {
            if let LenUnit::Px(v) = l {
                width = v;
            }
        } else if let Some(c) = parse_color(part) {
            color = Some(c);
        }
    }
    Some((width, color.unwrap_or(Color32::from_gray(140))))
}

pub fn parse_color(value: &str) -> Option<Color32> {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if lower == "transparent" {
        return Some(Color32::TRANSPARENT);
    }
    if let Some(hex) = lower.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(inner) = lower
        .strip_prefix("rgb(")
        .or(lower.strip_prefix("rgba("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<f32> = inner
            .split(|c| c == ',' || c == '/')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if parts.len() < 3 {
            return None;
        }
        let a = parts
            .get(3)
            .map(|x| (x * 255.0).clamp(0.0, 255.0) as u8)
            .unwrap_or(255);
        return Some(Color32::from_rgba_unmultiplied(
            parts[0].clamp(0.0, 255.0) as u8,
            parts[1].clamp(0.0, 255.0) as u8,
            parts[2].clamp(0.0, 255.0) as u8,
            a,
        ));
    }
    NAMED
        .iter()
        .find(|(n, _)| *n == lower.as_str())
        .map(|(_, c)| *c)
}

fn parse_hex(hex: &str) -> Option<Color32> {
    let expand = |s: &str| -> Option<String> {
        match s.len() {
            3 | 4 => Some(s.chars().flat_map(|c| [c, c]).collect()),
            6 | 8 => Some(s.to_string()),
            _ => None,
        }
    };
    let v = expand(hex)?;
    let byte = |i: usize| u8::from_str_radix(&v[i * 2..i * 2 + 2], 16).ok();
    let (r, g, b) = (byte(0)?, byte(1)?, byte(2)?);
    let a = if v.len() == 8 { byte(3)? } else { 255 };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

const NAMED: &[(&str, Color32)] = &[
    ("black", Color32::BLACK),
    ("white", Color32::WHITE),
    ("red", Color32::from_rgb(255, 0, 0)),
    ("green", Color32::from_rgb(0, 128, 0)),
    ("blue", Color32::from_rgb(0, 0, 255)),
    ("gray", Color32::from_rgb(128, 128, 128)),
    ("grey", Color32::from_rgb(128, 128, 128)),
    ("lightgray", Color32::from_rgb(211, 211, 211)),
    ("darkgray", Color32::from_rgb(169, 169, 169)),
    ("dimgray", Color32::from_rgb(105, 105, 105)),
    ("orange", Color32::from_rgb(255, 165, 0)),
    ("yellow", Color32::from_rgb(255, 255, 0)),
    ("purple", Color32::from_rgb(128, 0, 128)),
    ("pink", Color32::from_rgb(255, 192, 203)),
    ("brown", Color32::from_rgb(165, 42, 42)),
    ("navy", Color32::from_rgb(0, 0, 128)),
    ("teal", Color32::from_rgb(0, 128, 128)),
    ("maroon", Color32::from_rgb(128, 0, 0)),
    ("olive", Color32::from_rgb(128, 128, 0)),
    ("silver", Color32::from_rgb(192, 192, 192)),
    ("lime", Color32::from_rgb(0, 255, 0)),
    ("aqua", Color32::from_rgb(0, 255, 255)),
    ("cyan", Color32::from_rgb(0, 255, 255)),
    ("magenta", Color32::from_rgb(255, 0, 255)),
    ("gold", Color32::from_rgb(255, 215, 0)),
    ("coral", Color32::from_rgb(255, 127, 80)),
    ("crimson", Color32::from_rgb(220, 20, 60)),
    ("indigo", Color32::from_rgb(75, 0, 130)),
    ("violet", Color32::from_rgb(238, 130, 238)),
    ("skyblue", Color32::from_rgb(135, 206, 235)),
    ("steelblue", Color32::from_rgb(70, 130, 180)),
    ("slategray", Color32::from_rgb(112, 128, 144)),
    ("forestgreen", Color32::from_rgb(34, 139, 34)),
    ("darkorange", Color32::from_rgb(255, 140, 0)),
    ("royalblue", Color32::from_rgb(65, 105, 225)),
    ("dodgerblue", Color32::from_rgb(30, 144, 255)),
    ("beige", Color32::from_rgb(245, 245, 220)),
    ("ivory", Color32::from_rgb(255, 255, 240)),
    ("whitesmoke", Color32::from_rgb(245, 245, 245)),
    ("tan", Color32::from_rgb(210, 180, 140)),
];

/// Map a CSS `font-family` name onto a font key the reader can load from disk.
pub fn font_key(family: &str) -> Option<&'static str> {
    css_font_key(family)
}

fn css_font_key(family: &str) -> Option<&'static str> {
    let f = family.trim().to_ascii_lowercase();
    let has = |needle: &str| f.contains(needle);
    if f == "monospace" || has("mono") || has("consol") || has("code") {
        return Some("mono");
    }
    if has("yahei") || has("微软雅黑") || has("msyh") || has("mingliu") {
        Some("msyh")
    } else if has("simsun") || has("宋体") || has("songti") || has("sun") {
        Some("simsun")
    } else if has("simhei") || has("黑体") || has("heiti") {
        Some("simhei")
    } else if has("kai") || has("楷体") || has("simkai") {
        Some("simkai")
    } else if has("fang") || has("仿宋") {
        Some("simfang")
    } else if has("noto") || has("思源") || has("cjk") {
        Some("notocjk")
    } else if has("pingfang") || has("苹方") {
        Some("pingfang")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_and_skips_unknown_selectors() {
        let (s, errors) = parse(
            "/* heading */ h1, h2 { font-size: 2em; color: #333; }\n\
             .note { color: red; }\n\
             body { line-height: 1.8; font-family: '楷体', serif; }",
        );
        assert!(errors.is_empty(), "{errors:?}");
        let h1 = rule(&s, &["h1"]);
        assert_eq!(h1.font_size(), Some(LenUnit::Em(2.0)));
        assert_eq!(h1.color(), parse_color("#333"));
        let body = rule(&s, &["body"]);
        assert_eq!(body.line_height(), Some(LenUnit::Em(1.8)));
        assert_eq!(body.font_family().as_deref(), Some("楷体"));
        assert!(own(&s, ".note").0.is_empty());
    }

    #[test]
    fn media_queries_split_the_palette() {
        let (s, errors) = parse(
            "body { background: #FBFAF7; }\n\
             @media (prefers-color-scheme: dark) {\n  body { background: #1F1B16; color: #D8D2C4; }\n}\n",
        );
        assert!(errors.is_empty(), "{errors:?}");
        let light = s.with_theme(false);
        let dark = s.with_theme(true);
        assert_eq!(own(&light, "body").background(), parse_color("#FBFAF7"));
        assert!(own(&light, "body").color().is_none());
        assert_eq!(own(&dark, "body").background(), parse_color("#1F1B16"));
        assert_eq!(own(&dark, "body").color(), parse_color("#D8D2C4"));
        // An unprojected sheet keeps the blocks that hold for every paper, and
        // stays silent about the night ones.
        assert_eq!(rule(&s, &["body"]).background(), parse_color("#FBFAF7"));
        assert!(rule(&s, &["body"]).color().is_none());
    }

    #[test]
    fn reads_the_shipped_example_stylesheet() {
        let src = include_str!("../assets/reader-example.css");
        let (s, errors) = parse(src);
        assert!(errors.is_empty(), "{errors:?}");
        let body = rule(&s, &["body"]);
        assert!(body.line_height().is_some());
        let h2 = rule(&s, &["h2"]);
        assert!(h2.border_left().is_some() && h2.background().is_some());
        assert!(matches!(h2.padding(), Some([_, _, _, _])));
        assert!(own(&s, "p").text_indent().is_some());
        assert_eq!(own(&s, "h1").text_align(), Some(TextAlign::Center));
        assert!(!s.fonts().is_empty());
    }

    #[test]
    fn colors_and_shorthands() {
        assert_eq!(parse_color("#ff0000"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse_color("rgb(0, 255, 0)"), Some(Color32::from_rgb(0, 255, 0)));
        assert_eq!(parse_color("crimson"), Some(Color32::from_rgb(220, 20, 60)));
        let r = Rule(parse_decls(
            "margin: 12px 0 4px; padding: 8px 10px;",
            "p",
            &mut Vec::new(),
        ));
        assert_eq!(r.margin_top(), Some(LenUnit::Px(12.0)));
        assert_eq!(
            r.padding(),
            Some([
                LenUnit::Px(8.0),
                LenUnit::Px(10.0),
                LenUnit::Px(8.0),
                LenUnit::Px(10.0)
            ])
        );
    }

    #[test]
    fn font_family_maps_to_installed_keys() {
        assert_eq!(font_key("'Microsoft YaHei', sans-serif"), Some("msyh"));
        assert_eq!(font_key("Consolas"), Some("mono"));
        assert_eq!(font_key("Georgia"), None);
    }
}
