//! Модуль интернационализации (i18n).
//!
//! Обеспечивает поддержку двух языков интерфейса: русский (`ru`) и английский (`en`).
//! Предоставляет типизированные переводы для всех элементов графического интерфейса:
//! тултипов кнопок, названий действий меню, диалогов, вкладок и окна настроек.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Russian,
    English,
}

impl Language {
    /// Преобразует строковый код языка ("ru", "en") в перечисление Language.
    pub fn from_code(code: &str) -> Self {
        match code.to_lowercase().as_str() {
            "en" | "english" => Language::English,
            _ => Language::Russian,
        }
    }

    /// Возвращает двухбуквенный строковый идентификатор языка.
    pub fn as_code(&self) -> &'static str {
        match self {
            Language::Russian => "ru",
            Language::English => "en",
        }
    }

    /// Возвращает человекочитаемое название языка для отображения в списке настроек.
    pub fn display_name(&self) -> &'static str {
        match self {
            Language::Russian => "Русский",
            Language::English => "English",
        }
    }

    // --- Заголовки и тултипы панели инструментов (HeaderBar) ---

    pub fn tooltip_open(&self) -> &'static str {
        match self {
            Language::Russian => "Открыть файл (Ctrl+O)",
            Language::English => "Open file (Ctrl+O)",
        }
    }

    pub fn tooltip_save(&self) -> &'static str {
        match self {
            Language::Russian => "Сохранить (Ctrl+S)",
            Language::English => "Save (Ctrl+S)",
        }
    }

    pub fn tooltip_save_unsaved(&self) -> &'static str {
        match self {
            Language::Russian => "Сохранить (Ctrl+S) — Есть несохраненные изменения",
            Language::English => "Save (Ctrl+S) — Unsaved changes",
        }
    }

    pub fn tooltip_edit_mode(&self) -> &'static str {
        match self {
            Language::Russian => "Редактор (Ctrl+Tab для переключения)",
            Language::English => "Editor (Ctrl+Tab to switch)",
        }
    }

    pub fn tooltip_preview_mode(&self) -> &'static str {
        match self {
            Language::Russian => "Просмотр Markdown (Ctrl+Tab для переключения)",
            Language::English => "Markdown Preview (Ctrl+Tab to switch)",
        }
    }

    pub fn tooltip_search(&self) -> &'static str {
        match self {
            Language::Russian => "Поиск подстроки (Ctrl+F)",
            Language::English => "Find text (Ctrl+F)",
        }
    }

    pub fn tooltip_menu(&self) -> &'static str {
        match self {
            Language::Russian => "Главное меню",
            Language::English => "Main menu",
        }
    }

    // --- Названия вкладок ViewStack ---

    pub fn tab_editor(&self) -> &'static str {
        match self {
            Language::Russian => "Редактор",
            Language::English => "Editor",
        }
    }

    pub fn tab_preview(&self) -> &'static str {
        match self {
            Language::Russian => "Просмотр",
            Language::English => "Preview",
        }
    }

    // --- Пункты главного меню ---

    pub fn menu_new_tab(&self) -> &'static str {
        match self {
            Language::Russian => "Новая вкладка",
            Language::English => "New Tab",
        }
    }

    pub fn menu_close_tab(&self) -> &'static str {
        match self {
            Language::Russian => "Закрыть вкладку",
            Language::English => "Close Tab",
        }
    }

    pub fn menu_save_as(&self) -> &'static str {
        match self {
            Language::Russian => "Сохранить как...",
            Language::English => "Save as...",
        }
    }

    pub fn menu_preferences(&self) -> &'static str {
        match self {
            Language::Russian => "Настройки",
            Language::English => "Preferences",
        }
    }

    pub fn menu_about(&self) -> &'static str {
        match self {
            Language::Russian => "О программе",
            Language::English => "About",
        }
    }

    pub fn menu_close(&self) -> &'static str {
        match self {
            Language::Russian => "Закрыть окно",
            Language::English => "Close window",
        }
    }

    // --- Вкладки ---

    pub fn tooltip_new_tab(&self) -> &'static str {
        match self {
            Language::Russian => "Новая вкладка (Ctrl+T)",
            Language::English => "New tab (Ctrl+T)",
        }
    }

    pub fn untitled_document(&self) -> &'static str {
        match self {
            Language::Russian => "Безымянный",
            Language::English => "Untitled",
        }
    }

    pub fn search_placeholder(&self) -> &'static str {
        match self {
            Language::Russian => "Поиск в документе…",
            Language::English => "Search in document…",
        }
    }

    pub fn tab_close_heading(&self) -> &'static str {
        match self {
            Language::Russian => "Закрыть вкладку без сохранения?",
            Language::English => "Close tab without saving?",
        }
    }

    pub fn tab_close_body(&self) -> &'static str {
        match self {
            Language::Russian => "В этом документе есть несохраненные изменения. Закрытие вкладки приведет к их потере.",
            Language::English => "This document has unsaved changes. Closing the tab will discard them.",
        }
    }

    pub fn unsaved_window_heading(&self) -> &'static str {
        match self {
            Language::Russian => "Закрыть приложение без сохранения?",
            Language::English => "Close the application without saving?",
        }
    }

    pub fn unsaved_window_body(&self) -> &'static str {
        match self {
            Language::Russian => "В одной или нескольких вкладках есть несохраненные изменения. Закрытие приложения приведет к их потере.",
            Language::English => "One or more tabs have unsaved changes. Closing the application will discard them.",
        }
    }

    pub fn btn_save_all(&self) -> &'static str {
        match self {
            Language::Russian => "Сохранить все",
            Language::English => "Save All",
        }
    }

    // --- Диалоги открытия и сохранения файлов ---

    pub fn dialog_save_title(&self) -> &'static str {
        match self {
            Language::Russian => "Сохранить Markdown файл",
            Language::English => "Save Markdown File",
        }
    }

    pub fn dialog_open_title(&self) -> &'static str {
        match self {
            Language::Russian => "Открыть Markdown файл",
            Language::English => "Open Markdown File",
        }
    }

    pub fn filter_markdown(&self) -> &'static str {
        match self {
            Language::Russian => "Markdown (*.md, *.markdown, *.rmd)",
            Language::English => "Markdown (*.md, *.markdown, *.rmd)",
        }
    }

    pub fn btn_cancel(&self) -> &'static str {
        match self {
            Language::Russian => "Отмена",
            Language::English => "Cancel",
        }
    }

    pub fn btn_discard(&self) -> &'static str {
        match self {
            Language::Russian => "Не сохранять",
            Language::English => "Discard",
        }
    }

    pub fn btn_save(&self) -> &'static str {
        match self {
            Language::Russian => "Сохранить",
            Language::English => "Save",
        }
    }

    // --- Окно настроек (Preferences) ---

    pub fn pref_window_title(&self) -> &'static str {
        match self {
            Language::Russian => "Настройки",
            Language::English => "Preferences",
        }
    }

    pub fn pref_language_group_title(&self) -> &'static str {
        match self {
            Language::Russian => "Язык интерфейса",
            Language::English => "Interface Language",
        }
    }

    pub fn pref_language_group_desc(&self) -> &'static str {
        match self {
            Language::Russian => "Выбор языка элементов управления и диалогов",
            Language::English => "Select application language for UI elements and dialogs",
        }
    }

    pub fn pref_language_row_title(&self) -> &'static str {
        match self {
            Language::Russian => "Язык",
            Language::English => "Language",
        }
    }

    pub fn pref_appearance_group_title(&self) -> &'static str {
        match self {
            Language::Russian => "Оформление",
            Language::English => "Appearance",
        }
    }

    pub fn pref_appearance_group_desc(&self) -> &'static str {
        match self {
            Language::Russian => "Выбор цветовой темы приложения, редактора и предпросмотра",
            Language::English => "Color scheme for the app, editor, and preview",
        }
    }

    pub fn pref_theme_row_title(&self) -> &'static str {
        match self {
            Language::Russian => "Цветовая схема",
            Language::English => "Color Scheme",
        }
    }

    pub fn pref_theme_system(&self) -> &'static str {
        match self {
            Language::Russian => "Системная (Adwaita)",
            Language::English => "System (Adwaita)",
        }
    }

    pub fn pref_layout_group_title(&self) -> &'static str {
        match self {
            Language::Russian => "Макет предпросмотра",
            Language::English => "Preview Layout",
        }
    }

    pub fn pref_layout_group_desc(&self) -> &'static str {
        match self {
            Language::Russian => "Настройка внешнего вида и отступов документа",
            Language::English => "Document layout and margin settings",
        }
    }

    pub fn pref_padding_row_title(&self) -> &'static str {
        match self {
            Language::Russian => "Боковые поля предпросмотра (%)",
            Language::English => "Preview side padding (%)",
        }
    }

    pub fn pref_padding_row_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Отступ от левого и правого края экрана (по умолчанию 5%)",
            Language::English => "Left and right margin percentage (default 5%)",
        }
    }

    pub fn pref_fonts_group_title(&self) -> &'static str {
        match self {
            Language::Russian => "Шрифты",
            Language::English => "Fonts",
        }
    }

    pub fn pref_fonts_group_desc(&self) -> &'static str {
        match self {
            Language::Russian => "Настройка гарнитуры и размера текста",
            Language::English => "Font family and text size configuration",
        }
    }

    pub fn pref_font_family_title(&self) -> &'static str {
        match self {
            Language::Russian => "Шрифт редактора",
            Language::English => "Editor Font",
        }
    }

    pub fn pref_font_family_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Выбор из всех моноширинных шрифтов системы",
            Language::English => "Select from all monospace fonts in the system",
        }
    }

    pub fn pref_editor_size_title(&self) -> &'static str {
        match self {
            Language::Russian => "Размер шрифта редактора",
            Language::English => "Editor font size",
        }
    }

    pub fn pref_preview_size_title(&self) -> &'static str {
        match self {
            Language::Russian => "Размер шрифта предпросмотра",
            Language::English => "Preview font size",
        }
    }

    pub fn pref_line_numbers_title(&self) -> &'static str {
        match self {
            Language::Russian => "Номера строк",
            Language::English => "Line Numbers",
        }
    }

    pub fn pref_line_numbers_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Отображать колонку с номерами строк в редакторе",
            Language::English => "Display line numbers column in editor",
        }
    }

    // --- Окно «О программе» (About) ---

    pub fn about_comments(&self) -> &'static str {
        match self {
            Language::Russian => "Быстрый просмотрщик и редактор Markdown в стиле GitHub на Rust и GTK4 / Libadwaita",
            Language::English => "Fast GitHub-style Markdown viewer and editor built with Rust and GTK4 / Libadwaita",
        }
    }
}
