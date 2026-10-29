//! The stylesheets shipped with the reader.
//!
//! Each one follows a well-known typographic model — GitHub's document CSS,
//! Tufte's book pages, Chinese book typography, parchment, the green
//! eye-protection paper and an editor theme — and carries a night palette in a
//! `prefers-color-scheme` block, so one sheet covers both themes.

use super::{Sheet, parse};

pub struct Preset {
    pub name: &'static str,
    /// One-line description of the look, for the docs.
    #[allow(dead_code)]
    pub hint: &'static str,
    css: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "文档",
        hint: "GitHub 文档风：灰线标题、斑马纹代码块",
        css: include_str!("../../assets/preset-doc.css"),
    },
    Preset {
        name: "书籍",
        hint: "Tufte 书页：象牙纸、衬线大字、蓝色引文条",
        css: include_str!("../../assets/preset-book.css"),
    },
    Preset {
        name: "中文书版",
        hint: "中文排版：首行缩进两字、两端对齐、避头尾",
        css: include_str!("../../assets/preset-cn.css"),
    },
    Preset {
        name: "羊皮纸",
        hint: "复古暖纸：楷体、赭石标题、深描边引言块",
        css: include_str!("../../assets/preset-parchment.css"),
    },
    Preset {
        name: "护眼绿",
        hint: "豆沙绿纸：18px 大字、行距 2.0",
        css: include_str!("../../assets/preset-eye.css"),
    },
    Preset {
        name: "编辑器",
        hint: "代码风：等宽字体、深蓝底、常驻深色",
        css: include_str!("../../assets/preset-code.css"),
    },
];

pub fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// The sheet for a built-in style name; an unknown name falls back to nothing.
pub fn sheet(name: &str) -> Option<(Sheet, Vec<String>)> {
    find(name).map(|p| parse(p.css))
}

#[cfg(test)]
mod tests {
    use super::super::{LenUnit, TextAlign};
    use super::*;

    #[test]
    fn every_preset_parses_and_has_both_papers() {
        for p in PRESETS {
            let (s, errors) = parse(p.css);
            assert!(errors.is_empty(), "{}: {:?}", p.name, errors);
            let body = s.own("body");
            assert!(body.font_size().is_some(), "{}: no body size", p.name);
            assert!(
                body.line_height().is_some(),
                "{}: no body line-height",
                p.name
            );
            let dark = s.with_theme(true);
            let light = s.with_theme(false);
            assert!(!dark.is_empty() && !light.is_empty(), "{}", p.name);
            if p.name != "编辑器" {
                // A night palette that never changes its paper is not a night palette.
                assert_ne!(
                    dark.own("body").background(),
                    light.own("body").background(),
                    "{}: dark paper equals light paper",
                    p.name
                );
            }
        }
    }

    #[test]
    fn chinese_preset_indents_and_justifies() {
        let (s, _) = sheet("中文书版").unwrap();
        assert_eq!(s.own("p").text_indent(), Some(LenUnit::Em(2.0)));
        assert_eq!(s.own("p").text_align(), Some(TextAlign::Left));
        assert!(s.fonts().contains(&"simsun".to_string()));
    }

    #[test]
    fn presets_are_named_and_described() {
        assert_eq!(PRESETS.len(), 6);
        for p in PRESETS {
            assert!(!p.name.is_empty() && !p.hint.is_empty(), "{}", p.name);
            assert!(find(p.name).is_some(), "{} not reachable", p.name);
        }
    }

    #[test]
    fn unknown_name_is_not_a_preset() {
        assert!(sheet("不存在的样式").is_none());
    }
}
