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
            Language::Russian => "Открыть файлы (Ctrl+O)",
            Language::English => "Open files (Ctrl+O)",
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
            Language::Russian => "Редактор (Ctrl+E для переключения)",
            Language::English => "Editor (Ctrl+E to switch)",
        }
    }

    pub fn tooltip_preview_mode(&self) -> &'static str {
        match self {
            Language::Russian => "Просмотр Markdown (Ctrl+E для переключения)",
            Language::English => "Markdown Preview (Ctrl+E to switch)",
        }
    }

    pub fn tooltip_search(&self) -> &'static str {
        match self {
            Language::Russian => "Поиск подстроки (Ctrl+F)",
            Language::English => "Find text (Ctrl+F)",
        }
    }

    pub fn tooltip_search_previous(&self) -> &'static str {
        match self {
            Language::Russian => "Предыдущее совпадение (Shift+Enter)",
            Language::English => "Previous match (Shift+Enter)",
        }
    }

    pub fn tooltip_search_next(&self) -> &'static str {
        match self {
            Language::Russian => "Следующее совпадение (Enter)",
            Language::English => "Next match (Enter)",
        }
    }

    pub fn search_no_results(&self) -> &'static str {
        match self {
            Language::Russian => "Нет совпадений",
            Language::English => "No matches",
        }
    }

    pub fn search_counter(&self, current: usize, total: usize) -> String {
        match self {
            Language::Russian => format!("{current} из {total}"),
            Language::English => format!("{current} of {total}"),
        }
    }

    pub fn tooltip_replace(&self) -> &'static str {
        match self {
            Language::Russian => "Замена (Ctrl+R)",
            Language::English => "Replace (Ctrl+R)",
        }
    }

    pub fn replace_placeholder(&self) -> &'static str {
        match self {
            Language::Russian => "Заменить на…",
            Language::English => "Replace with…",
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

    pub fn menu_reopen_tab(&self) -> &'static str {
        match self {
            Language::Russian => "Восстановить закрытую вкладку",
            Language::English => "Reopen Closed Tab",
        }
    }

    pub fn menu_meditation(&self) -> &'static str {
        match self {
            Language::Russian => "Режим медитации",
            Language::English => "Meditation Mode",
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

    pub fn menu_help(&self) -> &'static str {
        match self {
            Language::Russian => "Справка",
            Language::English => "Help",
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

    pub fn btn_replace(&self) -> &'static str {
        match self {
            Language::Russian => "Заменить",
            Language::English => "Replace",
        }
    }

    pub fn btn_replace_all(&self) -> &'static str {
        match self {
            Language::Russian => "Заменить всё",
            Language::English => "Replace All",
        }
    }

    pub fn menu_find_replace(&self) -> &'static str {
        match self {
            Language::Russian => "Найти и заменить",
            Language::English => "Find and Replace",
        }
    }

    // --- Диалоги открытия и сохранения файлов ---

    pub fn dialog_save_title(&self) -> &'static str {
        match self {
            Language::Russian => "Сохранить Markdown файл",
            Language::English => "Save Markdown File",
        }
    }

    pub fn error_open_file(&self) -> &'static str {
        match self {
            Language::Russian => "Не удалось открыть файл",
            Language::English => "Could not open file",
        }
    }

    pub fn error_save_file(&self) -> &'static str {
        match self {
            Language::Russian => "Не удалось сохранить файл",
            Language::English => "Could not save file",
        }
    }

    pub fn error_reload_file(&self) -> &'static str {
        match self {
            Language::Russian => "Не удалось перезагрузить файл",
            Language::English => "Could not reload file",
        }
    }

    pub fn file_changed_on_disk(&self) -> &'static str {
        match self {
            Language::Russian => "Файл изменён на диске",
            Language::English => "File changed on disk",
        }
    }

    pub fn file_reloaded(&self) -> &'static str {
        match self {
            Language::Russian => "Файл перезагружен с диска",
            Language::English => "File reloaded from disk",
        }
    }

    pub fn btn_reload(&self) -> &'static str {
        match self {
            Language::Russian => "Перезагрузить",
            Language::English => "Reload",
        }
    }

    pub fn drafts_recovered(&self, count: usize) -> String {
        match self {
            Language::Russian => format!("Восстановлено черновиков после сбоя: {count}"),
            Language::English => format!("Recovered drafts after a crash: {count}"),
        }
    }

    pub fn dialog_open_title(&self) -> &'static str {
        match self {
            Language::Russian => "Открыть Markdown файлы",
            Language::English => "Open Markdown Files",
        }
    }

    pub fn filter_markdown(&self) -> &'static str {
        match self {
            Language::Russian => "Markdown (*.md, *.markdown)",
            Language::English => "Markdown (*.md, *.markdown)",
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

    pub fn pref_editor_group_title(&self) -> &'static str {
        match self {
            Language::Russian => "Редактор",
            Language::English => "Editor",
        }
    }

    pub fn pref_editor_group_desc(&self) -> &'static str {
        match self {
            Language::Russian => "Отступы, перенос строк и отображение пробелов",
            Language::English => "Indentation, line wrapping, and whitespace display",
        }
    }

    pub fn pref_tab_width_title(&self) -> &'static str {
        match self {
            Language::Russian => "Ширина табуляции",
            Language::English => "Tab Width",
        }
    }

    pub fn pref_tab_width_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Ширина существующих символов табуляции в моноширинных знаках",
            Language::English => "Display width of existing tab characters in monospace cells",
        }
    }

    pub fn pref_indent_width_title(&self) -> &'static str {
        match self {
            Language::Russian => "Размер отступов",
            Language::English => "Indent Size",
        }
    }

    pub fn pref_indent_width_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Сколько пробелов вставляют Tab и Shift+Tab (табы не вставляются)",
            Language::English => "Number of spaces inserted by Tab and Shift+Tab (no tab characters)",
        }
    }

    pub fn pref_soft_wrap_title(&self) -> &'static str {
        match self {
            Language::Russian => "Перенос строк",
            Language::English => "Soft Wrap",
        }
    }

    pub fn pref_soft_wrap_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Переносить длинные строки по ширине окна",
            Language::English => "Wrap long lines to the window width",
        }
    }

    pub fn pref_show_whitespace_title(&self) -> &'static str {
        match self {
            Language::Russian => "Показывать пробелы и табы",
            Language::English => "Show Spaces and Tabs",
        }
    }

    pub fn pref_show_whitespace_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Отмечать пробелы точками, а табы — горизонтальными линиями",
            Language::English => "Mark spaces with dots and tabs with horizontal lines",
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

    pub fn pref_markdown_lint_title(&self) -> &'static str {
        match self {
            Language::Russian => "Проверять стиль Markdown",
            Language::English => "Check Markdown Style",
        }
    }

    pub fn pref_markdown_lint_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => {
                "Подчёркивать нарушения правил MD009, MD010, MD012, MD022, MD025, MD031, MD032, MD040 и MD047"
            }
            Language::English => {
                "Underline violations of MD009, MD010, MD012, MD022, MD025, MD031, MD032, MD040, and MD047"
            }
        }
    }

    pub fn pref_help_group_title(&self) -> &'static str {
        match self {
            Language::Russian => "Справка",
            Language::English => "Help",
        }
    }

    pub fn pref_lint_rules_title(&self) -> &'static str {
        match self {
            Language::Russian => "Правила проверки Markdown",
            Language::English => "Markdown Lint Rules",
        }
    }

    pub fn pref_lint_rules_subtitle(&self) -> &'static str {
        match self {
            Language::Russian => "Справочник с объяснениями и примерами",
            Language::English => "Reference with explanations and examples",
        }
    }

    /// Описание нарушения стиля Markdown по идентификатору правила.
    ///
    /// Строки живут в i18n, а не в модуле диагностики: при смене языка
    /// список проблем пересоздавать не нужно, сообщение собирается в
    /// момент показа подсказки.
    pub fn lint_message(&self, rule: &str) -> &'static str {
        match rule {
            "MD009" => match self {
                Language::Russian => "Лишние пробелы в конце строки",
                Language::English => "Trailing spaces at the end of the line",
            },
            "MD010" => match self {
                Language::Russian => "Жёсткая табуляция",
                Language::English => "Hard tab character",
            },
            "MD012" => match self {
                Language::Russian => "Несколько пустых строк подряд",
                Language::English => "Multiple consecutive blank lines",
            },
            "MD022" => match self {
                Language::Russian => "Заголовок должен быть окружён пустыми строками",
                Language::English => "Headings should be surrounded by blank lines",
            },
            "MD025" => match self {
                Language::Russian => {
                    "В документе должен быть только один заголовок первого уровня"
                }
                Language::English => "Multiple top-level headings in the same document",
            },
            "MD031" => match self {
                Language::Russian => "Блок кода должен быть окружён пустыми строками",
                Language::English => "Fenced code blocks should be surrounded by blank lines",
            },
            "MD032" => match self {
                Language::Russian => "Список должен быть окружён пустыми строками",
                Language::English => "Lists should be surrounded by blank lines",
            },
            "MD040" => match self {
                Language::Russian => "У блока кода не указан язык",
                Language::English => "Fenced code blocks should have a language specified",
            },
            "MD047" => match self {
                Language::Russian => "Файл должен заканчиваться одним переводом строки",
                Language::English => "Files should end with a single newline character",
            },
            _ => match self {
                Language::Russian => "Нарушение стиля Markdown",
                Language::English => "Markdown style violation",
            },
        }
    }

    // --- Справка по стилю Markdown ---

    pub fn lint_details(&self) -> &'static str {
        match self {
            Language::Russian => "Подробнее…",
            Language::English => "Learn More…",
        }
    }

    pub fn lint_open_docs(&self) -> &'static str {
        match self {
            Language::Russian => "Документация markdownlint",
            Language::English => "markdownlint documentation",
        }
    }

    pub fn lint_tooltip_hint(&self) -> &'static str {
        match self {
            Language::Russian => "ПКМ — подробнее",
            Language::English => "Right-click for details",
        }
    }

    pub fn lint_help_window_title(&self) -> &'static str {
        match self {
            Language::Russian => "Справка по стилю Markdown",
            Language::English => "Markdown Style Help",
        }
    }

    pub fn lint_help_all_rules(&self) -> &'static str {
        match self {
            Language::Russian => "Правила проверки Markdown",
            Language::English => "Markdown Lint Rules",
        }
    }

    pub fn lint_help_search_placeholder(&self) -> &'static str {
        match self {
            Language::Russian => "Поиск по номеру или описанию",
            Language::English => "Search by number or description",
        }
    }

    pub fn lint_help_empty(&self) -> &'static str {
        match self {
            Language::Russian => "Ничего не найдено",
            Language::English => "No rules found",
        }
    }

    pub fn lint_help_why(&self) -> &'static str {
        match self {
            Language::Russian => "Почему это важно",
            Language::English => "Why it matters",
        }
    }

    pub fn lint_help_bad(&self) -> &'static str {
        match self {
            Language::Russian => "Неправильно",
            Language::English => "Incorrect",
        }
    }

    pub fn lint_help_good(&self) -> &'static str {
        match self {
            Language::Russian => "Правильно",
            Language::English => "Correct",
        }
    }

    pub fn lint_help_legend(&self) -> &'static str {
        match self {
            Language::Russian => "Обозначения: · — пробел, ⇥ — табуляция, ⏎ — перевод строки.",
            Language::English => "Legend: · — space, ⇥ — tab, ⏎ — newline.",
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
