//! A small, dependency-free tokenizer for fenced code blocks.
//!
//! Books in this reader mix C, C++, C#, Python, shell and JSON samples, so the
//! scanner is language-parameterised rather than hard-coded to one grammar. It
//! is deliberately forgiving: an unknown construct is emitted as plain text
//! instead of breaking the rest of the block.

use eframe::egui::text::LayoutJob;
use eframe::egui::{Color32, FontId, TextFormat};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tok {
    Text,
    Keyword,
    Type,
    String,
    Number,
    Comment,
    Func,
    Preproc,
}

struct Lang {
    /// Keywords, lowercase-compared.
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    /// `#` opens a comment (scripting languages).
    hash_comment: bool,
    /// `#` opens a directive (C preprocessor).
    preproc: bool,
    backtick_string: bool,
}

const DEFAULT: Lang = Lang {
    keywords: &[],
    types: &[],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: false,
    backtick_string: false,
};

const C: Lang = Lang {
    keywords: &[
        "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern",
        "for", "goto", "if", "register", "return", "sizeof", "static", "struct", "switch", "typedef",
        "union", "volatile", "while", "inline", "restrict",
    ],
    types: &[
        "char", "double", "float", "int", "long", "short", "signed", "unsigned", "void", "size_t",
        "FILE",
    ],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: true,
    backtick_string: false,
};

const CPP: Lang = Lang {
    keywords: &[
        "alignas", "alignof", "and", "asm", "auto", "break", "case", "catch", "class", "const",
        "consteval", "constexpr", "constinit", "continue", "co_await", "co_return", "co_yield",
        "decltype", "default", "delete", "do", "dynamic_cast", "else", "enum", "explicit", "export",
        "extern", "false", "final", "for", "friend", "goto", "if", "import", "inline", "mutable",
        "namespace", "new", "noexcept", "not", "operator", "or", "override", "private", "protected",
        "public", "register", "reinterpret_cast", "requires", "return", "sizeof", "static",
        "static_assert", "static_cast", "struct", "switch", "template", "this", "thread_local",
        "throw", "true", "try", "typedef", "typename", "union", "using", "virtual", "volatile",
        "while",
    ],
    types: &[
        "bool", "char", "char8_t", "char16_t", "char32_t", "double", "float", "int", "int8_t",
        "int16_t", "int32_t", "int64_t", "long", "short", "signed", "size_t", "std", "string",
        "uint8_t", "uint16_t", "uint32_t", "uint64_t", "unsigned", "void", "wchar_t",
    ],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: true,
    backtick_string: false,
};

const CS: Lang = Lang {
    keywords: &[
        "abstract", "as", "async", "await", "base", "break", "checked", "class", "const",
        "continue", "default", "delegate", "do", "dynamic", "else", "enum", "event", "explicit",
        "extern", "false", "finally", "fixed", "for", "foreach", "get", "goto", "if", "implicit",
        "in", "interface", "internal", "is", "lock", "namespace", "new", "null", "operator", "out",
        "override", "params", "partial", "private", "protected", "public", "readonly", "ref",
        "return", "sealed", "set", "static", "struct", "switch", "this", "throw", "true", "try",
        "typeof", "unsafe", "using", "var", "virtual", "volatile", "while", "yield",
    ],
    types: &[
        "bool", "byte", "char", "decimal", "double", "float", "int", "long", "object", "sbyte",
        "short", "string", "uint", "ulong", "ushort", "void",
    ],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: true,
    backtick_string: false,
};

const JAVA: Lang = Lang {
    keywords: &[
        "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class",
        "const", "continue", "default", "do", "double", "else", "enum", "extends", "final",
        "finally", "float", "for", "goto", "if", "implements", "import", "instanceof", "int",
        "interface", "long", "native", "new", "package", "private", "protected", "public",
        "return", "short", "static", "strictfp", "super", "switch", "synchronized", "this",
        "throw", "throws", "transient", "try", "var", "void", "volatile", "while", "true", "false",
        "null",
    ],
    types: &["String", "Integer", "Long", "Double", "Float", "Boolean", "Object", "List", "Map", "Set"],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: false,
    backtick_string: false,
};

