//! Построение главного окна, вкладок и обработчиков действий.
//!
//! Начиная с этого модуля окно не хранит текст документа. Каждая вкладка —
//! это независимый [`crate::document::Document`] со своим буфером, WebKit-
//! предпросмотром и позициями прокрутки. Окно связывает подсистемы:
//! - заголовочную панель (`window/header.rs`);
//! - контекст окна, стартовые вкладки и закрытие (`window/workspace.rs`);
//! - меню, горячие клавиши и диалоги (`window/menus.rs`, `actions.rs`, `dialogs.rs`);
//! - сохранение сессии и геометрии (`window/session.rs`).

use adw::prelude::*;
use gtk4::gio;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crate::document::{self, Document, FileChange};
use crate::i18n::Language;
use crate::settings::AppSettings;

mod actions;
mod dialogs;
mod header;
mod menus;
mod preferences;
mod session;
mod style;
mod tabs;
mod workspace;

use actions::{install_actions, ActionContext};
use dialogs::{save_document, show_file_conflict_dialog, FileConflictChoice};
use header::{apply_header_language, build_header, sync_header_with_active_tab};
use style::{apply_color_scheme, apply_editor_style_css, disable_sound_feedback};
use tabs::{active_document, parent_directory};
use workspace::{
    connect_tab_and_close_handlers, restore_startup_tabs, schedule_recovery_save, WindowContext,
};

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
    let header = build_header(current_lang);

    // --- Вкладки и корневая разметка ---
    let tab_view = adw::TabView::new();
    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(&tab_view));
    tab_bar.set_autohide(true);
    tab_bar.set_end_action_widget(Some(&header.new_tab_btn));
    tab_view.set_menu_model(Some(&header.tab_menu));

    let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root_box.append(&header.header_bar);
    root_box.append(&tab_bar);
    root_box.append(&tab_view);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&root_box));
    window.set_content(Some(&toast_overlay));

    // --- Единый контекст окна ---
    let ctx = WindowContext {
        window: window.clone(),
        tab_view: tab_view.clone(),
        docs: Rc::new(RefCell::new(Vec::new())),
        settings: settings.clone(),
        toast_overlay,
        last_open_directory: Rc::new(RefCell::new(None)),
        closed_tabs: Rc::new(RefCell::new(Vec::new())),
        recovery_scheduled: Rc::new(Cell::new(false)),
        suppress_mode_sync: Rc::new(Cell::new(false)),
        suppress_search_sync: Rc::new(Cell::new(false)),
        is_force_closing: Rc::new(Cell::new(false)),
        is_closing: Rc::new(Cell::new(false)),
    };

    // --- Обновление заголовка и кнопок для активной вкладки ---
    let refresh_active_ui: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        let header = header.clone();
        Rc::new(move || {
            sync_header_with_active_tab(
                &ctx.window,
                &header,
                &ctx.tab_view,
                &ctx.docs,
                &ctx.settings,
                &ctx.suppress_mode_sync,
                &ctx.suppress_search_sync,
            );
        })
    };

    let persist_session: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        Rc::new(move || ctx.persist_session())
    };

    // --- Добавление документа во вкладку ---
    let add_document: Rc<dyn Fn(Option<PathBuf>, bool) -> Option<adw::TabPage>> = {
        let ctx = ctx.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        let search_btn = header.search_btn.clone();
        let window_weak = ctx.window.downgrade();
        let docs_weak = Rc::downgrade(&ctx.docs);
        let persist_weak = Rc::downgrade(&persist_session);

        Rc::new(move |path: Option<PathBuf>, restore_mode: bool| {
            // Если файл уже открыт, не создаем дубликат, а переключаемся на него.
            if let Some(ref path) = path {
                let key = document::get_file_canonical_key(path);
                let existing = ctx
                    .docs
                    .borrow()
                    .iter()
                    .find(|doc| doc.canonical_key().as_deref() == Some(key.as_str()))
                    .cloned();
                if let Some(doc) = existing {
                    if let Some(page) = doc.tab_page.borrow().clone() {
                        ctx.tab_view.set_selected_page(&page);
                        return Some(page);
                    }
                }
            }

            let path_for_directory = path.as_deref().and_then(parent_directory);
            let doc = match Document::new(ctx.settings.clone(), path, restore_mode) {
                Ok(doc) => doc,
                Err(error) => {
                    let lang = Language::from_code(&ctx.settings.borrow().language);
                    ctx.show_toast(&format!("{}: {error}", lang.error_open_file()));
                    return None;
                }
            };
            if let Some(directory) = path_for_directory {
                *ctx.last_open_directory.borrow_mut() = Some(directory);
            }
            let page = ctx.tab_view.append(&doc.root);
            *doc.tab_page.borrow_mut() = Some(page.clone());
            doc.update_tab_title();

            {
                let refresh_for_modified = Rc::downgrade(&refresh_active_ui);
                doc.set_modified_handler(Rc::new(move || {
                    if let Some(refresh) = refresh_for_modified.upgrade() {
                        refresh();
                    }
                }));
            }

            // Следим за изменениями файла на диске. Слабые ссылки разрывают
            // цикл владения Document -> handler -> WindowContext -> docs.
            let weak_doc_for_file = Rc::downgrade(&doc);
            let settings_for_file = ctx.settings.clone();
            let refresh_for_file = Rc::downgrade(&refresh_active_ui);
            let persist_for_file = persist_weak.clone();
            let toast_overlay_for_file = ctx.toast_overlay.clone();
            let window_for_conflict = window_weak.clone();
            let conflict_dialog_visible = Rc::new(Cell::new(false));
            doc.watch_file(Rc::new(move |change| {
                let Some(doc) = weak_doc_for_file.upgrade() else {
                    return;
                };
                let lang = Language::from_code(&settings_for_file.borrow().language);

                match change {
                    FileChange::Reloaded => {
                        if let Some(refresh) = refresh_for_file.upgrade() {
                            refresh();
                        }
                        if let Some(persist) = persist_for_file.upgrade() {
                            persist();
                        }
                    }
                    FileChange::Conflict => {
                        if conflict_dialog_visible.get() {
                            return;
                        }
                        let Some(window) = window_for_conflict.upgrade() else {
                            return;
                        };
                        conflict_dialog_visible.set(true);

                        let file_name = doc
                            .current_file
                            .borrow()
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .and_then(|name| name.to_str())
                            .unwrap_or("document.md")
                            .to_string();

                        let flag = conflict_dialog_visible.clone();
                        let doc_for_reload = doc.clone();
                        let refresh_for_reload = refresh_for_file.clone();
                        let persist_for_reload = persist_for_file.clone();
                        let toast_overlay = toast_overlay_for_file.clone();
                        let settings_for_reload = settings_for_file.clone();

                        show_file_conflict_dialog(&window, lang, &file_name, move |choice| {
                            flag.set(false);
                            if choice == FileConflictChoice::KeepEditing {
                                return;
                            }
                            match doc_for_reload.reload_from_disk() {
                                Ok(()) => {
                                    if let Some(refresh) = refresh_for_reload.upgrade() {
                                        refresh();
                                    }
                                    if let Some(persist) = persist_for_reload.upgrade() {
                                        persist();
                                    }
                                }
                                Err(error) => {
                                    let lang = Language::from_code(
                                        &settings_for_reload.borrow().language,
                                    );
                                    let toast = adw::Toast::new(&format!(
                                        "{}: {error}",
                                        lang.error_reload_file()
                                    ));
                                    toast.set_timeout(6);
                                    toast_overlay.add_toast(toast);
                                }
                            }
                        });
                    }
                }
            }));

            // Обновление заголовка и отложенное сохранение безымянного черновика.
            let weak_doc = Rc::downgrade(&doc);
            let tab_view_for_change = ctx.tab_view.clone();
            let refresh_for_change = refresh_active_ui.clone();
            let docs_for_recovery = docs_weak.clone();
            let recovery_scheduled = ctx.recovery_scheduled.clone();
            doc.buffer.connect_changed(move |_| {
                if let Some(doc) = weak_doc.upgrade() {
                    let is_active = tab_view_for_change
                        .selected_page()
                        .map(|page| doc.matches_page(&page))
                        .unwrap_or(false);
                    if is_active {
                        refresh_for_change();
                    }

                    if doc.current_file.borrow().is_none() {
                        schedule_recovery_save(
                            docs_for_recovery.clone(),
                            recovery_scheduled.clone(),
                        );
                    }
                }
            });

            // Закрытие панели поиска клавишей Esc сбрасывает кнопку в заголовке.
            let weak_doc = Rc::downgrade(&doc);
            let tab_view_for_search = ctx.tab_view.clone();
            let search_btn_for_sync = search_btn.clone();
            let suppress_for_search = ctx.suppress_search_sync.clone();
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

            ctx.docs.borrow_mut().push(doc.clone());
            ctx.tab_view.set_selected_page(&page);
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
    let update_ui_language: Rc<dyn Fn(Language)> = {
        let header = header.clone();
        let docs = ctx.docs.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        Rc::new(move |lang: Language| {
            apply_header_language(&header, &docs, lang);
            refresh_active_ui();
        })
    };

    // --- Диалог открытия файла ---
    let show_open_dialog: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        let add_document = add_document.clone();
        let refresh_active_ui = refresh_active_ui.clone();

        Rc::new(move || {
            let lang = Language::from_code(&ctx.settings.borrow().language);
            let dialog = gtk4::FileDialog::builder()
                .title(lang.dialog_open_title())
                .build();

            let initial_directory = active_document(&ctx.tab_view, &ctx.docs)
                .and_then(|doc| {
                    doc.current_file
                        .borrow()
                        .as_ref()
                        .and_then(|path| parent_directory(path))
                })
                .or_else(|| ctx.last_open_directory.borrow().clone());
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
            let ctx_done = ctx.clone();
            let refresh_active_ui = refresh_active_ui.clone();
            dialog.open_multiple(
                Some(&ctx.window),
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

                    ctx_done.persist_session();
                    refresh_active_ui();
                },
            );
        })
    };

    // --- Сохранение активного документа ---
    let save_active_document: Rc<dyn Fn(bool)> = {
        let ctx = ctx.clone();
        let refresh_active_ui = refresh_active_ui.clone();

        Rc::new(move |save_as: bool| {
            let Some(doc) = active_document(&ctx.tab_view, &ctx.docs) else {
                return;
            };
            let ctx_done = ctx.clone();
            let refresh_active_ui = refresh_active_ui.clone();
            let doc_for_directory = doc.clone();
            save_document(
                &ctx.window,
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
                            *ctx_done.last_open_directory.borrow_mut() = Some(directory);
                        }
                        ctx_done.save_recovery_now();
                        ctx_done.persist_session();
                        refresh_active_ui();
                    }
                    Ok(false) => {}
                    Err(error) => {
                        let lang = Language::from_code(&ctx_done.settings.borrow().language);
                        ctx_done.show_toast(&format!("{}: {error}", lang.error_save_file()));
                    }
                }),
            );
        })
    };

    let actions = install_actions(ActionContext {
        window: ctx.window.clone(),
        header_bar: header.header_bar.clone(),
        tab_view: ctx.tab_view.clone(),
        docs: ctx.docs.clone(),
        settings: ctx.settings.clone(),
        editor_css_provider,
        add_document: add_document.clone(),
        persist_session: persist_session.clone(),
        refresh_active_ui: refresh_active_ui.clone(),
        show_open_dialog: show_open_dialog.clone(),
        save_active_document: save_active_document.clone(),
        update_ui_language,
        closed_tabs: ctx.closed_tabs.clone(),
    });
    window.insert_action_group("win", Some(&actions));

    // --- Обработчики кнопок заголовка ---
    {
        let show_open_dialog = show_open_dialog.clone();
        header.open_btn.connect_clicked(move |_| show_open_dialog());
    }
    {
        let save_active_document = save_active_document.clone();
        header
            .save_btn
            .connect_clicked(move |_| save_active_document(false));
    }
    {
        let ctx = ctx.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        header.edit_btn.connect_toggled(move |btn| {
            if !btn.is_active() || ctx.suppress_mode_sync.get() {
                return;
            }
            if let Some(doc) = active_document(&ctx.tab_view, &ctx.docs) {
                doc.switch_to_editor();
                ctx.persist_session();
                refresh_active_ui();
            }
        });
    }
    {
        let ctx = ctx.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        header.preview_btn.connect_toggled(move |btn| {
            if !btn.is_active() || ctx.suppress_mode_sync.get() {
                return;
            }
            if let Some(doc) = active_document(&ctx.tab_view, &ctx.docs) {
                doc.switch_to_preview();
                ctx.persist_session();
                refresh_active_ui();
            }
        });
    }
    {
        let ctx = ctx.clone();
        header.search_btn.connect_toggled(move |btn| {
            if ctx.suppress_search_sync.get() {
                return;
            }
            if let Some(doc) = active_document(&ctx.tab_view, &ctx.docs) {
                doc.search_bar.set_search_mode(btn.is_active());
                if btn.is_active() {
                    doc.search_entry.grab_focus();
                }
            }
        });
    }
    {
        let ctx = ctx.clone();
        let add_document = add_document.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        header.new_tab_btn.connect_clicked(move |_| {
            add_document(None, false);
            ctx.persist_session();
            refresh_active_ui();
        });
    }

    // --- Восстановление стартовых вкладок и сигналы вкладок/закрытия ---
    restore_startup_tabs(&ctx, &add_document, initial_files, restore_session);
    connect_tab_and_close_handlers(&ctx, &refresh_active_ui);

    // Первоначальная синхронизация заголовка и сохранение сессии.
    refresh_active_ui();
    ctx.persist_session();

    // --- Горячие клавиши приложения ---
    app.set_accels_for_action("win.save", &["<Control>s"]);
    app.set_accels_for_action("win.save_as", &["<Control><Shift>s"]);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    app.set_accels_for_action("win.new_tab", &["<Control>t"]);
    app.set_accels_for_action("win.reopen_tab", &["<Control><Shift>t"]);
    app.set_accels_for_action("win.toggle_meditation", &["<Control><Shift>h"]);
    app.set_accels_for_action("win.toggle_line_numbers", &["<Control><Shift>l"]);
    app.set_accels_for_action("win.toggle_soft_wrap", &["<Control><Shift>w"]);
    app.set_accels_for_action("win.toggle_whitespace", &["<Control><Shift>p"]);
    app.set_accels_for_action("win.close_tab", &["<Control>w"]);
    app.set_accels_for_action("win.next_tab", &["<Control>Tab"]);
    app.set_accels_for_action(
        "win.previous_tab",
        &["<Control><Shift>Tab", "<Control>ISO_Left_Tab"],
    );
    app.set_accels_for_action("win.toggle_preview", &["<Control>e"]);
    app.set_accels_for_action("win.find", &["<Control>f"]);
    app.set_accels_for_action("win.find_replace", &["<Control>r"]);
    app.set_accels_for_action("win.format_bold", &["<Control>b"]);
    app.set_accels_for_action("win.format_italic", &["<Control>i"]);
    app.set_accels_for_action("win.format_code", &["<Control>grave"]);
    app.set_accels_for_action("win.cycle_case", &["<Control><Shift>u"]);
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
        assert_eq!(wrapped_tab_position(0, 3, true), Some(1));
        assert_eq!(wrapped_tab_position(1, 3, true), Some(2));
        assert_eq!(wrapped_tab_position(2, 3, true), Some(0));

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

        tab_view.set_selected_page(&last);
        select_tab_wrapped(&tab_view, true);
        assert_eq!(
            tab_view.page_position(&tab_view.selected_page().expect("selected tab")),
            0
        );

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
