use eframe::egui::{self, Color32};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Selectable CJK fonts: (key, label, candidate file paths).
const FONTS: &[(&str, &str, &[&str])] = &[
    (
        "msyh",
        "微软雅黑",
        &["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\msyh.ttf"],
    ),
    (
        "simsun",
        "宋体",
        &["C:\\Windows\\Fonts\\simsun.ttc", "C:\\Windows\\Fonts\\simsun.ttf"],
    ),
    ("simhei", "黑体", &["C:\\Windows\\Fonts\\simhei.ttf"]),
    ("simkai", "楷体", &["C:\\Windows\\Fonts\\simkai.ttf"]),
    ("simfang", "仿宋", &["C:\\Windows\\Fonts\\simfang.ttf"]),
    (
        "notocjk",
        "思源黑体",
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        ],
    ),
    (
        "pingfang",
        "苹方",
        &["/System/Library/Fonts/PingFang.ttc"],
    ),
];

pub struct FontManager {
    /// (key, label) for fonts whose file is present on this machine.
    pub available: Vec<(String, String)>,
    cache: HashMap<String, Arc<egui::FontData>>,
    applied: Option<String>,
    applied_extras: Vec<String>,
}

impl FontManager {
    pub fn new() -> Self {
        let available = FONTS
            .iter()
            .filter(|(_, _, paths)| paths.iter().any(|p| Path::new(p).exists()))
            .map(|(k, l, _)| (k.to_string(), l.to_string()))
            .collect();
        Self {
            available,
            cache: HashMap::new(),
            applied: None,
            applied_extras: Vec::new(),
        }
    }

    /// Whether a stylesheet may select this face on this machine.
    pub fn has(&self, key: &str) -> bool {
        self.available.iter().any(|(k, _)| k == key)
    }

    fn data(&mut self, key: &str) -> Option<Arc<egui::FontData>> {
        if let Some(d) = self.cache.get(key) {
            return Some(d.clone());
        }
        let (_, _, paths) = FONTS.iter().find(|(k, _, _)| *k == key)?;
        for p in paths.iter().map(Path::new) {
            if let Ok(bytes) = std::fs::read(p) {
                let d = Arc::new(egui::FontData::from_owned(bytes));
                self.cache.insert(key.to_string(), d.clone());
                return Some(d);
            }
        }
        None
    }

    /// Install `want` (or the first available font) as the UI font.
    ///
    /// `extra` names additional fonts to load — a stylesheet may ask for a
    /// face the user never picked, and egui can only render what is installed.
    /// Returns the key actually in use, or `None` when no CJK font exists.
    pub fn apply(&mut self, ctx: &egui::Context, want: &str, extra: &[String]) -> Option<String> {
        if self.available.is_empty() {
            return None;
        }
        let known = self
            .available
            .iter()
            .any(|(k, _)| k == want)
            .then(|| want.to_string())
            .unwrap_or_else(|| self.available[0].0.clone());
        let mut extras: Vec<String> = extra
            .iter()
            .filter(|k| **k != known && self.available.iter().any(|(a, _)| a == *k))
            .cloned()
            .collect();
        extras.sort();
        extras.dedup();
        if self.applied.as_deref() == Some(known.as_str())
            && self.applied_extras == extras
        {
            return Some(known);
        }
        // The preferred font is primary; any other installed one stays as a
        // glyph fallback so rare characters still render.
        let mut fallback = self.available[0].0.clone();
        if fallback == known {
            fallback = self
                .available
                .get(1)
                .map(|(k, _)| k.clone())
                .unwrap_or_else(|| known.clone());
        }
        let mut fonts = egui::FontDefinitions::default();
        let latin = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .and_then(|l| l.first().cloned());
        let mut keys = vec![known.clone()];
        if fallback != known {
            keys.push(fallback);
        }
        keys.extend(extras.iter().cloned());
        let mut loaded = Vec::new();
        for k in keys {
            if let Some(d) = self.data(&k) {
                fonts.font_data.insert(k.clone(), d);
                loaded.push(k);
            }
        }
        if loaded.is_empty() {
            return None;
        }
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let list = fonts.families.entry(family).or_default();
            // Keep the bundled Latin font ahead of the CJK faces: some Windows
            // CJK cmaps claim ASCII code points such as 'o' without offering a
            // usable glyph, and because the character is "present" egui never
            // falls back — the letter turns into a tofu box.
            let at = list.len().min(1);
            for k in loaded.iter().rev() {
                list.insert(at, k.clone());
            }
        }
        // A stylesheet selects a face by name, so each loaded font also gets a
        // family of its own. The bundled Latin font leads for the same reason
        // the comment above gives.
        for k in &loaded {
            let mut list = latin.iter().cloned().collect::<Vec<_>>();
            list.push(k.clone());
            fonts
                .families
                .insert(egui::FontFamily::Name(k.clone().into()), list);
        }
        ctx.set_fonts(fonts);
        self.applied = Some(known.clone());
        self.applied_extras = extras;
        Some(known)
    }
}

