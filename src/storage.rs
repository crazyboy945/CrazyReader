use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Note {
    pub id: u64,
    pub page: usize,
    pub anchor: Option<String>,
    pub text: String,
    pub quote: Option<String>,
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookEntry {
    pub path: String,
    pub title: String,
    pub format: String,
    pub last_opened: String,
    pub page: usize,
    pub scroll: f32,
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Excerpts the reader highlighted; matched against the text at render time.
    #[serde(default)]
    pub marks: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Library {
    pub books: HashMap<String, BookEntry>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub font_size: f32,
    pub dark: bool,
    #[serde(default)]
    pub double_page: bool,
    /// Reading background color as packed 0xRRGGBB. `None` follows the theme.
    #[serde(default)]
    pub bg: Option<u32>,
    /// Key of the selected CJK font; empty means the first available one.
    #[serde(default)]
    pub font: String,
    /// Last directory browsed in the left panel.
    #[serde(default)]
    pub folder: Option<String>,
    /// Expanded branches of the folder tree.
    #[serde(default)]
    pub folder_open: Vec<String>,
    /// Stylesheet applied to Markdown documents.
    #[serde(default)]
    pub css: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: 17.0,
            dark: false,
            double_page: false,
            bg: None,
            font: String::new(),
            folder: None,
            folder_open: Vec::new(),
            css: None,
        }
    }
}

impl Library {
    pub fn load() -> (Self, PathBuf) {
        let dir = config_dir();
        let file = dir.join("library.json");
        let lib = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        (lib, file)
    }

    pub fn save(lib: &Library, file: &Path) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(lib) {
            let tmp = file.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, file);
            }
        }
    }
}

fn config_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs_home());
    base.join("CrazyReader")
}

fn dirs_home() -> PathBuf {
    std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
