//! Окно настроек приложения.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::document::Document;
use crate::i18n::Language;
use crate::settings::AppSettings;

use super::style::{apply_color_scheme, apply_editor_style_css, get_system_monospace_fonts};

/// Преобразует GTK-акселератор в одну человекочитаемую строку.
///
/// Пример: `<Control><Shift>w` -> `Ctrl + Shift + W`.
/// В отличие от `GtkShortcutLabel`, который рисует каждую клавишу отдельным
/// блоком, здесь получается единая строка для одной компактной плашки.
fn shortcut_display_text(accelerator: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = accelerator;

    while let Some(start) = rest.find('<') {
        // Текст перед модификатором (обычно пустой) сохраняем как есть.
        if start > 0 {
            parts.push(rest[..start].to_string());
        }

        let Some(relative_end) = rest[start..].find('>') else {
            break;
        };
        let end = start + relative_end;
        let token = &rest[start + 1..end];
        let label = match token {
            "Control" | "Primary" => "Ctrl",
            "Shift" => "Shift",
            "Alt" => "Alt",
            "Super" | "Meta" => "Super",
            other => other,
        };
        parts.push(label.to_string());
        rest = &rest[end + 1..];
    }

    if !rest.is_empty() {
        parts.push(rest.to_string());
    }

    // Одиночную латинскую клавишу показываем заглавной: w -> W.
    // Названия вроде Tab или ISO_Left_Tab не трогаем.
    if let Some(last) = parts.last_mut() {
        let mut chars = last.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if ch.is_ascii_alphabetic() {
                *last = ch.to_ascii_uppercase().to_string();
            }
        }
    }

    parts.join(" + ")
}

/// Добавляет к строке настроек компактную подсказку горячей клавиши.
///
/// Вся комбинация рисуется одной плашкой `Ctrl + Shift + W`, а не набором
/// отдельных клавиш. CSS-класс `shortcut-badge` задается в `style.rs`, а
/// suffix у `AdwSwitchRow` располагает плашку рядом с переключателем.
fn add_shortcut_hint(row: &impl IsA<adw::ActionRow>, accelerator: &str) {
    let shortcut = gtk4::Label::new(Some(&shortcut_display_text(accelerator)));
    shortcut.add_css_class("shortcut-badge");
    shortcut.set_valign(gtk4::Align::Center);
    row.add_suffix(&shortcut);
}

