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
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::document::{self, Document};
use crate::i18n::Language;
use crate::settings::AppSettings;

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
            let snapshot: Vec<Rc<Document>> = docs.borrow().clone();
            let selected = tab_view.selected_page();
            let mut state = settings.borrow_mut();

            for doc in &snapshot {
                doc.export_state(&mut state);
            }

            let mut files = Vec::new();
            let mut active_index = 0usize;
            for doc in &snapshot {
                if let Some(key) = doc.canonical_key() {
                    if selected
                        .as_ref()
                        .map(|page| doc.matches_page(page))
                        .unwrap_or(false)
                    {
                        active_index = files.len();
                    }
                    files.push(key);
                }
            }

            state.session_files = files;
            state.session_active_document = active_index;
            state.save();
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
            filter.add_pattern("*.rmd");
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
            let is_max = win.is_maximized();
            {
                let mut state = settings.borrow_mut();
                state.window_maximized = is_max;
                if !is_max {
                    state.window_width = win.default_width();
                    state.window_height = win.default_height();
                }
            }
            settings.borrow().save();
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
    app.set_accels_for_action("win.close_tab", &["<Control>w"]);
    app.set_accels_for_action("win.next_tab", &["<Control>Tab"]);
    app.set_accels_for_action(
        "win.previous_tab",
        &["<Control><Shift>Tab", "<Control>ISO_Left_Tab"],
    );
    app.set_accels_for_action("win.toggle_preview", &["<Control>e"]);
    app.set_accels_for_action("win.find", &["<Control>f"]);
    app.set_accels_for_action("win.close", &["<Control>q", "<Control>F4"]);

    window
}

/// Возвращает каталог файла для начальной папки диалога открытия.
///
/// Для относительного имени без каталога возвращает `None`: иначе GTK
/// попытался бы открыть пустой путь.
fn parent_directory(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// Находит документ, соответствующий выбранной вкладке.
fn active_document(
    tab_view: &adw::TabView,
    docs: &Rc<RefCell<Vec<Rc<Document>>>>,
) -> Option<Rc<Document>> {
    let page = tab_view.selected_page()?;
    docs.borrow()
        .iter()
        .find(|doc| doc.matches_page(&page))
        .cloned()
}

/// Выбирает следующую/предыдущую вкладку, заворачивая список по кругу.
///
/// `adw::TabView::select_next_page()` и `select_previous_page()` не
/// заворачиваются: на последней вкладке они просто возвращают `false`.
/// Поэтому позиция вычисляется вручную, а затем выбирается нужная страница.
fn select_tab_wrapped(tab_view: &adw::TabView, forward: bool) {
    let n_pages = tab_view.n_pages();
    if n_pages <= 0 {
        return;
    }

    let current = tab_view
        .selected_page()
        .map(|page| tab_view.page_position(&page))
        .unwrap_or(-1);

    let Some(target) = wrapped_tab_position(current, n_pages, forward) else {
        return;
    };

    if let Some(page) = tab_page_at_position(tab_view, target as u32) {
        tab_view.set_selected_page(&page);
    }
}

/// Вычисляет позицию следующей вкладки с заворотом.
///
/// Возвращает `None`, если вкладок нет. При некорректной текущей позиции
/// выбирает первую вкладку для движения вперед и последнюю — для движения назад.
fn wrapped_tab_position(current: i32, total: i32, forward: bool) -> Option<i32> {
    if total <= 0 {
        return None;
    }
    if current < 0 || current >= total {
        return Some(if forward { 0 } else { total - 1 });
    }

    if forward {
        Some((current + 1) % total)
    } else {
        Some((current - 1 + total) % total)
    }
}

/// Возвращает страницу вкладки по ее позиции в `adw::TabView`.
fn tab_page_at_position(tab_view: &adw::TabView, position: u32) -> Option<adw::TabPage> {
    let pages = tab_view.pages();
    pages
        .item(position)
        .and_then(|object| object.downcast::<adw::TabPage>().ok())
}

/// Перестраивает главное меню при смене языка интерфейса.
fn rebuild_main_menu(menu: &gio::Menu, lang: Language) {
    menu.remove_all();
    menu.append(Some(lang.menu_new_tab()), Some("win.new_tab"));
    menu.append(Some(lang.menu_save_as()), Some("win.save_as"));
    menu.append(Some(lang.menu_preferences()), Some("win.preferences"));
    menu.append(Some(lang.menu_about()), Some("win.about"));
    menu.append(Some(lang.menu_close_tab()), Some("win.close_tab"));
    menu.append(Some(lang.menu_close()), Some("win.close"));
}

/// Перестраивает контекстное меню вкладок.
fn rebuild_tab_menu(menu: &gio::Menu, lang: Language) {
    menu.remove_all();
    menu.append(Some(lang.menu_new_tab()), Some("win.new_tab"));
    menu.append(Some(lang.menu_close_tab()), Some("win.close_tab"));
}

/// Полностью отключает звуковую обратную связь GTK.
///
/// Одного `gtk-error-bell` недостаточно: GTK4 отдельно проигрывает
/// «event sounds» (например, звуки открытия/закрытия окон) и
/// «input feedback sounds» для клавиатурных действий. В Markdown-редакторе
/// все три категории лишние, поэтому выключаем их и для стандартных
/// настроек, и для настроек текущего GDK-дисплея.
fn disable_sound_feedback() {
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
fn apply_color_scheme(theme: &str) {
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
fn apply_editor_style_css(
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
        }}"
    );
    provider.load_from_data(&css);
}

