use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Имя каталога конфигурации приложения в XDG_CONFIG_HOME.
const CONFIG_DIR_NAME: &str = "aster";

/// Положение прокрутки и режим просмотра, сохраненные для одного документа.
///
/// В отличие от позиции курсора, здесь хранятся два независимых значения:
/// `editor_scroll` — вертикальная прокрутка текстового редактора (в пикселях GTK),
/// `preview_scroll` — вертикальная прокрутка HTML-предпросмотра (в пикселях WebKit).
/// Благодаря этому при переключении режимов не нужно пересчитывать координаты
/// элементов HTML: каждый режим просто возвращается туда, где пользователь его оставил.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct FileViewState {
    /// Позиция вертикальной прокрутки редактора (GtkAdjustment::value).
    #[serde(default)]
    pub editor_scroll: f64,
    /// Позиция вертикальной прокрутки окна предпросмотра (window.scrollY).
    #[serde(default)]
    pub preview_scroll: f64,
    /// Был ли документ открыт в режиме предпросмотра в прошлой сессии.
    #[serde(default)]
    pub preview_active: bool,
}

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
    /// Ширина символа табуляции при отображении, в количестве моноширинных знаков.
    #[serde(default = "default_tab_width")]
    pub tab_width: u32,
    /// Количество пробелов, которое вставляет Tab и добавляет автоотступ.
    #[serde(default = "default_indent_width")]
    pub indent_width: u32,
    /// Включен ли мягкий перенос строк.
    #[serde(default = "default_soft_wrap")]
    pub soft_wrap: bool,
    /// Показывать ли пробелы и табы в редакторе.
    #[serde(default = "default_show_whitespace")]
    pub show_whitespace: bool,
    /// Сохраненные позиции курсора (строка, колонка) для недавно открывавшихся файлов.
    #[serde(default)]
    pub file_cursor_positions: HashMap<String, (usize, usize)>,
    /// Сохраненные позиции прокрутки редактора и предпросмотра для каждого файла.
    ///
    /// Ключ — канонический абсолютный путь к файлу, значение — [`FileViewState`].
    #[serde(default)]
    pub file_view_states: HashMap<String, FileViewState>,
    /// Список файлов, открытых во вкладках на момент завершения прошлой сессии.
    ///
    /// Хранятся только документы, имеющие путь на диске. Новая безымянная вкладка
    /// не сохраняется, так как ее содержимое некуда восстановить без отдельного файла.
    #[serde(default)]
    pub session_files: Vec<String>,
    /// Индекс активной вкладки в [`AppSettings::session_files`].
    #[serde(default)]
    pub session_active_document: usize,
}

fn default_show_line_numbers() -> bool {
    true
}

fn default_tab_width() -> u32 {
    2
}

fn default_indent_width() -> u32 {
    2
}

fn default_soft_wrap() -> bool {
    true
}

fn default_show_whitespace() -> bool {
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
            tab_width: default_tab_width(),
            indent_width: default_indent_width(),
            soft_wrap: default_soft_wrap(),
            show_whitespace: default_show_whitespace(),
            file_cursor_positions: HashMap::new(),
            file_view_states: HashMap::new(),
            session_files: Vec::new(),
            session_active_document: 0,
        }
    }
}

impl AppSettings {
    /// Путь к актуальному файлу настроек: `~/.config/aster/settings.json`.
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut p| {
            p.push(CONFIG_DIR_NAME);
            p.push("settings.json");
            p
        })
    }

    /// Загружает настройки из файла конфигурации.
    ///
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

    /// Переключает единый режим отображения пробелов и мягкого переноса.
    ///
    /// Возвращает новое состояние режима. Если хотя бы одна из двух
    /// настроек выключена, режим включается целиком; если обе включены —
    /// выключаются обе. Так `Ctrl+Shift+P` всегда приводит редактор к
    /// одному из двух предсказуемых состояний, а не оставляет смешанное.
    pub fn toggle_whitespace_wrap(&mut self) -> bool {
        let enabled = !(self.show_whitespace && self.soft_wrap);
        self.show_whitespace = enabled;
        self.soft_wrap = enabled;
        enabled
    }

    /// Сохраняет текущие настройки на диск в формате JSON с отступами.
    ///
    /// Запись выполняется атомарно: сначала во временный файл рядом с
    /// `settings.json`, затем `rename`. Если приложение упадет во время
    /// записи, старый файл настроек останется целым.
    pub fn save(&self) {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = write_atomic(&path, &json);
            }
        }
    }
}