/// Окно настроек (`adw::PreferencesWindow`).
pub(crate) fn show_preferences_dialog(
    parent: &adw::ApplicationWindow,
    settings: Rc<RefCell<AppSettings>>,
    docs: Rc<RefCell<Vec<Rc<Document>>>>,
    editor_css_provider: gtk4::CssProvider,
    refresh_active_ui: Rc<dyn Fn()>,
    update_ui_language: Rc<dyn Fn(Language)>,
) {
    let current_lang = Language::from_code(&settings.borrow().language);

    let (preferences_width, preferences_height) = {
        let settings = settings.borrow();
        (
            settings.preferences_width.max(320),
            settings.preferences_height.max(0),
        )
    };

    let window = adw::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .title(current_lang.pref_window_title())
        .default_width(preferences_width)
        .build();

    // Пользователь должен иметь возможность изменить размер окна.
    window.set_resizable(true);
    if preferences_height > 0 {
        window.set_default_height(preferences_height);
    }

    // Сохраняем фактический размер при закрытии. `default_width/height`
    // хранят только стартовые значения, поэтому для пользовательского
    // размера используем текущие `width()` и `height()`.
    {
        let settings = settings.clone();
        window.connect_close_request(move |window| {
            let width = window.width();
            let height = window.height();
            if width > 0 && height > 0 {
                let mut state = settings.borrow_mut();
                state.preferences_width = width;
                state.preferences_height = height;
                state.save();
            }
            gtk4::glib::Propagation::Proceed
        });
    }

    let page = adw::PreferencesPage::new();

    // --- Язык интерфейса ---
    let language_group = adw::PreferencesGroup::builder()
        .title(current_lang.pref_language_group_title())
        .description(current_lang.pref_language_group_desc())
        .build();

    let languages = [
        (Language::Russian.display_name(), Language::Russian),
        (Language::English.display_name(), Language::English),
    ];
    let lang_labels: Vec<&str> = languages.iter().map(|(label, _)| *label).collect();

    let lang_row = adw::ComboRow::builder()
        .title(current_lang.pref_language_row_title())
        .model(&gtk4::StringList::new(&lang_labels))
        .build();
    lang_row.set_selected(match current_lang {
        Language::Russian => 0,
        Language::English => 1,
    });

    {
        let settings = settings.clone();
        let pref_window = window.clone();
        let update_ui_language = update_ui_language.clone();
        lang_row.connect_selected_notify(move |row| {
            let index = row.selected() as usize;
            if let Some(&(_, selected_lang)) = languages.get(index) {
                settings.borrow_mut().language = selected_lang.as_code().to_string();
                settings.borrow().save();
                pref_window.set_title(Some(selected_lang.pref_window_title()));
                update_ui_language(selected_lang);
            }
        });
    }

    language_group.add(&lang_row);
    page.add(&language_group);

    // --- Оформление ---
    let appearance_group = adw::PreferencesGroup::builder()
        .title(current_lang.pref_appearance_group_title())
        .description(current_lang.pref_appearance_group_desc())
        .build();

    let theme_names = [
        (current_lang.pref_theme_system(), "system"),
        ("GitHub Light", "light"),
        ("GitHub Dark", "dark"),
        ("Dracula", "dracula"),
        ("Kimbie Dark", "kimbie_dark"),
        ("Solarized Dark", "solarized_dark"),
        ("Solarized Light", "solarized_light"),
    ];
    let theme_labels: Vec<&str> = theme_names.iter().map(|(label, _)| *label).collect();

    let theme_row = adw::ComboRow::builder()
        .title(current_lang.pref_theme_row_title())
        .model(&gtk4::StringList::new(&theme_labels))
        .build();
    let current_theme = settings.borrow().theme.clone();
    let initial_theme_idx = theme_names
        .iter()
        .position(|(_, code)| *code == current_theme)
        .unwrap_or(0);
    theme_row.set_selected(initial_theme_idx as u32);

    {
        let settings = settings.clone();
        let provider = editor_css_provider.clone();
        let docs = docs.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        theme_row.connect_selected_notify(move |row| {
            let index = row.selected() as usize;
            let Some(&(_, theme)) = theme_names.get(index) else {
                return;
            };

            {
                let mut state = settings.borrow_mut();
                state.theme = theme.to_string();
                state.save();
            }
            apply_color_scheme(theme);

            let style_mgr = adw::StyleManager::default();
            let (family, size) = {
                let state = settings.borrow();
                (state.editor_font_family.clone(), state.editor_font_size)
            };
            apply_editor_style_css(&provider, theme, style_mgr.is_dark(), &family, size);

            for doc in docs.borrow().iter() {
                doc.apply_theme(theme, style_mgr.is_dark());
            }
            refresh_active_ui();
        });
    }

    appearance_group.add(&theme_row);
    page.add(&appearance_group);

    // --- Макет предпросмотра ---
    let layout_group = adw::PreferencesGroup::builder()
        .title(current_lang.pref_layout_group_title())
        .description(current_lang.pref_layout_group_desc())
        .build();

    let padding_spin = adw::SpinRow::builder()
        .title(current_lang.pref_padding_row_title())
        .subtitle(current_lang.pref_padding_row_subtitle())
        .adjustment(&gtk4::Adjustment::new(
            settings.borrow().preview_padding_percent as f64,
            0.0,
            30.0,
            1.0,
            5.0,
            0.0,
        ))
        .build();

    {
        let settings = settings.clone();
        let docs = docs.clone();
        padding_spin.connect_changed(move |row| {
            settings.borrow_mut().preview_padding_percent = row.value() as u32;
            settings.borrow().save();
            for doc in docs.borrow().iter() {
                if doc.is_preview_active.get() {
                    doc.refresh_preview();
                }
            }
        });
    }
    layout_group.add(&padding_spin);
    page.add(&layout_group);

    // --- Редактор ---
    let editor_group = adw::PreferencesGroup::builder()
        .title(current_lang.pref_editor_group_title())
        .description(current_lang.pref_editor_group_desc())
        .build();

    let tab_width_spin = adw::SpinRow::builder()
        .title(current_lang.pref_tab_width_title())
        .subtitle(current_lang.pref_tab_width_subtitle())
        .adjustment(&gtk4::Adjustment::new(
            settings.borrow().tab_width as f64,
            1.0,
            8.0,
            1.0,
            1.0,
            0.0,
        ))
        .build();

    {
        let settings = settings.clone();
        let docs = docs.clone();
        tab_width_spin.connect_changed(move |row| {
            settings.borrow_mut().tab_width = row.value() as u32;
            settings.borrow().save();
            for doc in docs.borrow().iter() {
                doc.apply_editor_options();
            }
        });
    }
    editor_group.add(&tab_width_spin);

    let indent_width_spin = adw::SpinRow::builder()
        .title(current_lang.pref_indent_width_title())
        .subtitle(current_lang.pref_indent_width_subtitle())
        .adjustment(&gtk4::Adjustment::new(
            settings.borrow().indent_width as f64,
            1.0,
            8.0,
            1.0,
            1.0,
            0.0,
        ))
        .build();

    {
        let settings = settings.clone();
        let docs = docs.clone();
        indent_width_spin.connect_changed(move |row| {
            settings.borrow_mut().indent_width = row.value() as u32;
            settings.borrow().save();
            for doc in docs.borrow().iter() {
                doc.apply_editor_options();
            }
        });
    }
    editor_group.add(&indent_width_spin);

    let soft_wrap_row = adw::SwitchRow::builder()
        .title(current_lang.pref_soft_wrap_title())
        .subtitle(current_lang.pref_soft_wrap_subtitle())
        .active(settings.borrow().soft_wrap)
        .build();
    add_shortcut_hint(&soft_wrap_row, "<Control><Shift>w");

    {
        let settings = settings.clone();
        let docs = docs.clone();
        soft_wrap_row.connect_active_notify(move |row| {
            settings.borrow_mut().soft_wrap = row.is_active();
            settings.borrow().save();
            for doc in docs.borrow().iter() {
                doc.apply_editor_options();
            }
        });
    }
    editor_group.add(&soft_wrap_row);

    let show_whitespace_row = adw::SwitchRow::builder()
        .title(current_lang.pref_show_whitespace_title())
        .subtitle(current_lang.pref_show_whitespace_subtitle())
        .active(settings.borrow().show_whitespace)
        .build();
    add_shortcut_hint(&show_whitespace_row, "<Control><Shift>p");

    {
        let settings = settings.clone();
        let docs = docs.clone();
        show_whitespace_row.connect_active_notify(move |row| {
            settings.borrow_mut().show_whitespace = row.is_active();
            settings.borrow().save();
            for doc in docs.borrow().iter() {
                doc.apply_editor_options();
            }
        });
    }
    editor_group.add(&show_whitespace_row);

    // Номера строк относятся к режиму редактора, поэтому живут в той же
    // группе, что и перенос/пробелы, а не в группе шрифтов.
    let line_numbers_row = adw::SwitchRow::builder()
        .title(current_lang.pref_line_numbers_title())
        .subtitle(current_lang.pref_line_numbers_subtitle())
        .active(settings.borrow().show_line_numbers)
        .build();
    add_shortcut_hint(&line_numbers_row, "<Control><Shift>l");

    {
        let settings = settings.clone();
        let docs = docs.clone();
        line_numbers_row.connect_active_notify(move |row| {
            let active = row.is_active();
            settings.borrow_mut().show_line_numbers = active;
            settings.borrow().save();
            for doc in docs.borrow().iter() {
                doc.set_line_numbers_visible(active);
            }
        });
    }
    editor_group.add(&line_numbers_row);

    page.add(&editor_group);

    // --- Шрифты ---
    let fonts_group = adw::PreferencesGroup::builder()
        .title(current_lang.pref_fonts_group_title())
        .description(current_lang.pref_fonts_group_desc())
        .build();

    let font_options = get_system_monospace_fonts();
    let font_options_refs: Vec<&str> = font_options.iter().map(|s| s.as_str()).collect();

    let font_family_row = adw::ComboRow::builder()
        .title(current_lang.pref_font_family_title())
        .subtitle(current_lang.pref_font_family_subtitle())
        .model(&gtk4::StringList::new(&font_options_refs))
        .build();

    let saved_font_family = settings.borrow().editor_font_family.clone();
    let initial_font_idx = font_options
        .iter()
        .position(|name| name.eq_ignore_ascii_case(&saved_font_family))
        .unwrap_or(0);
    font_family_row.set_selected(initial_font_idx as u32);

    {
        let settings = settings.clone();
        let provider = editor_css_provider.clone();
        let docs = docs.clone();
        let font_options = font_options.clone();
        font_family_row.connect_selected_notify(move |row| {
            let index = row.selected() as usize;
            let Some(selected_font) = font_options.get(index) else {
                return;
            };

            let (theme, size) = {
                let mut state = settings.borrow_mut();
                state.editor_font_family = selected_font.clone();
                state.save();
                (state.theme.clone(), state.editor_font_size)
            };

            apply_editor_style_css(
                &provider,
                &theme,
                adw::StyleManager::default().is_dark(),
                selected_font,
                size,
            );
            for doc in docs.borrow().iter() {
                doc.apply_editor_options();
                doc.gutter.queue_draw();
            }
        });
    }
    fonts_group.add(&font_family_row);

    let editor_spin = adw::SpinRow::builder()
        .title(current_lang.pref_editor_size_title())
        .adjustment(&gtk4::Adjustment::new(
            settings.borrow().editor_font_size as f64,
            8.0,
            48.0,
            1.0,
            2.0,
            0.0,
        ))
        .build();

    {
        let settings = settings.clone();
        let provider = editor_css_provider.clone();
        let docs = docs.clone();
        editor_spin.connect_changed(move |row| {
            let size = row.value() as u32;
            let (theme, family) = {
                let mut state = settings.borrow_mut();
                state.editor_font_size = size;
                state.save();
                (state.theme.clone(), state.editor_font_family.clone())
            };
            apply_editor_style_css(
                &provider,
                &theme,
                adw::StyleManager::default().is_dark(),
                &family,
                size,
            );
            for doc in docs.borrow().iter() {
                doc.apply_editor_options();
                doc.gutter.queue_draw();
            }
        });
    }
    fonts_group.add(&editor_spin);

    let preview_spin = adw::SpinRow::builder()
        .title(current_lang.pref_preview_size_title())
        .adjustment(&gtk4::Adjustment::new(
            settings.borrow().preview_font_size as f64,
            10.0,
            48.0,
            1.0,
            2.0,
            0.0,
        ))
        .build();

    {
        let settings = settings.clone();
        let docs = docs.clone();
        preview_spin.connect_changed(move |row| {
            settings.borrow_mut().preview_font_size = row.value() as u32;
            settings.borrow().save();
            for doc in docs.borrow().iter() {
                if doc.is_preview_active.get() {
                    doc.refresh_preview();
                }
            }
        });
    }
    fonts_group.add(&preview_spin);

    page.add(&fonts_group);
    window.add(&page);
    window.present();
}

#[cfg(test)]
mod tests {
    use super::shortcut_display_text;

    #[test]
    fn test_shortcut_display_text() {
        assert_eq!(
            shortcut_display_text("<Control><Shift>w"),
            "Ctrl + Shift + W"
        );
        assert_eq!(
            shortcut_display_text("<Control><Shift>l"),
            "Ctrl + Shift + L"
        );
        assert_eq!(
            shortcut_display_text("<Control><Shift>Tab"),
            "Ctrl + Shift + Tab"
        );
        assert_eq!(shortcut_display_text("F5"), "F5");
    }
}
