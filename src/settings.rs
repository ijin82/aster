use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Структура настроек приложения.
///
/// Сериализуется и десериализуется в JSON с помощью библиотеки `serde`.
/// Значения по умолчанию задаются через трейт `Default`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Цветовая тема приложения:
    /// "system", "light", "dark", "dracula", "kimbie_dark", "solarized_dark", "solarized_light".
    pub theme: String,
    /// Семейство шрифта редактора (например, "Monospace", "Fira Code", "JetBrains Mono").
    #[serde(default = "default_editor_font_family")]
    pub editor_font_family: String,
    /// Размер шрифта текстового редактора в пунктах (pt).
    pub editor_font_size: u32,
    /// Размер базового шрифта в окне предпросмотра Markdown в пикселях (px).
    pub preview_font_size: u32,
    /// Боковые отступы (padding) блока предпросмотра в процентах (по умолчанию 5%).
    #[serde(default = "default_preview_padding")]
    pub preview_padding_percent: u32,
    /// Было ли окно развернуто на весь экран (maximized).
    #[serde(default)]
    pub window_maximized: bool,
    /// Сохраненная ширина окна.
    #[serde(default = "default_window_width")]
    pub window_width: i32,
    /// Сохраненная высота окна.
    #[serde(default = "default_window_height")]
    pub window_height: i32,
    /// Язык интерфейса приложения ("ru" или "en").
    #[serde(default = "default_language")]
    pub language: String,
    /// Показывать ли номера строк в редакторе текста.
    #[serde(default = "default_show_line_numbers")]
    pub show_line_numbers: bool,
}

fn default_show_line_numbers() -> bool {
    true
}

fn default_language() -> String {
    "ru".to_string()
}

fn default_editor_font_family() -> String {
    "Monospace".to_string()
}

fn default_preview_padding() -> u32 {
    5
}

fn default_window_width() -> i32 {
    960
}

fn default_window_height() -> i32 {
    720
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            editor_font_family: default_editor_font_family(),
            editor_font_size: 14,
            preview_font_size: 16,
            preview_padding_percent: 5,
            window_maximized: false,
            window_width: default_window_width(),
            window_height: default_window_height(),
            language: default_language(),
            show_line_numbers: default_show_line_numbers(),
        }
    }
}

impl AppSettings {
    /// Возвращает путь к файлу конфигурации:
    /// Обычно это `~/.config/rmd-view/settings.json` в Linux (по стандарту XDG).
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut p| {
            p.push("rmd-view");
            p.push("settings.json");
            p
        })
    }

    /// Загружает настройки из файла конфигурации.
    /// Если файл отсутствует или поврежден, возвращаются настройки по умолчанию.
    pub fn load() -> Self {
        if let Some(path) = Self::config_path() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(settings) = serde_json::from_str::<AppSettings>(&content) {
                    return settings;
                }
            }
        }
        Self::default()
    }

    /// Сохраняет текущие настройки на диск в формате JSON с отступами.
    /// Создает директорию `~/.config/rmd-view`, если она еще не существует.
    pub fn save(&self) {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = fs::write(path, json);
            }
        }
    }
}