/// Атомарно записывает текст в файл через временный файл и `rename`.
fn write_atomic(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let temp_path = path.with_extension("json.tmp");

    if let Err(error) = fs::write(&temp_path, content) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_roundtrip_with_session_and_view_states() {
        let mut settings = AppSettings::default();
        settings.theme = "dracula".to_string();
        settings.session_files = vec!["/tmp/a.md".to_string(), "/tmp/b.md".to_string()];
        settings.session_active_document = 1;
        settings.file_view_states.insert(
            "/tmp/a.md".to_string(),
            FileViewState {
                editor_scroll: 42.5,
                preview_scroll: 120.0,
                preview_active: true,
            },
        );

        let json = serde_json::to_string(&settings).expect("serialize settings");
        let restored: AppSettings = serde_json::from_str(&json).expect("deserialize settings");

        assert_eq!(restored.session_files, settings.session_files);
        assert_eq!(restored.session_active_document, 1);
        assert_eq!(
            restored.file_view_states.get("/tmp/a.md"),
            Some(&FileViewState {
                editor_scroll: 42.5,
                preview_scroll: 120.0,
                preview_active: true,
            })
        );
    }

    #[test]
    fn test_old_settings_json_without_new_fields_is_accepted() {
        // Старые версии приложения не знали о вкладках и позициях прокрутки,
        // поэтому отсутствующие поля должны заполняться значениями по умолчанию.
        let old_json = r#"{
            "theme": "light",
            "editor_font_size": 14,
            "preview_font_size": 16
        }"#;

        let restored: AppSettings = serde_json::from_str(old_json).expect("deserialize old settings");
        assert!(restored.session_files.is_empty());
        assert_eq!(restored.session_active_document, 0);
        assert!(restored.file_view_states.is_empty());
        assert_eq!(restored.language, "ru");
        assert_eq!(restored.tab_width, 2);
        assert_eq!(restored.indent_width, 2);
        assert!(restored.soft_wrap);
        assert!(restored.show_whitespace);
    }

    #[test]
    fn test_toggle_whitespace_wrap_is_atomic() {
        let mut settings = AppSettings::default();
        assert!(settings.show_whitespace);
        assert!(settings.soft_wrap);

        // Оба включены -> выключаются оба.
        assert!(!settings.toggle_whitespace_wrap());
        assert!(!settings.show_whitespace);
        assert!(!settings.soft_wrap);

        // Оба выключены -> включаются оба.
        assert!(settings.toggle_whitespace_wrap());
        assert!(settings.show_whitespace);
        assert!(settings.soft_wrap);

        // Смешанное состояние тоже приводится к включенному режиму.
        settings.soft_wrap = false;
        assert!(settings.toggle_whitespace_wrap());
        assert!(settings.show_whitespace);
        assert!(settings.soft_wrap);
    }
}


#[cfg(test)]
mod atomic_write_tests {
    use super::*;

    #[test]
    fn test_write_atomic_replaces_file() {
        let dir = std::env::temp_dir().join(format!(
            "aster-atomic-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        let path = dir.join("settings.json");

        write_atomic(&path, "first").expect("first write");
        assert_eq!(fs::read_to_string(&path).expect("read first"), "first");

        write_atomic(&path, "second").expect("second write");
        assert_eq!(fs::read_to_string(&path).expect("read second"), "second");
        assert!(!path.with_extension("json.tmp").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
