//! A small subset of CSS, enough to restyle a rendered Markdown document.
//!
//! Only element selectors (`body`, `h1`…`h6`, `p`, `li`, `a`, `code`, `pre`,
//! `blockquote`, `hr`) are honoured; class, id, pseudo and descendant selectors
//! are parsed and dropped, because the document model carries no class names.

use eframe::egui::Color32;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Len {
    Px(f32),
    Em(f32),
    Percent(f32),
}

impl Len {
    /// Resolve against the inherited font size (`em` base) and column width.
    pub fn px(self, em_base: f32, ref_px: f32) -> f32 {
        match self {
            Len::Px(v) => v,
            Len::Em(v) => v * em_base,
            Len::Percent(v) => v / 100.0 * ref_px,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Default)]
pub struct Rule {
    pub font_size: Option<Len>,
    pub font_family: Option<String>,
    pub color: Option<Color32>,
    pub background: Option<Color32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub line_height: Option<Len>,
    pub margin_top: Option<Len>,
    pub margin_bottom: Option<Len>,
    pub padding: Option<[Len; 4]>,
    pub border_left: Option<(f32, Color32)>,
    pub text_align: Option<TextAlign>,
    pub text_indent: Option<Len>,
    pub max_width: Option<Len>,
}

impl Rule {
    fn merge_over(&mut self, other: &Rule) {
        let Rule {
            font_size,
            font_family,
            color,
            background,
            bold,
            italic,
            underline,
            strike,
            line_height,
            margin_top,
            margin_bottom,
            padding,
            border_left,
            text_align,
            text_indent,
            max_width,
        } = other;
        *self = Rule {
            font_size: font_size.or(self.font_size),
            font_family: font_family.clone().or(self.font_family.clone()),
            color: color.or(self.color),
            background: background.or(self.background),
            bold: bold.or(self.bold),
            italic: italic.or(self.italic),
            underline: underline.or(self.underline),
            strike: strike.or(self.strike),
            line_height: line_height.or(self.line_height),
            margin_top: margin_top.or(self.margin_top),
            margin_bottom: margin_bottom.or(self.margin_bottom),
            padding: padding.or(self.padding),
            border_left: border_left.or(self.border_left),
            text_align: text_align.or(self.text_align),
            text_indent: text_indent.or(self.text_indent),
            max_width: max_width.or(self.max_width),
        };
    }
}

#[derive(Debug, Default)]
pub struct Sheet {
    rules: HashMap<String, Rule>,
    /// Order the selectors first appeared in, so later declarations win.
    order: Vec<String>,
    pub errors: Vec<String>,
}

impl Sheet {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Font keys the sheet names, so the reader can install them up front.
    /// `"mono"` is egui's built-in family and needs no file.
    pub fn fonts(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .order
            .iter()
            .filter_map(|k| self.rules.get(k))
            .filter_map(|r| r.font_family.as_deref())
            .filter_map(font_key)
            .filter(|k| *k != "mono")
            .map(|k| k.to_string())
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// Merged rule for `selectors`, with `body` as the inherited base.
    pub fn rule(&self, selectors: &[&str]) -> Rule {
        let mut out = self.own("body");
        for sel in selectors {
            out.merge_over(&self.own(sel));
        }
        out
    }

    /// Declarations written for exactly this selector, with nothing inherited.
    pub fn own(&self, sel: &str) -> Rule {
        self.rules.get(sel).cloned().unwrap_or_default()
    }

    fn insert(&mut self, sel: &str, decls: &str) {
        for name in sel.split(',') {
            let name = name.trim().to_ascii_lowercase();
            if !is_supported(&name) {
                continue;
            }
            if !self.rules.contains_key(&name) {
                self.order.push(name.clone());
            }
            let parsed = parse_decls(decls, &name, &mut self.errors);
            let entry = self.rules.entry(name).or_default();
            entry.merge_over(&parsed);
        }
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
pub fn parse(src: &str) -> Sheet {
    let mut sheet = Sheet::default();
    let text = strip_comments(src);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let open = match text[i..].find('{') {
            Some(o) => i + o,
            None => break,
        };
        let selector = text[i..open].trim();
        let close = match text[open..].find('}') {
            Some(c) => open + c,
            None => {
                sheet
                    .errors
                    .push(format!("选择器 “{selector}” 缺少右花括号"));
                break;
            }
        };
        if !selector.is_empty() && !selector.starts_with('@') {
            sheet.insert(selector, &text[open + 1..close]);
        }
        i = close + 1;
    }
    sheet
}

fn strip_comments(src: &str) -> String {
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

fn parse_decls(decls: &str, sel: &str, errors: &mut Vec<String>) -> Rule {
    let mut r = Rule::default();
    for decl in decls.split(';') {
        let Some(colon) = decl.find(':') else { continue };
        let prop = decl[..colon].trim().to_ascii_lowercase();
        let value = decl[colon + 1..].trim();
        if value.is_empty() {
            continue;
        }
        let lower = value.to_ascii_lowercase();
        match prop.as_str() {
            "font-size" => r.font_size = parse_len(value, sel, &prop, errors),
            "font-family" => r.font_family = Some(first_family(value)),
            "color" => r.color = parse_color(value),
            "background-color" | "background" => r.background = parse_color(value),
            "font-weight" => {
                r.bold = Some(matches!(lower.as_str(), "bold" | "bolder") || num(&lower) >= 600.0)
            }
            "font-style" => r.italic = Some(matches!(lower.as_str(), "italic" | "oblique")),
            "text-decoration" | "text-decoration-line" => {
                r.underline = Some(lower.contains("underline"));
                r.strike = Some(lower.contains("line-through"));
            }
            "line-height" => {
                // A bare number is a multiplier of the font size, not a length.
                r.line_height = match value.trim().parse::<f32>() {
                    Ok(m) if m > 0.0 => Some(Len::Em(m)),
                    _ => parse_len(value, sel, &prop, errors),
                }
            }
            "margin-top" => r.margin_top = parse_len(value, sel, &prop, errors),
            "margin-bottom" => r.margin_bottom = parse_len(value, sel, &prop, errors),
            "margin" => {
                if let Some(s) = sides(&lower) {
                    r.margin_top = Some(s[0]);
                    r.margin_bottom = Some(s[2]);
                }
            }
            "padding" => r.padding = padding_shorthand(&lower),
            "padding-top" | "padding-bottom" | "padding-left" | "padding-right" => {
                r.padding = apply_padding(r.padding, &prop, parse_len(value, sel, &prop, errors))
            }
            "border-left" => r.border_left = parse_border(value),
            "text-align" => {
                r.text_align = Some(match lower.as_str() {
                    "center" => TextAlign::Center,
                    "right" | "end" => TextAlign::Right,
                    _ => TextAlign::Left,
                })
            }
            "text-indent" => r.text_indent = parse_len(value, sel, &prop, errors),
            "max-width" => r.max_width = parse_len(value, sel, &prop, errors),
            _ => {}
        }
    }
    r
}

fn padding_shorthand(value: &str) -> Option<[Len; 4]> {
    sides(value)
}

/// Expand a 1-4 value CSS box shorthand into [top, right, bottom, left].
fn sides(value: &str) -> Option<[Len; 4]> {
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

fn apply_padding(cur: Option<[Len; 4]>, prop: &str, v: Option<Len>) -> Option<[Len; 4]> {
    let mut p = cur.unwrap_or([Len::Px(0.0); 4]);
    let Some(v) = v else { return cur };
    match prop {
        "padding-top" => p[0] = v,
        "padding-right" => p[1] = v,
        "padding-bottom" => p[2] = v,
        _ => p[3] = v,
    }
    Some(p)
}

fn parse_len(value: &str, sel: &str, prop: &str, errors: &mut Vec<String>) -> Option<Len> {
    match parse_len_raw(value) {
        Some(l) => Some(l),
        None => {
            errors.push(format!("“{sel} {prop}: {value}” 无法解析长度"));
            None
        }
    }
}

fn parse_len_raw(value: &str) -> Option<Len> {
    let v = value.trim().to_ascii_lowercase();
    if v == "auto" || v == "normal" || v.is_empty() {
        return None;
    }
    let (num, unit) = split_number(&v)?;
    Some(match unit {
        "" | "px" => Len::Px(num),
        "em" | "rem" => Len::Em(num),
        "pt" => Len::Px(num * 4.0 / 3.0),
        "%" => Len::Percent(num),
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

fn first_family(value: &str) -> String {
    for part in value.split(',') {
        let name = part.trim().trim_matches('"').trim_matches('\'').trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    String::new()
}

fn parse_border(value: &str) -> Option<(f32, Color32)> {
    let mut width = 1.0f32;
    let mut color = None;
    for part in value.split_whitespace() {
        if matches!(part.to_ascii_lowercase().as_str(), "solid" | "dashed" | "dotted" | "double") {
            continue;
        }
        if let Some(l) = parse_len_raw(part) {
            if let Len::Px(v) = l {
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
    if let Some(inner) =
        lower
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
        let a = parts.get(3).map(|x| (x * 255.0).clamp(0.0, 255.0) as u8).unwrap_or(255);
        return Some(Color32::from_rgba_unmultiplied(
            parts[0].clamp(0.0, 255.0) as u8,
            parts[1].clamp(0.0, 255.0) as u8,
            parts[2].clamp(0.0, 255.0) as u8,
            a,
        ));
    }
    NAMED.iter().find(|(n, _)| *n == lower.as_str()).map(|(_, c)| *c)
}

fn parse_hex(hex: &str) -> Option<Color32> {
    let expand = |s: &str| -> Option<String> {
        match s.len() {
            3 | 4 => Some(
                s.chars()
                    .flat_map(|c| [c, c])
                    .collect(),
            ),
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
    let f = family.trim().to_ascii_lowercase();
    let has = |needle: &str| f.contains(needle);
    if f == "monospace" || has("mono") || has("consol") || has("code") {
        return Some("mono");
    }
    if has("yahei") || has("微软雅黑") || has("msyh") || has("mingliu") {
        Some("msyh")
    } else if has("simsun") || has("宋体") || has("songti") || has("sun") {
        Some("simsun")
    } else if has("simhei") || has("黑体") || has("heiti") || has("hei") {
        Some("simhei")
    } else if has("kai") || has("楷体") {
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
        let s = parse(
            "/* heading */ h1, h2 { font-size: 2em; color: #333; }\n\
             .note { color: red; }\n\
             body { line-height: 1.8; font-family: '楷体', serif; }",
        );
        let h1 = s.rule(&["h1"]);
        assert!(matches!(h1.font_size, Some(Len::Em(2.0))));
        assert_eq!(h1.color, parse_color("#333"));
        let body = s.rule(&["body"]);
        assert!(matches!(body.line_height, Some(Len::Em(1.8))));
        assert_eq!(body.font_family.as_deref(), Some("楷体"));
        assert!(s.rules.get(".note").is_none());
    }

    #[test]
    fn reads_the_shipped_example_stylesheet() {
        let src = include_str!("../assets/reader-example.css");
        let s = parse(src);
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        let body = s.rule(&["body"]);
        assert!(matches!(body.line_height, Some(Len::Em(_))));
        let h2 = s.rule(&["h2"]);
        assert!(h2.border_left.is_some() && h2.background.is_some());
        assert!(matches!(h2.padding, Some([_, _, _, _])));
        let p = s.own("p");
        assert!(p.text_indent.is_some());
        assert_eq!(s.own("h1").text_align, Some(TextAlign::Center));
        assert!(!s.fonts().is_empty());
    }

    #[test]
    fn colors_and_shorthands() {
        assert_eq!(parse_color("#ff0000"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse_color("rgb(0, 255, 0)"), Some(Color32::from_rgb(0, 255, 0)));
        assert_eq!(parse_color("crimson"), Some(Color32::from_rgb(220, 20, 60)));
        let r = parse_decls("margin: 12px 0 4px; padding: 8px 10px;", "p", &mut Vec::new());
        assert!(matches!(r.margin_top, Some(Len::Px(12.0))));
        assert_eq!(r.padding, Some([Len::Px(8.0), Len::Px(10.0), Len::Px(8.0), Len::Px(10.0)]));
    }

    #[test]
    fn font_family_maps_to_installed_keys() {
        assert_eq!(font_key("'Microsoft YaHei', sans-serif"), Some("msyh"));
        assert_eq!(font_key("Consolas"), Some("mono"));
        assert_eq!(font_key("Georgia"), None);
    }
}
