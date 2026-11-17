//! Регистрация действий окна `win.*` и связанных с ними горячих клавиш.
//!
//! Контекст передается сюда, чтобы `window.rs` не разрастался десятками
//! клонов `Rc` и замыканий вокруг каждой кнопки меню.

use adw::prelude::*;
use gtk4::{gio, glib};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::document::Document;
use crate::i18n::Language;
use crate::settings::AppSettings;
use crate::shortcuts;

use super::dialogs::show_about_dialog;
use super::preferences::show_preferences_dialog;
use super::tabs::{active_document, select_tab_wrapped};

/// Все зависимости, необходимые для регистрации действий.
pub(crate) struct ActionContext {
    pub window: adw::ApplicationWindow,
    pub header_bar: adw::HeaderBar,
    pub tab_view: adw::TabView,
    pub docs: Rc<RefCell<Vec<Rc<Document>>>>,
    pub settings: Rc<RefCell<AppSettings>>,
    pub editor_css_provider: gtk4::CssProvider,
    pub add_document: Rc<dyn Fn(Option<PathBuf>, bool) -> Option<adw::TabPage>>,
    pub persist_session: Rc<dyn Fn()>,
    pub refresh_active_ui: Rc<dyn Fn()>,
    pub show_open_dialog: Rc<dyn Fn()>,
    pub save_active_document: Rc<dyn Fn(bool)>,
    pub update_ui_language: Rc<dyn Fn(Language)>,
    pub closed_tabs: Rc<RefCell<Vec<PathBuf>>>,
}