/// Запрашивает у системы список всех моноширинных семейств шрифтов.
fn get_system_monospace_fonts() -> Vec<String> {
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

/// Сохраняет один документ. При необходимости открывает диалог «Сохранить как».
fn save_document(
    parent: &adw::ApplicationWindow,
    doc: &Rc<Document>,
    save_as: bool,
    on_done: Rc<dyn Fn(Result<bool, String>)>,
) {
    let has_path = doc.current_file.borrow().is_some();

    if !save_as && has_path {
        on_done(doc.save_existing().map(|()| true));
        return;
    }

    let lang = Language::from_code(&doc.settings.borrow().language);
    let initial_name = doc
        .current_file
        .borrow()
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("document.md")
        .to_string();

    let dialog = gtk4::FileDialog::builder()
        .title(lang.dialog_save_title())
        .initial_name(initial_name)
        .build();

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some(lang.filter_markdown()));
    filter.add_pattern("*.md");
    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));

    let doc = doc.clone();
    dialog.save(
        Some(parent),
        gio::Cancellable::NONE,
        move |res| {
            let result = match res {
                Ok(file) => match file.path() {
                    Some(path) => doc.save_to(&path).map(|()| true),
                    None => Ok(false),
                },
                // Закрытие диалога пользователем не является ошибкой.
                Err(_) => Ok(false),
            };
            on_done(result);
        },
    );
}

/// Последовательно сохраняет несколько документов (кнопка «Сохранить все»).
///
/// Если пользователь отменяет «Сохранить как» для одного из безымянных
/// документов, цепочка прерывается и окно не закрывается.
fn save_documents_sequentially(
    parent: adw::ApplicationWindow,
    docs: Vec<Rc<Document>>,
    index: usize,
    on_done: Rc<dyn Fn(Result<bool, String>)>,
) {
    if index >= docs.len() {
        on_done(Ok(true));
        return;
    }

    let doc = docs[index].clone();
    let parent_for_dialog = parent.clone();
    let parent_next = parent.clone();
    let docs_next = docs.clone();
    let on_done_next = on_done.clone();

    save_document(
        &parent_for_dialog,
        &doc,
        false,
        Rc::new(move |result| match result {
            Ok(true) => save_documents_sequentially(
                parent_next.clone(),
                docs_next.clone(),
                index + 1,
                on_done_next.clone(),
            ),
            Ok(false) => on_done_next(Ok(false)),
            Err(error) => on_done_next(Err(error)),
        }),
    );
}

/// Показывает диалог о несохраненных изменениях.
///
/// Три обработчика соответствуют ответам «Отмена», «Не сохранять» и «Сохранить».
fn show_unsaved_changes_dialog<F1, F2, F3>(
    parent: &adw::ApplicationWindow,
    heading: &str,
    body: &str,
    lang: Language,
    save_label: &str,
    on_cancel: F1,
    on_discard: F2,
    on_save: F3,
) where
    F1: FnOnce() + 'static,
    F2: FnOnce() + 'static,
    F3: FnOnce() + 'static,
{
    let dialog = adw::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .heading(heading)
        .body(body)
        .build();

    dialog.add_response("cancel", lang.btn_cancel());
    dialog.add_response("discard", lang.btn_discard());
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.add_response("save", save_label);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    let on_cancel = RefCell::new(Some(on_cancel));
    let on_discard = RefCell::new(Some(on_discard));
    let on_save = RefCell::new(Some(on_save));

    dialog.connect_response(None, move |_, response| match response {
        "discard" => {
            if let Some(callback) = on_discard.borrow_mut().take() {
                callback();
            }
        }
        "save" => {
            if let Some(callback) = on_save.borrow_mut().take() {
                callback();
            }
        }
        _ => {
            if let Some(callback) = on_cancel.borrow_mut().take() {
                callback();
            }
        }
    });

    dialog.present();
}

/// Окно настроек (`adw::PreferencesWindow`).
fn show_preferences_dialog(
    parent: &adw::ApplicationWindow,
    settings: Rc<RefCell<AppSettings>>,
    docs: Rc<RefCell<Vec<Rc<Document>>>>,
    editor_css_provider: gtk4::CssProvider,
    refresh_active_ui: Rc<dyn Fn()>,
    update_ui_language: Rc<dyn Fn(Language)>,
) {
    let current_lang = Language::from_code(&settings.borrow().language);

    let window = adw::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .title(current_lang.pref_window_title())
        .default_width(480)
        .build();

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

    let line_numbers_row = adw::SwitchRow::builder()
        .title(current_lang.pref_line_numbers_title())
        .subtitle(current_lang.pref_line_numbers_subtitle())
        .active(settings.borrow().show_line_numbers)
        .build();

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
    fonts_group.add(&line_numbers_row);

    page.add(&fonts_group);
    window.add(&page);
    window.present();
}

/// Стандартное окно «О программе».
fn show_about_dialog(parent: &adw::ApplicationWindow, lang: Language) {
    let about = adw::AboutWindow::builder()
        .transient_for(parent)
        .modal(true)
        .application_name("Aster")
        .application_icon("io.github.aster")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Developer")
        .comments(lang.about_comments())
        .issue_url("https://github.com")
        .license_type(gtk4::License::MitX11)
        .build();
    about.present();
}

#[cfg(test)]
mod tests {
    use super::*;

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
