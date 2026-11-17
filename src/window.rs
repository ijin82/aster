//! Построение главного окна, вкладок и обработчиков действий.
//!
//! Начиная с этого модуля окно не хранит текст документа. Каждая вкладка —
//! это независимый [`crate::document::Document`] со своим буфером, WebKit-
//! предпросмотром и позициями прокрутки. Окно отвечает только за:
//! - заголовочную панель и переключатель режимов;
//! - `adw::TabView` / `adw::TabBar` и работу с вкладками;
//! - меню, горячие клавиши и диалоги;
//! - восстановление и сохранение сессии.

use adw::prelude::*;
use gtk4::gio;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crate::document::{self, Document};
use crate::i18n::Language;
use crate::settings::AppSettings;

mod actions;
mod dialogs;
mod menus;
mod preferences;
mod session;
mod style;
mod tabs;

use actions::{install_actions, ActionContext};
use dialogs::{save_document, save_documents_sequentially, show_unsaved_changes_dialog};
use menus::{rebuild_main_menu, rebuild_tab_menu};
use style::{apply_color_scheme, apply_editor_style_css, disable_sound_feedback};
use tabs::{active_document, parent_directory};

/// Создает главное окно приложения.
///
/// * `initial_files` — файлы, переданные в командной строке или через `open`.
/// * `restore_session` — если `true` и файлы не переданы, открываются вкладки
///   из прошлой сессии (`settings.json`). Если `false`, создается одна
///   безымянная вкладка (либо вкладки для `initial_files`).
pub fn build_window(
    app: &adw::Application,
    initial_files: Vec<PathBuf>,
    restore_session: bool,
) -> adw::ApplicationWindow {
    let settings = Rc::new(RefCell::new(AppSettings::load()));

    // Aster — редактор Markdown, поэтому любые системные звуки интерфейса
    // (error bell, звуки событий и звуковая обратная связь ввода) отключены.
    disable_sound_feedback();

    apply_color_scheme(&settings.borrow().theme);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Aster")
        .icon_name("io.github.aster")
        .default_width(settings.borrow().window_width)
        .default_height(settings.borrow().window_height)
        .build();

    if settings.borrow().window_maximized {
        window.maximize();
    }

    // --- CSS редактора: тема и шрифт ---
    //
    // ВАЖНО: шрифт задается именно CSS-правилом, а не `GtkTextTag`.
    // `GtkTextTag` применяется к диапазону существующих символов, поэтому
    // текст, набранный в пустом буфере или после удаления всего текста,
    // оставался без тега и отображался системным шрифтом. CSS-правило
    // действует на весь виджет, включая любой новый символ.
    let editor_css_provider = gtk4::CssProvider::new();
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &editor_css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );
    }
    {
        let settings = settings.borrow();
        apply_editor_style_css(
            &editor_css_provider,
            &settings.theme,
            adw::StyleManager::default().is_dark(),
            &settings.editor_font_family,
            settings.editor_font_size,
        );
    }

    let current_lang = Language::from_code(&settings.borrow().language);

    // --- Корневая разметка ---
    let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let header_bar = adw::HeaderBar::new();

    let open_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    open_btn.set_tooltip_text(Some(current_lang.tooltip_open()));
    header_bar.pack_start(&open_btn);

    let save_btn = gtk4::Button::from_icon_name("document-save-symbolic");
    save_btn.set_tooltip_text(Some(current_lang.tooltip_save()));
    header_bar.pack_start(&save_btn);

    // Переключатель «Редактор / Просмотр» применяется к активной вкладке.
    let edit_btn = gtk4::ToggleButton::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(current_lang.tooltip_edit_mode())
        .active(true)
        .build();
    let preview_btn = gtk4::ToggleButton::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text(current_lang.tooltip_preview_mode())
        .group(&edit_btn)
        .build();
    let switch_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    switch_box.add_css_class("linked");
    switch_box.append(&edit_btn);
    switch_box.append(&preview_btn);
    header_bar.set_title_widget(Some(&switch_box));

    let search_btn = gtk4::ToggleButton::builder()
        .icon_name("edit-find-symbolic")
        .tooltip_text(current_lang.tooltip_search())
        .build();
    header_bar.pack_end(&search_btn);

    let menu_btn = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(current_lang.tooltip_menu())
        .build();

    let menu = gio::Menu::new();
    rebuild_main_menu(&menu, current_lang);
    menu_btn.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_btn);

    // --- Вкладки ---
    let tab_view = adw::TabView::new();
    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(&tab_view));
    // При одной вкладке полоса вкладок скрыта, при двух и более — появляется.
    tab_bar.set_autohide(true);

    let new_tab_btn = gtk4::Button::from_icon_name("tab-new-symbolic");
    new_tab_btn.add_css_class("flat");
    new_tab_btn.set_tooltip_text(Some(current_lang.tooltip_new_tab()));
    tab_bar.set_end_action_widget(Some(&new_tab_btn));

    let tab_menu = gio::Menu::new();
    rebuild_tab_menu(&tab_menu, current_lang);
    tab_view.set_menu_model(Some(&tab_menu));

    root_box.append(&header_bar);
    root_box.append(&tab_bar);
    root_box.append(&tab_view);

    // Неблокирующие уведомления (например, ошибки чтения/записи файла)
    // показываются поверх всего содержимого окна.
    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&root_box));
    window.set_content(Some(&toast_overlay));

    let show_toast: Rc<dyn Fn(&str)> = {
        let toast_overlay = toast_overlay.clone();
        Rc::new(move |message: &str| {
            let toast = adw::Toast::new(message);
            toast.set_timeout(6);
            toast_overlay.add_toast(toast);
        })
    };

    // --- Разделяемое состояние окна ---
    let docs: Rc<RefCell<Vec<Rc<Document>>>> = Rc::new(RefCell::new(Vec::new()));
    // Каталог последнего открытого/активного файла. Нужен, чтобы диалог
    // открытия начинался в той же папке, даже если текущая вкладка безымянная.
    let last_open_directory: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    // Стек закрытых файлов текущей сессии для Ctrl+Shift+T.
    let closed_tabs: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
    const MAX_CLOSED_TABS: usize = 50;
    let suppress_mode_sync = Rc::new(Cell::new(false));
    let suppress_search_sync = Rc::new(Cell::new(false));
    let is_force_closing = Rc::new(Cell::new(false));
    let is_closing = Rc::new(Cell::new(false));

    // --- Обновление заголовка и кнопок для активной вкладки ---
    let refresh_active_ui = {
        let window = window.clone();
        let save_btn = save_btn.clone();
        let edit_btn = edit_btn.clone();
        let preview_btn = preview_btn.clone();
        let search_btn = search_btn.clone();
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let settings = settings.clone();
        let suppress_mode_sync = suppress_mode_sync.clone();
        let suppress_search_sync = suppress_search_sync.clone();

        Rc::new(move || {
            let lang = Language::from_code(&settings.borrow().language);
            let Some(doc) = active_document(&tab_view, &docs) else {
                window.set_title(Some("Aster"));
                save_btn.set_sensitive(false);
                edit_btn.set_sensitive(false);
                preview_btn.set_sensitive(false);
                search_btn.set_sensitive(false);
                return;
            };

            save_btn.set_sensitive(true);
            edit_btn.set_sensitive(true);
            preview_btn.set_sensitive(true);
            search_btn.set_sensitive(true);

            window.set_title(Some(&format!("{} — Aster", doc.window_title(lang))));

            let is_modified = doc.is_modified.get();
            if is_modified {
                save_btn.add_css_class("suggested-action");
                save_btn.set_tooltip_text(Some(lang.tooltip_save_unsaved()));
            } else {
                save_btn.remove_css_class("suggested-action");
                save_btn.set_tooltip_text(Some(lang.tooltip_save()));
            }

            // Синхронизируем кнопки режима с состоянием активной вкладки.
            // Флаг подавляет обработчики `toggled`, чтобы программное
            // переключение не вызывало повторный рендеринг предпросмотра.
            suppress_mode_sync.set(true);
            edit_btn.set_active(!doc.is_preview_active.get());
            preview_btn.set_active(doc.is_preview_active.get());
            suppress_mode_sync.set(false);

            suppress_search_sync.set(true);
            search_btn.set_active(doc.search_bar.is_search_mode());
            suppress_search_sync.set(false);
        })
    };

    // --- Сохранение сессии и позиций всех документов ---
    let persist_session = {
        let settings = settings.clone();
        let docs = docs.clone();
        let tab_view = tab_view.clone();
        Rc::new(move || {
            session::persist_session(&settings, &docs, &tab_view);
        })
    };

    // --- Добавление документа во вкладку ---
    let add_document: Rc<dyn Fn(Option<PathBuf>, bool) -> Option<adw::TabPage>> = {
        let settings = settings.clone();
        let docs = docs.clone();
        let tab_view = tab_view.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let suppress_search_sync = suppress_search_sync.clone();
        let search_btn = search_btn.clone();
        let show_toast = show_toast.clone();
        let settings_for_error = settings.clone();
        let last_open_directory = last_open_directory.clone();

        Rc::new(move |path: Option<PathBuf>, restore_mode: bool| {
            // Если файл уже открыт, не создаем дубликат, а переключаемся на него.
            if let Some(ref path) = path {
                let key = document::get_file_canonical_key(path);
                let existing = docs
                    .borrow()
                    .iter()
                    .find(|doc| doc.canonical_key().as_deref() == Some(key.as_str()))
                    .cloned();
                if let Some(doc) = existing {
                    if let Some(page) = doc.tab_page.borrow().clone() {
                        tab_view.set_selected_page(&page);
                        return Some(page);
                    }
                }
            }

            let path_for_directory = path.as_deref().and_then(parent_directory);
            let doc = match Document::new(settings.clone(), path, restore_mode) {
                Ok(doc) => doc,
                Err(error) => {
                    let lang = Language::from_code(&settings_for_error.borrow().language);
                    show_toast(&format!("{}: {error}", lang.error_open_file()));
                    return None;
                }
            };
            if let Some(directory) = path_for_directory {
                *last_open_directory.borrow_mut() = Some(directory);
            }
            let page = tab_view.append(&doc.root);
            *doc.tab_page.borrow_mut() = Some(page.clone());
            doc.update_tab_title();

            // Изменение текста активной вкладки должно сразу обновить
            // точку в заголовке и подсветку кнопки «Сохранить».
            let weak_doc = Rc::downgrade(&doc);
            let tab_view_for_change = tab_view.clone();
            let refresh_for_change = refresh_active_ui.clone();
            doc.buffer.connect_changed(move |_| {
                if let Some(doc) = weak_doc.upgrade() {
                    let is_active = tab_view_for_change
                        .selected_page()
                        .map(|page| doc.matches_page(&page))
                        .unwrap_or(false);
                    if is_active {
                        refresh_for_change();
                    }
                }
            });

            // Закрытие панели поиска клавишей Esc должно сбрасывать
            // соответствующую кнопку в заголовке.
            let weak_doc = Rc::downgrade(&doc);
            let tab_view_for_search = tab_view.clone();
            let search_btn_for_sync = search_btn.clone();
            let suppress_for_search = suppress_search_sync.clone();
            doc.search_bar
                .connect_search_mode_enabled_notify(move |bar| {
                    if let Some(doc) = weak_doc.upgrade() {
                        let is_active = tab_view_for_search
                            .selected_page()
                            .map(|page| doc.matches_page(&page))
                            .unwrap_or(false);
                        if is_active && !suppress_for_search.get() {
                            suppress_for_search.set(true);
                            search_btn_for_sync.set_active(bar.is_search_mode());
                            suppress_for_search.set(false);
                        }
                    }
                });

            docs.borrow_mut().push(doc.clone());
            tab_view.set_selected_page(&page);
            refresh_active_ui();

            if doc.is_preview_active.get() {
                doc.web_view.grab_focus();
            } else {
                doc.text_view.grab_focus();
            }

            Some(page)
        })
    };

    // --- Локализация заголовка, меню и всех открытых документов ---
    let update_ui_language = {
        let open_btn = open_btn.clone();
        let save_btn = save_btn.clone();
        let edit_btn = edit_btn.clone();
        let preview_btn = preview_btn.clone();
        let search_btn = search_btn.clone();
        let menu_btn = menu_btn.clone();
        let new_tab_btn = new_tab_btn.clone();
        let menu = menu.clone();
        let tab_menu = tab_menu.clone();
        let docs = docs.clone();
        let refresh_active_ui = refresh_active_ui.clone();

        Rc::new(move |lang: Language| {
            open_btn.set_tooltip_text(Some(lang.tooltip_open()));
            save_btn.set_tooltip_text(Some(lang.tooltip_save()));
            edit_btn.set_tooltip_text(Some(lang.tooltip_edit_mode()));
            preview_btn.set_tooltip_text(Some(lang.tooltip_preview_mode()));
            search_btn.set_tooltip_text(Some(lang.tooltip_search()));
            menu_btn.set_tooltip_text(Some(lang.tooltip_menu()));
            new_tab_btn.set_tooltip_text(Some(lang.tooltip_new_tab()));

            rebuild_main_menu(&menu, lang);
            rebuild_tab_menu(&tab_menu, lang);

            for doc in docs.borrow().iter() {
                doc.apply_language(lang);
            }
            refresh_active_ui();
        })
    };

    // --- Диалог открытия файла ---
    let show_open_dialog = {
        let window = window.clone();
        let settings = settings.clone();
        let add_document = add_document.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let last_open_directory = last_open_directory.clone();

        Rc::new(move || {
            let lang = Language::from_code(&settings.borrow().language);
            let dialog = gtk4::FileDialog::builder()
                .title(lang.dialog_open_title())
                .build();

            // Начинаем с папки активного документа. Если активная вкладка
            // безымянная — с папки последнего открытого файла.
            let initial_directory = active_document(&tab_view, &docs)
                .and_then(|doc| {
                    doc.current_file
                        .borrow()
                        .as_ref()
                        .and_then(|path| parent_directory(path))
                })
                .or_else(|| last_open_directory.borrow().clone());
            if let Some(directory) = initial_directory {
                dialog.set_initial_folder(Some(&gio::File::for_path(directory)));
            }

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some(lang.filter_markdown()));
            filter.add_pattern("*.md");
            filter.add_pattern("*.markdown");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let add_document = add_document.clone();
            let persist_session = persist_session.clone();
            let refresh_active_ui = refresh_active_ui.clone();
            dialog.open_multiple(
                Some(&window),
                gio::Cancellable::NONE,
                move |res| {
                    let Ok(files) = res else {
                        return;
                    };

                    for index in 0..files.n_items() {
                        let Some(object) = files.item(index) else {
                            continue;
                        };
                        let Ok(file) = object.downcast::<gio::File>() else {
                            continue;
                        };
                        if let Some(path) = file.path() {
                            add_document(Some(path), false);
                        }
                    }

                    persist_session();
                    refresh_active_ui();
                },
            );
        })
    };

    // --- Сохранение активного документа ---
    let save_active_document = {
        let window = window.clone();
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let settings = settings.clone();
        let show_toast = show_toast.clone();
        let last_open_directory = last_open_directory.clone();

        Rc::new(move |save_as: bool| {
            let Some(doc) = active_document(&tab_view, &docs) else {
                return;
            };
            let persist_session = persist_session.clone();
            let refresh_active_ui = refresh_active_ui.clone();
            let settings = settings.clone();
            let show_toast = show_toast.clone();
            let last_open_directory = last_open_directory.clone();
            let doc_for_directory = doc.clone();
            save_document(
                &window,
                &doc,
                save_as,
                Rc::new(move |result| match result {
                    Ok(true) => {
                        if let Some(directory) = doc_for_directory
                            .current_file
                            .borrow()
                            .as_ref()
                            .and_then(|path| parent_directory(path))
                        {
                            *last_open_directory.borrow_mut() = Some(directory);
                        }
                        persist_session();
                        refresh_active_ui();
                    }
                    Ok(false) => {}
                    Err(error) => {
                        let lang = Language::from_code(&settings.borrow().language);
                        show_toast(&format!("{}: {error}", lang.error_save_file()));
                    }
                }),
            );
        })
    };

    let actions = install_actions(ActionContext {
        window: window.clone(),
        header_bar: header_bar.clone(),
        tab_view: tab_view.clone(),
        docs: docs.clone(),
        settings: settings.clone(),
        editor_css_provider: editor_css_provider.clone(),
        add_document: add_document.clone(),
        persist_session: persist_session.clone(),
        refresh_active_ui: refresh_active_ui.clone(),
        show_open_dialog: show_open_dialog.clone(),
        save_active_document: save_active_document.clone(),
        update_ui_language: update_ui_language.clone(),
        closed_tabs: closed_tabs.clone(),
    });
    window.insert_action_group("win", Some(&actions));

    // --- Обработчики кнопок заголовка ---
    {
        let show_open_dialog = show_open_dialog.clone();
        open_btn.connect_clicked(move |_| show_open_dialog());
    }
    {
        let save_active_document = save_active_document.clone();
        save_btn.connect_clicked(move |_| save_active_document(false));
    }
    {
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let suppress_mode_sync = suppress_mode_sync.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        edit_btn.connect_toggled(move |btn| {
            if !btn.is_active() || suppress_mode_sync.get() {
                return;
            }
            if let Some(doc) = active_document(&tab_view, &docs) {
                doc.switch_to_editor();
                persist_session();
                refresh_active_ui();
            }
        });
    }
    {
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let suppress_mode_sync = suppress_mode_sync.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        preview_btn.connect_toggled(move |btn| {
            if !btn.is_active() || suppress_mode_sync.get() {
                return;
            }
            if let Some(doc) = active_document(&tab_view, &docs) {
                doc.switch_to_preview();
                persist_session();
                refresh_active_ui();
            }
        });
    }
    {
        let tab_view = tab_view.clone();
        let docs = docs.clone();
        let suppress_search_sync = suppress_search_sync.clone();
        search_btn.connect_toggled(move |btn| {
            if suppress_search_sync.get() {
                return;
            }
            if let Some(doc) = active_document(&tab_view, &docs) {
                doc.search_bar.set_search_mode(btn.is_active());
                if btn.is_active() {
                    doc.search_entry.grab_focus();
                }
            }
        });
    }
    {
        let add_document = add_document.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        new_tab_btn.connect_clicked(move |_| {
            add_document(None, false);
            persist_session();
            refresh_active_ui();
        });
    }

    // --- Восстановление сессии / начальных файлов ---
    let mut opened_pages: Vec<adw::TabPage> = Vec::new();
    let mut selected_page: Option<adw::TabPage> = None;

    if restore_session {
        let (session_files, active_index) = {
            let state = settings.borrow();
            (state.session_files.clone(), state.session_active_document)
        };

        for (index, path) in session_files.iter().enumerate() {
            let path = PathBuf::from(path);
            if !path.is_file() {
                continue;
            }
            if let Some(page) = add_document(Some(path), true) {
                if index == active_index {
                    selected_page = Some(page.clone());
                }
                opened_pages.push(page);
            }
        }
    }

    if opened_pages.is_empty() {
        for path in initial_files {
            if !path.is_file() {
                continue;
            }
            if let Some(page) = add_document(Some(path), false) {
                opened_pages.push(page);
            }
        }
    }

    if opened_pages.is_empty() {
        if let Some(page) = add_document(None, false) {
            opened_pages.push(page);
        }
    }

    // Активной делаем вкладку из сессии, иначе первую открытую.
    let active_page = selected_page.or_else(|| opened_pages.first().cloned());
    if let Some(page) = active_page {
        tab_view.set_selected_page(&page);
    }
    if let Some(doc) = active_document(&tab_view, &docs) {
        if let Some(directory) = doc
            .current_file
            .borrow()
            .as_ref()
            .and_then(|path| parent_directory(path))
        {
            *last_open_directory.borrow_mut() = Some(directory);
        }
    }

    // --- Сигналы вкладок (подключаем после восстановления стартовых вкладок) ---
    {
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let tab_view_for_directory = tab_view.clone();
        let docs_for_directory = docs.clone();
        let last_open_directory = last_open_directory.clone();
        tab_view.connect_selected_page_notify(move |_| {
            if let Some(doc) = active_document(&tab_view_for_directory, &docs_for_directory) {
                if let Some(directory) = doc
                    .current_file
                    .borrow()
                    .as_ref()
                    .and_then(|path| parent_directory(path))
                {
                    *last_open_directory.borrow_mut() = Some(directory);
                }
            }
            persist_session();
            refresh_active_ui();
        });
    }

    {
        let window = window.clone();
        let docs = docs.clone();
        let settings = settings.clone();
        let tab_view = tab_view.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let show_toast_for_close_page = show_toast.clone();

        tab_view.connect_close_page(move |view, page| {
            let doc = docs
                .borrow()
                .iter()
                .find(|doc| doc.matches_page(page))
                .cloned();

            let Some(doc) = doc else {
                return glib::Propagation::Proceed;
            };

            // Чистая вкладка закрывается стандартным обработчиком TabView.
            if !doc.is_modified.get() {
                return glib::Propagation::Proceed;
            }

            let lang = Language::from_code(&settings.borrow().language);
            let view_for_cancel = view.clone();
            let page_for_cancel = page.clone();
            let view_for_discard = view.clone();
            let page_for_discard = page.clone();
            let view_for_save = view.clone();
            let page_for_save = page.clone();
            let window_for_save = window.clone();
            let doc_for_discard = doc.clone();
            let doc_for_save = doc.clone();
            let settings_for_discard = settings.clone();
            let persist_for_discard = persist_session.clone();
            let refresh_for_discard = refresh_active_ui.clone();
            let persist_for_save = persist_session.clone();
            let refresh_for_save = refresh_active_ui.clone();
            let show_toast_for_save = show_toast_for_close_page.clone();
            let settings_for_save = settings.clone();

            show_unsaved_changes_dialog(
                &window,
                lang.tab_close_heading(),
                lang.tab_close_body(),
                lang,
                lang.btn_save(),
                move || {
                    view_for_cancel.close_page_finish(&page_for_cancel, false);
                },
                move || {
                    doc_for_discard
                        .export_state(&mut settings_for_discard.borrow_mut());
                    view_for_discard.close_page_finish(&page_for_discard, true);
                    persist_for_discard();
                    refresh_for_discard();
                },
                move || {
                    save_document(
                        &window_for_save,
                        &doc_for_save,
                        false,
                        Rc::new(move |result| match result {
                            Ok(true) => {
                                view_for_save.close_page_finish(&page_for_save, true);
                                persist_for_save();
                                refresh_for_save();
                            }
                            Ok(false) => {
                                view_for_save.close_page_finish(&page_for_save, false);
                            }
                            Err(error) => {
                                view_for_save.close_page_finish(&page_for_save, false);
                                let lang = Language::from_code(&settings_for_save.borrow().language);
                                show_toast_for_save(&format!(
                                    "{}: {error}",
                                    lang.error_save_file()
                                ));
                            }
                        }),
                    );
                },
            );

            glib::Propagation::Stop
        });
    }

    {
        let window = window.clone();
        let docs = docs.clone();
        let settings = settings.clone();
        let persist_session = persist_session.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let is_closing = is_closing.clone();
        let closed_tabs = closed_tabs.clone();

        tab_view.connect_page_detached(move |_, page, _| {
            // При закрытии окна вкладки отсоединяются автоматически; сессию
            // уже сохранил обработчик close_request, поэтому не затираем ее.
            if is_closing.get() {
                return;
            }

            let removed = {
                let mut list = docs.borrow_mut();
                list.iter()
                    .position(|doc| doc.matches_page(page))
                    .map(|index| list.remove(index))
            };

            if let Some(doc) = removed {
                if let Some(path) = doc.current_file.borrow().clone() {
                    let mut stack = closed_tabs.borrow_mut();
                    if stack.len() >= MAX_CLOSED_TABS {
                        stack.remove(0);
                    }
                    stack.push(path);
                }
                doc.export_state(&mut settings.borrow_mut());
            }

            persist_session();
            refresh_active_ui();

            // Закрытие последней вкладки закрывает окно (как в GNOME Text Editor).
            if docs.borrow().is_empty() {
                is_closing.set(true);
                persist_session();
                window.close();
            }
        });
    }

    // --- Сохранение геометрии окна ---
    let save_window_geometry = {
        let settings = settings.clone();
        Rc::new(move |win: &adw::ApplicationWindow| {
            session::save_window_geometry(&settings, win);
        })
    };

    // --- Закрытие окна с проверкой всех вкладок ---
    window.connect_close_request({
        let docs = docs.clone();
        let settings = settings.clone();
        let persist_session = persist_session.clone();
        let save_window_geometry = save_window_geometry.clone();
        let is_force_closing = is_force_closing.clone();
        let is_closing = is_closing.clone();
        let show_toast = show_toast.clone();

        move |win| {
            if is_force_closing.get() {
                return glib::Propagation::Proceed;
            }

            let modified: Vec<Rc<Document>> = docs
                .borrow()
                .iter()
                .filter(|doc| doc.is_modified.get())
                .cloned()
                .collect();

            if modified.is_empty() {
                is_closing.set(true);
                persist_session();
                save_window_geometry(win);
                return glib::Propagation::Proceed;
            }

            let lang = Language::from_code(&settings.borrow().language);

            let win_for_discard = win.clone();
            let win_for_save = win.clone();
            let persist_discard = persist_session.clone();
            let persist_save = persist_session.clone();
            let geom_discard = save_window_geometry.clone();
            let geom_save = save_window_geometry.clone();
            let force_discard = is_force_closing.clone();
            let force_save = is_force_closing.clone();
            let closing_discard = is_closing.clone();
            let closing_save = is_closing.clone();
            let docs_for_save = modified.clone();
            let settings_for_save = settings.clone();
            let show_toast_for_save = show_toast.clone();

            show_unsaved_changes_dialog(
                win,
                lang.unsaved_window_heading(),
                lang.unsaved_window_body(),
                lang,
                lang.btn_save_all(),
                || {
                    // Отмена: окно остается открытым, ничего не меняем.
                },
                move || {
                    closing_discard.set(true);
                    persist_discard();
                    geom_discard(&win_for_discard);
                    force_discard.set(true);
                    win_for_discard.close();
                },
                move || {
                    let win = win_for_save.clone();
                    let settings_for_save = settings_for_save.clone();
                    let show_toast_for_save = show_toast_for_save.clone();
                    save_documents_sequentially(
                        win_for_save.clone(),
                        docs_for_save.clone(),
                        0,
                        Rc::new(move |result| match result {
                            Ok(true) => {
                                closing_save.set(true);
                                persist_save();
                                geom_save(&win);
                                force_save.set(true);
                                win.close();
                            }
                            Ok(false) => {}
                            Err(error) => {
                                let lang = Language::from_code(&settings_for_save.borrow().language);
                                show_toast_for_save(&format!(
                                    "{}: {error}",
                                    lang.error_save_file()
                                ));
                            }
                        }),
                    );
                },
            );

            glib::Propagation::Stop
        }
    });

    // Первоначальная синхронизация заголовка и сохранение сессии.
    refresh_active_ui();
    persist_session();

    // --- Горячие клавиши приложения ---
    app.set_accels_for_action("win.save", &["<Control>s"]);
    app.set_accels_for_action("win.save_as", &["<Control><Shift>s"]);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    app.set_accels_for_action("win.new_tab", &["<Control>t"]);
    app.set_accels_for_action("win.reopen_tab", &["<Control><Shift>t"]);
    app.set_accels_for_action("win.toggle_meditation", &["<Control><Shift>h"]);
    app.set_accels_for_action("win.close_tab", &["<Control>w"]);
    app.set_accels_for_action("win.next_tab", &["<Control>Tab"]);
    app.set_accels_for_action(
        "win.previous_tab",
        &["<Control><Shift>Tab", "<Control>ISO_Left_Tab"],
    );
    app.set_accels_for_action("win.toggle_preview", &["<Control>e"]);
    app.set_accels_for_action("win.find", &["<Control>f"]);
    app.set_accels_for_action("win.find_replace", &["<Control>r"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.close", &["<Control>q", "<Control>F4"]);

    window
}

#[cfg(test)]
mod tests {
    use super::tabs::{select_tab_wrapped, tab_page_at_position, wrapped_tab_position};
    use super::*;
    use std::path::Path;

    #[test]
    fn test_parent_directory_for_open_dialog() {
        assert_eq!(
            parent_directory(Path::new("/tmp/a-tests/01-prose-ru.md")),
            Some(PathBuf::from("/tmp/a-tests"))
        );
        assert_eq!(
            parent_directory(Path::new("/tmp/a.md")),
            Some(PathBuf::from("/tmp"))
        );
        assert_eq!(parent_directory(Path::new("a.md")), None);
    }

    #[test]
    fn test_tab_position_wraps_forward_and_backward() {
        // Вперед: 0 -> 1 -> 2 -> 0.
        assert_eq!(wrapped_tab_position(0, 3, true), Some(1));
        assert_eq!(wrapped_tab_position(1, 3, true), Some(2));
        assert_eq!(wrapped_tab_position(2, 3, true), Some(0));

        // Назад: 2 -> 1 -> 0 -> 2.
        assert_eq!(wrapped_tab_position(2, 3, false), Some(1));
        assert_eq!(wrapped_tab_position(1, 3, false), Some(0));
        assert_eq!(wrapped_tab_position(0, 3, false), Some(2));

        assert_eq!(wrapped_tab_position(-1, 3, true), Some(0));
        assert_eq!(wrapped_tab_position(-1, 3, false), Some(2));
        assert_eq!(wrapped_tab_position(0, 0, true), None);
    }

    #[gtk4::test]
    fn test_tab_view_selection_wraps_at_boundaries() {
        let tab_view = adw::TabView::new();
        for index in 0..3 {
            tab_view.append(&gtk4::Label::new(Some(&index.to_string())));
        }

        let first = tab_page_at_position(&tab_view, 0).expect("first tab");
        let last = tab_page_at_position(&tab_view, 2).expect("last tab");

        // С последней вкладки вперед переходим на первую.
        tab_view.set_selected_page(&last);
        select_tab_wrapped(&tab_view, true);
        assert_eq!(
            tab_view.page_position(&tab_view.selected_page().expect("selected tab")),
            0
        );

        // С первой вкладки назад переходим на последнюю.
        tab_view.set_selected_page(&first);
        select_tab_wrapped(&tab_view, false);
        assert_eq!(
            tab_view.page_position(&tab_view.selected_page().expect("selected tab")),
            2
        );
    }

    #[gtk4::test]
    fn test_all_gtk_sound_feedback_is_disabled() {
        disable_sound_feedback();
        let Some(settings) = gtk4::Settings::default() else {
            return;
        };

        assert!(!settings.is_gtk_error_bell());
        assert!(!settings.is_gtk_enable_event_sounds());
        assert!(!settings.is_gtk_enable_input_feedback_sounds());
    }
}
