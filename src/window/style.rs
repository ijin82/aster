//! Темы, CSS редактора, шрифты и отключение системных звуков.
//!
//! Эти функции не зависят от вкладок и состояния конкретного документа,
//! поэтому вынесены из `window.rs`.

use adw::prelude::*;

/// Полностью отключает звуковую обратную связь GTK.
///
/// Одного `gtk-error-bell` недостаточно: GTK4 отдельно проигрывает
/// «event sounds» (например, звуки открытия/закрытия окон) и
/// «input feedback sounds» для клавиатурных действий. В Markdown-редакторе
/// все три категории лишние, поэтому выключаем их и для стандартных
/// настроек, и для настроек текущего GDK-дисплея.
pub(crate) fn disable_sound_feedback() {
    fn silence(settings: &gtk4::Settings) {
        settings.set_gtk_error_bell(false);
        settings.set_gtk_enable_event_sounds(false);
        settings.set_gtk_enable_input_feedback_sounds(false);
    }

    if let Some(settings) = gtk4::Settings::default() {
        silence(&settings);
    }
    if let Some(display) = gtk4::gdk::Display::default() {
        silence(&gtk4::Settings::for_display(&display));
    }
}

/// Применяет выбранную цветовую схему к Adwaita StyleManager.
pub(crate) fn apply_color_scheme(theme: &str) {
    let style_mgr = adw::StyleManager::default();
    match theme {
        "dark" | "dracula" | "kimbie_dark" | "solarized_dark" => {
            style_mgr.set_color_scheme(adw::ColorScheme::ForceDark)
        }
        "light" | "solarized_light" => {
            style_mgr.set_color_scheme(adw::ColorScheme::ForceLight)
        }
        _ => style_mgr.set_color_scheme(adw::ColorScheme::Default),
    }
}

/// Применяет тему и шрифт редактора через CSS.
///
/// Шрифт намеренно задается здесь, а не через `GtkTextTag`:
/// CSS действует на все символы виджета, включая только что введенные,
/// поэтому размер и гарнитура не «переключаются» на дефолтные при наборе.
pub(crate) fn apply_editor_style_css(
    provider: &gtk4::CssProvider,
    theme: &str,
    is_dark: bool,
    font_family: &str,
    font_size: u32,
) {
    let (bg, fg, cursor_color) = match theme {
        "dracula" => ("#282a36", "#f8f8f2", "#bd93f9"),
        "kimbie_dark" => ("#221a0f", "#d3af86", "#f79a32"),
        "solarized_dark" => ("#002b36", "#839496", "#268bd2"),
        "solarized_light" => ("#fdf6e3", "#657b83", "#268bd2"),
        "light" => ("#ffffff", "#1f2328", "#0969da"),
        "dark" => ("#0d1117", "#e6edf3", "#4493f8"),
        _ => {
            if is_dark {
                ("#0d1117", "#e6edf3", "#4493f8")
            } else {
                ("#ffffff", "#1f2328", "#0969da")
            }
        }
    };

    // Экранируем кавычки и обратные слеши, чтобы имя семейства не сломало CSS.
    let family = font_family.replace('\\', "\\\\").replace('"', "\\\"");

    let css = format!(
        ".editor-view, textview.editor-view, textview.editor-view text {{
            background-color: {bg};
            color: {fg};
            caret-color: {cursor_color};
            font-family: \"{family}\", monospace;
            font-size: {font_size}pt;
        }}
        .shortcut-badge {{
            background-color: rgba(127, 127, 127, 0.14);
            border: 1px solid rgba(127, 127, 127, 0.28);
            border-radius: 6px;
            padding: 1px 8px;
        }}"
    );
    provider.load_from_data(&css);
}

/// Запрашивает у системы список всех моноширинных семейств шрифтов.
pub(crate) fn get_system_monospace_fonts() -> Vec<String> {
    let temp_widget = gtk4::Label::new(None);
    let context = temp_widget.create_pango_context();
    let mut families: Vec<String> = context
        .list_families()
        .into_iter()
        .filter(|family| family.is_monospace())
        .map(|family| family.name().to_string())
        .collect();

    if !families
        .iter()
        .any(|name| name.eq_ignore_ascii_case("Monospace"))
    {
        families.push("Monospace".to_string());
    }

    families.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    families
}