/// Создает группу действий `win.*`.
pub(crate) fn install_actions(ctx: ActionContext) -> gio::SimpleActionGroup {
    let ActionContext {
        window,
        header_bar,
        tab_view,
        docs,
        settings,
        editor_css_provider,
        add_document,
        persist_session,
        refresh_active_ui,
        show_open_dialog,
        save_active_document,
        update_ui_language,
        closed_tabs,
    } = ctx;

    // --- Действия окна ---
    let actions = gio::SimpleActionGroup::new();

    let save_action = gio::SimpleAction::new("save", None);
    {
        let save_active_document = save_active_document.clone();
        save_action.connect_activate(move |_, _| save_active_document(false));
    }
    actions.add_action(&save_action);

    let save_as_action = gio::SimpleAction::new("save_as", None);
    {
        let save_active_document = save_active_document.clone();
        save_as_action.connect_activate(move |_, _| save_active_document(true));
    }
    actions.add_action(&save_as_action);

    let open_action = gio::SimpleAction::new("open", None);
    {
        let show_open_dialog = show_open_dialog.clone();
        open_action.connect_activate(move |_, _| show_open_dialog());
    }
    actions.add_action(&open_action);

    let new_tab_action = gio::SimpleAction::new("new_tab", None);
    {
        let add_document = add_document.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        new_tab_action.connect_activate(move |_, _| {
            add_document(None, false);
            persist_session();
            refresh_active_ui();
        });
    }
    actions.add_action(&new_tab_action);

    let reopen_tab_action = gio::SimpleAction::new("reopen_tab", None);
    {
        let closed_tabs = closed_tabs.clone();
        let add_document = add_document.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        reopen_tab_action.connect_activate(move |_, _| {
            // Пропускаем файлы, которые успели удалить с диска, и
            // восстанавливаем ближайшую существующую вкладку.
            while let Some(path) = closed_tabs.borrow_mut().pop() {
                if path.is_file() {
                    add_document(Some(path), false);
                    persist_session();
                    refresh_active_ui();
                    break;
                }
            }
        });
    }
    actions.add_action(&reopen_tab_action);

    let meditation_action = gio::SimpleAction::new("toggle_meditation", None);
    {
        let header_bar = header_bar.clone();
        meditation_action.connect_activate(move |_, _| {
            header_bar.set_visible(!header_bar.is_visible());
        });
    }
    actions.add_action(&meditation_action);

    let close_tab_action = gio::SimpleAction::new("close_tab", None);
    {
        let tab_view = tab_view.clone();
        close_tab_action.connect_activate(move |_, _| {
            if let Some(page) = tab_view.selected_page() {
                tab_view.close_page(&page);
            }
        });
    }
    actions.add_action(&close_tab_action);

    // Традиционное для вкладок переключение по кругу: Ctrl+Tab и Ctrl+Shift+Tab.
    // Эти действия не связаны с режимом предпросмотра — тот теперь на Ctrl+E.
    let next_tab_action = gio::SimpleAction::new("next_tab", None);
    {
        let tab_view = tab_view.clone();
        next_tab_action.connect_activate(move |_, _| {
            select_tab_wrapped(&tab_view, true);
        });
    }
    actions.add_action(&next_tab_action);

    let previous_tab_action = gio::SimpleAction::new("previous_tab", None);
    {
        let tab_view = tab_view.clone();
        previous_tab_action.connect_activate(move |_, _| {
            select_tab_wrapped(&tab_view, false);
        });
    }
    actions.add_action(&previous_tab_action);

    let close_window_action = gio::SimpleAction::new("close", None);
    {
        let window = window.clone();
        close_window_action.connect_activate(move |_, _| window.close());
    }
    actions.add_action(&close_window_action);

    // Открытие файла внешним процессом через D-Bus. Параметр — путь к файлу.
    let open_document_action = gio::SimpleAction::new("open_document", Some(glib::VariantTy::STRING));
    {
        let add_document = add_document.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        open_document_action.connect_activate(move |_, param| {
            if let Some(path) = param.and_then(|value| value.get::<String>()) {
                add_document(Some(PathBuf::from(path)), false);
                persist_session();
                refresh_active_ui();
            }
        });
    }
    actions.add_action(&open_document_action);

    let toggle_preview_action = gio::SimpleAction::new("toggle_preview", None);
    {
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        toggle_preview_action.connect_activate(move |_, _| {
            if let Some(doc) = active_document(&tab_view, &docs) {
                let preview = !doc.is_preview_active.get();
                doc.set_preview_mode(preview);
                persist_session();
                refresh_active_ui();
            }
        });
    }
    actions.add_action(&toggle_preview_action);

    let find_action = gio::SimpleAction::new("find", None);
    {
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        find_action.connect_activate(move |_, _| {
            if let Some(doc) = active_document(&tab_view, &docs) {
                let active = !doc.search_bar.is_search_mode();
                doc.search_bar.set_search_mode(active);
                if active {
                    doc.search_entry.grab_focus();
                }
                refresh_active_ui();
            }
        });
    }
    actions.add_action(&find_action);

    let find_replace_action = gio::SimpleAction::new("find_replace", None);
    {
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        find_replace_action.connect_activate(move |_, _| {
            if let Some(doc) = active_document(&tab_view, &docs) {
                doc.open_replace_mode();
                refresh_active_ui();
            }
        });
    }
    actions.add_action(&find_replace_action);

    let preferences_action = gio::SimpleAction::new("preferences", None);
    {
        let window = window.clone();
        let settings = settings.clone();
        let docs = docs.clone();
        let editor_css_provider = editor_css_provider.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let update_ui_language = update_ui_language.clone();
        preferences_action.connect_activate(move |_, _| {
            show_preferences_dialog(
                &window,
                settings.clone(),
                docs.clone(),
                editor_css_provider.clone(),
                refresh_active_ui.clone(),
                update_ui_language.clone(),
            );
        });
    }
    actions.add_action(&preferences_action);

    let about_action = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        let settings = settings.clone();
        about_action.connect_activate(move |_, _| {
            let lang = Language::from_code(&settings.borrow().language);
            show_about_dialog(&window, lang);
        });
    }
    actions.add_action(&about_action);



    install_shortcut_translation(&window);

    actions
}


/// Перехватывает Ctrl-комбинации и переводит hardware keycode в латинскую
/// клавишу. Это делает хоткеи независимыми от активной раскладки.
fn install_shortcut_translation(window: &adw::ApplicationWindow) {
    let controller = gtk4::EventControllerKey::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let window_for_controller = window.clone();
    controller.connect_key_pressed(move |_, _keyval, keycode, state| {
        if !state.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
            || state.contains(gtk4::gdk::ModifierType::ALT_MASK)
        {
            return glib::Propagation::Proceed;
        }

        let Some(key) = shortcuts::latin_char_for_keycode(keycode) else {
            return glib::Propagation::Proceed;
        };
        let shift = state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
        let Some(action) = shortcuts::ctrl_shortcut_action(key, shift) else {
            return glib::Propagation::Proceed;
        };

        let _ =
            gtk4::prelude::WidgetExt::activate_action(&window_for_controller, action, None);
        glib::Propagation::Stop
    });

    window.add_controller(controller);
}