const JS: Lang = Lang {
    keywords: &[
        "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
        "default", "delete", "do", "else", "export", "extends", "finally", "for", "function", "get",
        "if", "import", "in", "instanceof", "let", "new", "of", "return", "set", "static", "super",
        "switch", "this", "throw", "try", "typeof", "var", "void", "while", "with", "yield", "true",
        "false", "null", "undefined",
    ],
    types: &["Array", "Boolean", "Date", "JSON", "Map", "Math", "Number", "Object", "Promise", "Set", "String", "Symbol"],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: false,
    backtick_string: true,
};

const PY: Lang = Lang {
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
        "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
        "yield", "True", "False", "None", "self", "print",
    ],
    types: &[
        "bool", "bytes", "dict", "float", "int", "list", "set", "str", "tuple", "object", "type",
    ],
    line: &["#"],
    block: None,
    hash_comment: true,
    preproc: false,
    backtick_string: false,
};

const RUST: Lang = Lang {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "static", "struct", "super", "trait", "true", "type",
        "union", "unsafe", "use", "where", "while",
    ],
    types: &[
        "bool", "Box", "char", "f32", "f64", "i8", "i16", "i32", "i64", "isize", "Option", "Result",
        "String", "str", "u8", "u16", "u32", "u64", "usize", "Vec",
    ],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: false,
    backtick_string: false,
};

const GO: Lang = Lang {
    keywords: &[
        "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
        "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range",
        "return", "select", "struct", "switch", "type", "var", "true", "false", "nil",
    ],
    types: &["bool", "byte", "error", "float64", "int", "int64", "rune", "string", "uint"],
    line: &["//"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: false,
    backtick_string: true,
};

const SH: Lang = Lang {
    keywords: &[
        "and", "bash", "break", "case", "continue", "do", "done", "elif", "else", "esac", "eval",
        "exec", "exit", "export", "fi", "for", "function", "if", "in", "local", "read", "return",
        "set", "shift", "source", "then", "trap", "until", "while",
    ],
    types: &[],
    line: &["#"],
    block: None,
    hash_comment: true,
    preproc: false,
    backtick_string: false,
};

const SQL: Lang = Lang {
    keywords: &[
        "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create",
        "table", "drop", "alter", "join", "left", "right", "inner", "outer", "on", "group", "by",
        "order", "having", "limit", "as", "and", "or", "not", "null", "primary", "key", "foreign",
        "references", "index", "view", "distinct", "union", "case", "when", "then", "else", "end",
    ],
    types: &["int", "integer", "text", "varchar", "date", "bool", "numeric"],
    line: &["--"],
    block: Some(("/*", "*/")),
    hash_comment: false,
    preproc: false,
    backtick_string: false,
};

const JSON: Lang = Lang {
    keywords: &["true", "false", "null"],
    types: &[],
    line: &[],
    block: None,
    hash_comment: false,
    preproc: false,
    backtick_string: false,
};

fn lang_of(tag: &str) -> &'static Lang {
    match tag {
        "c" | "h" => &C,
        "cpp" | "c++" | "cc" | "cxx" | "hpp" | "h++" | "cuda" => &CPP,
        "csharp" | "c#" | "cs" | "dotnet" | "vb" => &CS,
        "java" | "kotlin" | "kt" | "scala" | "groovy" => &JAVA,
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" | "node" | "vue" => &JS,
        "py" | "python" | "python3" | "ipython" => &PY,
        "rs" | "rust" => &RUST,
        "go" | "golang" => &GO,
        "sh" | "bash" | "zsh" | "shell" | "console" | "powershell" | "ps1" => &SH,
        "sql" | "mysql" | "postgres" | "plsql" => &SQL,
        "json" | "jsonc" | "json5" | "yaml" | "yml" | "toml" => &JSON,
        _ => &DEFAULT,
    }
}

struct Palette {
    text: Color32,
    keyword: Color32,
    type_: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    func: Color32,
    preproc: Color32,
}