/// Reading-area palette derived from a user-chosen background color.
#[derive(Clone, Copy)]
pub struct ReadingTheme {
    pub fill: Color32,
    pub text: Color32,
    pub faint: Color32,
    pub extreme: Color32,
}

/// The one blue the whole UI points with: active tab, current chapter, headers.
pub fn accent(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(0x8F, 0xA7, 0xFF)
    } else {
        Color32::from_rgb(0x3B, 0x5B, 0xDB)
    }
}

/// A translucent wash of [`accent`], for hover and selection backgrounds.
pub fn accent_soft(dark: bool, alpha: u8) -> Color32 {
    let c = accent(dark);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

pub const BG_PRESETS: &[(&str, Option<u32>)] = &[
    ("跟随主题", None),
    ("米白", Some(0xF9F6EE)),
    ("纯白", Some(0xFFFFFF)),
    ("羊皮纸", Some(0xF2E8D2)),
    ("淡青", Some(0xE4EEF0)),
    ("豆沙绿", Some(0xC9E3CC)),
    ("浅灰", Some(0xE8E8E8)),
    ("深灰", Some(0x2B2B2B)),
    ("夜黑", Some(0x141414)),
];

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

pub fn reading_theme(bg: Option<u32>) -> Option<ReadingTheme> {
    let v = bg?;
    let rgb = ((v >> 16) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8);
    Some(theme_from_paper(Color32::from_rgb(rgb.0, rgb.1, rgb.2), None, false))
}

/// Reading palette for a paper colour that came from a stylesheet.
///
/// The sheet's own ink is only used when it actually contrasts with its paper;
/// a colour that has drifted near the background (an unreadable `#F5F5F5` on
/// `#F2EBD9`, say) is dropped in favour of one derived from the paper itself.
pub fn theme_from_paper(fill: Color32, ink: Option<Color32>, ink_set: bool) -> ReadingTheme {
    let a = fill.to_array();
    let rgb = (a[0], a[1], a[2]);
    let lum = luminance(rgb);
    let light = lum > 140.0;
    let fallback = if light {
        (0x24, 0x22, 0x1E)
    } else {
        (0xD8, 0xD6, 0xD0)
    };
    let soft = if light {
        (0x8A, 0x86, 0x7C)
    } else {
        (0x96, 0x94, 0x8E)
    };
    let asked = ink.map(|c| {
        let i = c.to_array();
        (i[0], i[1], i[2])
    });
    let text = match asked {
        Some(c) if ink_set && (luminance(c) - lum).abs() >= 60.0 => c,
        _ => fallback,
    };
    ReadingTheme {
        fill,
        text: Color32::from_rgb(text.0, text.1, text.2),
        faint: {
            let c = mix(rgb, soft, 0.18);
            Color32::from_rgb(c.0, c.1, c.2)
        },
        extreme: {
            let c = mix(rgb, text, 0.08);
            Color32::from_rgb(c.0, c.1, c.2)
        },
    }
}

fn luminance(rgb: (u8, u8, u8)) -> f32 {
    0.299 * rgb.0 as f32 + 0.587 * rgb.1 as f32 + 0.114 * rgb.2 as f32
}