const LIGHT: Palette = Palette {
    text: Color32::from_rgb(0x33, 0x30, 0x2B),
    keyword: Color32::from_rgb(0x1F, 0x4F, 0xBF),
    type_: Color32::from_rgb(0x0A, 0x6F, 0x7A),
    string: Color32::from_rgb(0x0B, 0x6B, 0x3A),
    number: Color32::from_rgb(0xA8, 0x52, 0x00),
    comment: Color32::from_rgb(0x8C, 0x88, 0x80),
    func: Color32::from_rgb(0x7A, 0x35, 0x9E),
    preproc: Color32::from_rgb(0x9E, 0x20, 0x5C),
};

const DARK: Palette = Palette {
    text: Color32::from_rgb(0xDE, 0xDA, 0xD2),
    keyword: Color32::from_rgb(0x79, 0xB0, 0xFF),
    type_: Color32::from_rgb(0x5B, 0xD2, 0xE0),
    string: Color32::from_rgb(0x8F, 0xD8, 0x9A),
    number: Color32::from_rgb(0xE8, 0xB0, 0x6A),
    comment: Color32::from_rgb(0x8C, 0x88, 0x80),
    func: Color32::from_rgb(0xD2, 0xA0, 0xF0),
    preproc: Color32::from_rgb(0xF0, 0x80, 0xB0),
};

fn color_of(p: &Palette, t: Tok) -> Color32 {
    match t {
        Tok::Text => p.text,
        Tok::Keyword => p.keyword,
        Tok::Type => p.type_,
        Tok::String => p.string,
        Tok::Number => p.number,
        Tok::Comment => p.comment,
        Tok::Func => p.func,
        Tok::Preproc => p.preproc,
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$' || c == '@'
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Build a syntax-highlighted layout job for `text`.
pub fn code_job(text: &str, lang_tag: &str, size: f32, dark: bool) -> LayoutJob {
    let lang = lang_of(lang_tag);
    let pal = if dark { &DARK } else { &LIGHT };
    let mut job = LayoutJob::default();
    let mut i = 0usize;
    // Byte offsets, so every slice below stays on a char boundary.
    let push = |from: usize, to: usize, tok: Tok, italics: bool, job: &mut LayoutJob| {
        if to <= from {
            return;
        }
        let fmt = TextFormat {
            font_id: FontId::monospace(size),
            color: color_of(pal, tok),
            italics,
            ..Default::default()
        };
        job.append(&text[from..to], 0.0, fmt);
    };

    while i < text.len() {
        let rest = &text[i..];
        let line_start = i == 0 || text.as_bytes()[i - 1] == b'\n';

        if line_start {
            let trimmed = rest.trim_start_matches([' ', '\t']);
            let indent = rest.len() - trimmed.len();
            if lang.preproc && trimmed.starts_with('#') {
                let end = rest.find('\n').map(|o| i + o).unwrap_or(text.len());
                push(i, i + indent, Tok::Text, false, &mut job);
                push(i + indent, end, Tok::Preproc, false, &mut job);
                i = end;
                continue;
            }
            if lang.hash_comment && trimmed.starts_with('#') {
                let end = rest.find('\n').map(|o| i + o).unwrap_or(text.len());
                push(i, i + indent, Tok::Text, false, &mut job);
                push(i + indent, end, Tok::Comment, true, &mut job);
                i = end;
                continue;
            }
        }

        if let Some((open, close)) = lang.block
            && rest.starts_with(open)
        {
            let end = rest[open.len()..]
                .find(close)
                .map(|o| i + open.len() + o + close.len())
                .unwrap_or(text.len());
            push(i, end, Tok::Comment, true, &mut job);
            i = end;
            continue;
        }
        if lang.line.iter().any(|c| rest.starts_with(*c)) {
            let end = rest.find('\n').map(|o| i + o).unwrap_or(text.len());
            push(i, end, Tok::Comment, true, &mut job);
            i = end;
            continue;
        }
        if lang.hash_comment && rest.starts_with('#') {
            let end = rest.find('\n').map(|o| i + o).unwrap_or(text.len());
            push(i, end, Tok::Comment, true, &mut job);
            i = end;
            continue;
        }

        let c = text[i..].chars().next().unwrap();
        if c == '"' || c == '\'' || (c == '`' && lang.backtick_string) {
            let mut j = i + c.len_utf8();
            while j < text.len() {
                let ch = text[j..].chars().next().unwrap();
                if ch == '\\' {
                    j += 2;
                    continue;
                }
                if ch == c {
                    j += ch.len_utf8();
                    break;
                }
                if ch == '\n' && c != '`' {
                    break;
                }
                j += ch.len_utf8();
            }
            push(i, j.min(text.len()), Tok::String, false, &mut job);
            i = j.min(text.len());
            continue;
        }
        if c.is_ascii_digit()
            || (c == '.' && text[i + 1..].starts_with(|n: char| n.is_ascii_digit()))
        {
            let mut j = i;
            while j < text.len() {
                let ch = text[j..].chars().next().unwrap();
                if ch.is_ascii_alphanumeric() || ch == '.' || ((ch == 'x' || ch == 'X') && j - i < 2)
                {
                    j += ch.len_utf8();
                } else {
                    break;
                }
            }
            push(i, j, Tok::Number, false, &mut job);
            i = j;
            continue;
        }
        if is_ident_start(c) {
            let mut j = i + c.len_utf8();
            while j < text.len() && is_ident(text[j..].chars().next().unwrap()) {
                j += text[j..].chars().next().unwrap().len_utf8();
            }
            let word = &text[i..j];
            let lower = word.to_ascii_lowercase();
            let tok = if lang.keywords.contains(&word) || lang.keywords.contains(&lower.as_str()) {
                Tok::Keyword
            } else if lang.types.contains(&word) {
                Tok::Type
            } else if text[j..].trim_start().starts_with('(') {
                Tok::Func
            } else {
                Tok::Text
            };
            push(i, j, tok, false, &mut job);
            i = j;
            continue;
        }
        // One character at a time keeps punctuation in its own (mergeable) run.
        let w = c.len_utf8();
        push(i, i + w, Tok::Text, false, &mut job);
        i += w;
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::text::ByteIndex;

    /// The sections of a job, with adjacent equal formats already merged by
    /// `LayoutJob::append`.
    fn runs(src: &str, job: &LayoutJob) -> Vec<(String, Color32)> {
        job.sections
            .iter()
            .map(|s| {
                let r = &s.byte_range;
                let (ByteIndex(a), ByteIndex(b)) = (r.start, r.end);
                (src[a.min(src.len())..b.min(src.len())].to_string(), s.format.color)
            })
            .collect()
    }

    #[test]
    fn classifies_rust() {
        let src = "fn main() {\n    let x = 42; // note\n}";
        let job = code_job(src, "rs", 14.0, false);
        let runs = runs(src, &job);
        let find = |t: &str| runs.iter().find(|(s, _)| s == t).map(|(_, c)| *c);
        assert_eq!(find("fn"), Some(LIGHT.keyword));
        assert_eq!(find("main"), Some(LIGHT.func));
        assert_eq!(find("let"), Some(LIGHT.keyword));
        assert_eq!(find("42"), Some(LIGHT.number));
        assert_eq!(find("// note"), Some(LIGHT.comment));
        assert_eq!(job.text, src);
    }

    #[test]
    fn classifies_c_preprocessor_and_string() {
        let src = "#include <stdio.h>\nchar *s = \"hi\";";
        let job = code_job(src, "c", 14.0, true);
        let runs = runs(src, &job);
        let find = |t: &str| runs.iter().find(|(s, _)| s == t).map(|(_, c)| *c);
        assert_eq!(find("#include <stdio.h>"), Some(DARK.preproc));
        assert_eq!(find("char"), Some(DARK.type_));
        assert_eq!(find("\"hi\""), Some(DARK.string));
    }

    #[test]
    fn unknown_language_still_tokenizes_comments() {
        let src = "// a\nplain text";
        let job = code_job(src, "", 14.0, false);
        assert_eq!(job.text, src);
        assert!(runs(src, &job).iter().any(|(s, c)| s == "// a" && *c == LIGHT.comment));
    }

    #[test]
    fn survives_unterminated_constructs() {
        for src in ["/* open", "\"open string", "'", "#", "let x: Vec<", "\u{4e2d}\u{6587} \u{6ce8}\u{91ca}"] {
            for tag in ["", "rs", "c", "py", "json", "sh", "nope"] {
                let job = code_job(src, tag, 14.0, false);
                assert_eq!(job.text, src, "{src:?} as {tag}");
            }
        }
    }
}
