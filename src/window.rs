use adw::prelude::*;
use gtk4::gio;
use gtk4::glib;
use gtk4::pango;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use webkit6::prelude::*;

use crate::highlighter;
use crate::renderer;
use crate::settings::AppSettings;

/// Состояние приложения, разделяемое между различными обработчиками сигналов GTK.
#[derive(Clone)]
pub struct AppState {
    /// Путь к текущему открытому файлу (None, если файл новый / безымянный).
    pub current_file: Rc<RefCell<Option<PathBuf>>>,
    /// Флаг наличия несохраненных изменений в тексте.
    pub is_modified: Rc<RefCell<bool>>,
    /// Текущие пользовательские настройки (тема, шрифты, отступы, состояние окна).
    pub settings: Rc<RefCell<AppSettings>>,
    /// Активен ли сейчас режим предпросмотра (true) или редактор (false).
    pub is_preview_active: Rc<RefCell<bool>>,
    /// Запоминаем долю прокрутки (0.0 .. 1.0) для синхронизации позиции между режимами
    pub scroll_ratio: Rc<RefCell<f64>>,
}

/// Создает и конфигурирует главное окно приложения.
pub fn build_window(app: &adw::Application, initial_file: Option<PathBuf>) -> adw::ApplicationWindow {
    let settings = AppSettings::load();
    let app_state = AppState {
        current_file: Rc::new(RefCell::new(initial_file.clone())),
        is_modified: Rc::new(RefCell::new(false)),
        settings: Rc::new(RefCell::new(settings.clone())),
        is_preview_active: Rc::new(RefCell::new(false)),
        scroll_ratio: Rc::new(RefCell::new(0.0)),
    };

    // Применяем цветовую схему из настроек
    apply_color_scheme(&app_state.settings.borrow().theme);

    // Создаем главное окно в стиле Libadwaita с сохраненными размерами
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("rmd-view")
        .default_width(settings.window_width)
        .default_height(settings.window_height)
        .build();

    // Восстанавливаем состояние "на весь экран", если оно было сохранено
    if settings.window_maximized {
        window.maximize();
    }

    let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // --- 1. Заголовочная панель (HeaderBar) ---
    let header_bar = adw::HeaderBar::new();

    let open_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    open_btn.set_tooltip_text(Some("Открыть файл (Ctrl+O)"));
    header_bar.pack_start(&open_btn);

    let save_btn = gtk4::Button::from_icon_name("document-save-symbolic");
    save_btn.set_tooltip_text(Some("Сохранить (Ctrl+S)"));
    header_bar.pack_start(&save_btn);

    // Переключатель вида (Редактор <-> Просмотр) в центре HeaderBar
    let view_stack = adw::ViewStack::new();

    let edit_btn = gtk4::ToggleButton::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text("Редактор (Ctrl+Tab для переключения)")
        .active(true)
        .build();

    let preview_btn = gtk4::ToggleButton::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text("Просмотр Markdown (Ctrl+Tab для переключения)")
        .group(&edit_btn)
        .build();

    let switch_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    switch_box.add_css_class("linked");
    switch_box.append(&edit_btn);
    switch_box.append(&preview_btn);
    header_bar.set_title_widget(Some(&switch_box));

    let search_btn = gtk4::ToggleButton::builder()
        .icon_name("edit-find-symbolic")
        .tooltip_text("Поиск подстроки (Ctrl+F)")
        .build();
    header_bar.pack_end(&search_btn);

    let menu_btn = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Главное меню")
        .build();

    let menu = gio::Menu::new();
    menu.append(Some("Сохранить как..."), Some("win.save_as"));
    menu.append(Some("Настройки"), Some("win.preferences"));
    menu.append(Some("О программе"), Some("win.about"));
    menu.append(Some("Закрыть окно"), Some("win.close"));
    menu_btn.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_btn);

    root_box.append(&header_bar);

    // --- 2. Панель поиска ---
    let search_bar = gtk4::SearchBar::new();
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_hexpand(true);
    let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    search_box.append(&search_entry);
    search_bar.set_child(Some(&search_box));
    search_bar.connect_entry(&search_entry);
    search_bar.bind_property("search-mode-enabled", &search_btn, "active")
        .bidirectional()
        .sync_create()
        .build();
    root_box.append(&search_bar);

    // --- 3. Рабочая область: Редактор текста ---
    let tag_table = gtk4::TextTagTable::new();
    let editor_font_tag = gtk4::TextTag::builder()
        .name("editor-font")
        .build();
    tag_table.add(&editor_font_tag);

    // Регистрируем теги подсветки синтаксиса Markdown
    let style_mgr = adw::StyleManager::default();
    highlighter::register_highlight_tags(
        &tag_table,
        &app_state.settings.borrow().theme,
        style_mgr.is_dark(),
    );

    let buffer = gtk4::TextBuffer::builder()
        .tag_table(&tag_table)
        .build();

    let text_view = gtk4::TextView::builder()
        .buffer(&buffer)
        .wrap_mode(gtk4::WrapMode::Word)
        .left_margin(18)
        .right_margin(18)
        .top_margin(14)
        .bottom_margin(14)
        .build();

    text_view.add_css_class("editor-view");

    // CssProvider для стилизации темы редактора (фон и цвет курсора/текста)
    let editor_theme_css_provider = gtk4::CssProvider::new();
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &editor_theme_css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );
    }
    apply_editor_theme_css(&editor_theme_css_provider, &app_state.settings.borrow().theme, style_mgr.is_dark());

    let editor_scroll = gtk4::ScrolledWindow::builder()
        .child(&text_view)
        .hexpand(true)
        .vexpand(true)
        .build();

    view_stack.add_titled(&editor_scroll, Some("editor"), "Редактор");

    // --- 4. Рабочая область: Предпросмотр WebKitGTK ---
    let web_view = webkit6::WebView::new();

    if let Some(web_settings) = webkit6::prelude::WebViewExt::settings(&web_view) {
        web_settings.set_allow_file_access_from_file_urls(true);
        web_settings.set_allow_universal_access_from_file_urls(true);
    }

    let preview_scroll = gtk4::ScrolledWindow::builder()
        .child(&web_view)
        .hexpand(true)
        .vexpand(true)
        .build();

    view_stack.add_titled(&preview_scroll, Some("preview"), "Просмотр");
    root_box.append(&view_stack);
    window.set_content(Some(&root_box));

    // Применяем семейство и размер шрифта редактора
    apply_editor_font(
        &buffer,
        &editor_font_tag,
        &app_state.settings.borrow().editor_font_family,
        app_state.settings.borrow().editor_font_size,
    );

    // Если был передан файл при запуске — загружаем его содержимое
    if let Some(ref path) = initial_file {
        if let Ok(content) = std::fs::read_to_string(path) {
            buffer.set_text(&content);
            apply_editor_font(
                &buffer,
                &editor_font_tag,
                &app_state.settings.borrow().editor_font_family,
                app_state.settings.borrow().editor_font_size,
            );
            highlighter::highlight_markdown_buffer(&buffer);
            *app_state.is_modified.borrow_mut() = false;
        }
    }
    update_window_title(&window, &app_state);

    // Отслеживаем изменения в буфере текста
    let state_clone = app_state.clone();
    let window_weak = window.downgrade();
    let font_tag_for_buffer = editor_font_tag.clone();
    buffer.connect_changed(move |buf| {
        *state_clone.is_modified.borrow_mut() = true;
        let start = buf.start_iter();
        let end = buf.end_iter();
        buf.apply_tag(&font_tag_for_buffer, &start, &end);
        highlighter::highlight_markdown_buffer(buf);
        if let Some(win) = window_weak.upgrade() {
            update_window_title(&win, &state_clone);
        }
    });

    // Замыкание для перерендеринга Markdown в HTML и загрузки в WebView
    let refresh_preview = {
        let buffer = buffer.clone();
        let web_view = web_view.clone();
        let app_state = app_state.clone();
        Rc::new(move || {
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            let text = buffer.text(&start, &end, false);
            let style_mgr = adw::StyleManager::default();
            let is_dark = style_mgr.is_dark();
            let theme = app_state.settings.borrow().theme.clone();
            let font_size = app_state.settings.borrow().preview_font_size;
            let padding_percent = app_state.settings.borrow().preview_padding_percent;

            let base_uri = app_state.current_file.borrow().as_ref().and_then(|path| {
                path.parent().and_then(|parent| {
                    let canonical = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
                    let mut s = canonical.to_string_lossy().to_string();
                    if !s.ends_with('/') {
                        s.push('/');
                    }
                    Some(format!("file://{s}"))
                })
            });

            let html = renderer::render_markdown_to_html(
                text.as_str(),
                &theme,
                is_dark,
                font_size,
                padding_percent,
                base_uri.as_deref(),
            );

            web_view.load_html(&html, base_uri.as_deref());
        })
    };

    // При завершении загрузки предпросмотра восстанавливаем позицию прокрутки
    let scroll_ratio_for_load = app_state.scroll_ratio.clone();
    web_view.connect_load_changed(move |view, event| {
        if event == webkit6::LoadEvent::Finished {
            let ratio = *scroll_ratio_for_load.borrow();
            let script = format!(
                "window.scrollTo(0, (document.documentElement.scrollHeight - window.innerHeight) * {});",
                ratio
            );
            view.evaluate_javascript(&script, None::<&str>, None::<&str>, gio::Cancellable::NONE, |_| {});
        }
    });

    // Логика переключения режимов (Редактор <-> Предпросмотр) с сохранением позиции
    let refresh_for_switch = refresh_preview.clone();
    let view_stack_clone = view_stack.clone();
    let edit_btn_clone = edit_btn.clone();
    let preview_btn_clone = preview_btn.clone();
    let text_view_for_toggle = text_view.clone();
    let state_toggle = app_state.clone();
    let editor_scroll_clone = editor_scroll.clone();
    let web_view_clone = web_view.clone();

    let toggle_view = Rc::new(move |to_preview: bool| {
        *state_toggle.is_preview_active.borrow_mut() = to_preview;
        if to_preview {
            // Запоминаем текущую относительную позицию редактора перед переходом в просмотр
            let vadj = editor_scroll_clone.vadjustment();
            let upper = vadj.upper() - vadj.page_size();
            let ratio = if upper > 0.0 {
                (vadj.value() / upper).clamp(0.0, 1.0)
            } else {
                0.0
            };
            *state_toggle.scroll_ratio.borrow_mut() = ratio;

            refresh_for_switch();
            view_stack_clone.set_visible_child_name("preview");
            preview_btn_clone.set_active(true);
        } else {
            // При переходе из предпросмотра в редактор считываем scroll-ratio из браузерного JS
            let scroll_ratio_holder = state_toggle.scroll_ratio.clone();
            let vadj = editor_scroll_clone.vadjustment();
            let tv = text_view_for_toggle.clone();

            web_view_clone.evaluate_javascript(
                "var total = document.documentElement.scrollHeight - window.innerHeight; total > 0 ? (window.scrollY / total) : 0;",
                None::<&str>,
                None::<&str>,
                gio::Cancellable::NONE,
                move |result| {
                    if let Ok(js_val) = result {
                        let ratio = js_val.to_double().clamp(0.0, 1.0);
                        *scroll_ratio_holder.borrow_mut() = ratio;
                        let upper = vadj.upper() - vadj.page_size();
                        if upper > 0.0 {
                            vadj.set_value(ratio * upper);
                        }
                    }
                    tv.grab_focus();
                },
            );

            view_stack_clone.set_visible_child_name("editor");
            edit_btn_clone.set_active(true);
        }
    });

    let toggle_view_1 = toggle_view.clone();
    preview_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            toggle_view_1(true);
        }
    });

    let toggle_view_2 = toggle_view.clone();
    edit_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            toggle_view_2(false);
        }
    });

    // --- Обработчик сохранения файла (Save / Save As) ---
    let save_action_state = app_state.clone();
    let save_action_window = window.clone();
    let save_action_buffer = buffer.clone();
    let do_save = Rc::new(move |force_save_as: bool| {
        let current_path = save_action_state.current_file.borrow().clone();
        if let (false, Some(path)) = (force_save_as, current_path) {
            let start = save_action_buffer.start_iter();
            let end = save_action_buffer.end_iter();
            let text = save_action_buffer.text(&start, &end, false);
            if std::fs::write(&path, text.as_bytes()).is_ok() {
                *save_action_state.is_modified.borrow_mut() = false;
                update_window_title(&save_action_window, &save_action_state);
            }
        } else {
            let file_dialog = gtk4::FileDialog::builder()
                .title("Сохранить Markdown файл")
                .initial_name("document.md")
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Markdown (*.md)"));
            filter.add_pattern("*.md");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            file_dialog.set_filters(Some(&filters));

            let buffer = save_action_buffer.clone();
            let state = save_action_state.clone();
            let win = save_action_window.clone();

            file_dialog.save(Some(&save_action_window), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let start = buffer.start_iter();
                        let end = buffer.end_iter();
                        let text = buffer.text(&start, &end, false);
                        if std::fs::write(&path, text.as_bytes()).is_ok() {
                            *state.current_file.borrow_mut() = Some(path);
                            *state.is_modified.borrow_mut() = false;
                            update_window_title(&win, &state);
                        }
                    }
                }
            });
        }
    });

    let do_save_btn = do_save.clone();
    save_btn.connect_clicked(move |_| {
        do_save_btn(false);
    });

    // --- Обработчик открытия файла ---
    let window_for_open = window.clone();
    let buffer_for_open = buffer.clone();
    let state_for_open = app_state.clone();
    let do_save_for_open = do_save.clone();
    let font_tag_for_open = editor_font_tag.clone();
    let trigger_file_open = Rc::new(move || {
        let file_dialog = gtk4::FileDialog::builder()
            .title("Открыть Markdown файл")
            .build();

        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Markdown (*.md, *.markdown, *.rmd)"));
        filter.add_pattern("*.md");
        filter.add_pattern("*.markdown");
        filter.add_pattern("*.rmd");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let buffer = buffer_for_open.clone();
        let state = state_for_open.clone();
        let win = window_for_open.clone();
        let tag = font_tag_for_open.clone();

        file_dialog.open(Some(&window_for_open), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        buffer.set_text(&content);
                        let family = state.settings.borrow().editor_font_family.clone();
                        let size = state.settings.borrow().editor_font_size;
                        apply_editor_font(&buffer, &tag, &family, size);
                        highlighter::highlight_markdown_buffer(&buffer);
                        *state.current_file.borrow_mut() = Some(path);
                        *state.is_modified.borrow_mut() = false;
                        update_window_title(&win, &state);
                    }
                }
            }
        });
    });

    let window_for_open_check = window.clone();
    let state_for_open_check = app_state.clone();
    let do_open_dialog = Rc::new(move || {
        let is_mod = *state_for_open_check.is_modified.borrow();
        if is_mod {
            let trigger_open_discard = trigger_file_open.clone();
            let trigger_open_save = trigger_file_open.clone();
            let do_save = do_save_for_open.clone();
            show_unsaved_changes_dialog(
                &window_for_open_check,
                "Открыть другой файл?",
                "В текущем файле есть несохраненные изменения. Открытие нового файла приведет к их потере.",
                move || {
                    trigger_open_discard();
                },
                move || {
                    do_save(false);
                    trigger_open_save();
                },
            );
        } else {
            trigger_file_open();
        }
    });

    let do_open_btn = do_open_dialog.clone();
    open_btn.connect_clicked(move |_| {
        do_open_btn();
    });

    // --- Обработчик закрытия окна с сохранением геометрии (размеров и maximized) ---
    let state_for_close = app_state.clone();
    let do_save_for_close = do_save.clone();
    let is_force_closing = Rc::new(RefCell::new(false));

    let save_window_geometry = {
        let state = app_state.clone();
        Rc::new(move |win: &adw::ApplicationWindow| {
            let is_max = win.is_maximized();
            state.settings.borrow_mut().window_maximized = is_max;
            if !is_max {
                let width = win.default_width();
                let height = win.default_height();
                state.settings.borrow_mut().window_width = width;
                state.settings.borrow_mut().window_height = height;
            }
            state.settings.borrow().save();
        })
    };

    let force_closing_clone = is_force_closing.clone();
    let geometry_saver = save_window_geometry.clone();
    window.connect_close_request(move |win| {
        if *force_closing_clone.borrow() {
            geometry_saver(win);
            return glib::Propagation::Proceed;
        }

        let is_mod = *state_for_close.is_modified.borrow();
        if is_mod {
            let win_for_discard = win.clone();
            let win_for_save = win.clone();
            let flag_discard = force_closing_clone.clone();
            let flag_save = force_closing_clone.clone();
            let do_save = do_save_for_close.clone();
            let geom_discard = geometry_saver.clone();
            let geom_save = geometry_saver.clone();

            show_unsaved_changes_dialog(
                win,
                "Закрыть без сохранения?",
                "В документе есть несохраненные изменения. Закрытие приведет к их потере.",
                move || {
                    geom_discard(&win_for_discard);
                    *flag_discard.borrow_mut() = true;
                    win_for_discard.close();
                },
                move || {
                    do_save(false);
                    geom_save(&win_for_save);
                    *flag_save.borrow_mut() = true;
                    win_for_save.close();
                },
            );
            glib::Propagation::Stop
        } else {
            geometry_saver(win);
            glib::Propagation::Proceed
        }
    });

    // --- Регистрация действий окна (GAction) для меню и шорткатов ---
    let actions = gio::SimpleActionGroup::new();

    let do_save_as = do_save.clone();
    let save_as_action = gio::SimpleAction::new("save_as", None);
    save_as_action.connect_activate(move |_, _| {
        do_save_as(true);
    });
    actions.add_action(&save_as_action);

    let do_save_shortcut = do_save.clone();
    let save_action = gio::SimpleAction::new("save", None);
    save_action.connect_activate(move |_, _| {
        do_save_shortcut(false);
    });
    actions.add_action(&save_action);

    let do_open_action = do_open_dialog.clone();
    let open_action = gio::SimpleAction::new("open", None);
    open_action.connect_activate(move |_, _| {
        do_open_action();
    });
    actions.add_action(&open_action);

    let win_for_close_action = window.clone();
    let close_action = gio::SimpleAction::new("close", None);
    close_action.connect_activate(move |_, _| {
        win_for_close_action.close();
    });
    actions.add_action(&close_action);

    let toggle_state = app_state.clone();
    let toggle_view_shortcut = toggle_view.clone();
    let preview_action = gio::SimpleAction::new("toggle_preview", None);
    preview_action.connect_activate(move |_, _| {
        let is_prev = *toggle_state.is_preview_active.borrow();
        toggle_view_shortcut(!is_prev);
    });
    actions.add_action(&preview_action);

    let search_bar_clone = search_bar.clone();
    let search_entry_clone = search_entry.clone();
    let find_action = gio::SimpleAction::new("find", None);
    find_action.connect_activate(move |_, _| {
        let current = search_bar_clone.is_search_mode();
        search_bar_clone.set_search_mode(!current);
        if !current {
            search_entry_clone.grab_focus();
        }
    });
    actions.add_action(&find_action);

    let window_for_pref = window.clone();
    let buffer_for_pref = buffer.clone();
    let tag_for_pref = editor_font_tag.clone();
    let tag_table_for_pref = tag_table.clone();
    let css_provider_for_pref = editor_theme_css_provider.clone();
    let state_for_pref = app_state.clone();
    let refresh_pref = refresh_preview.clone();
    let pref_action = gio::SimpleAction::new("preferences", None);
    pref_action.connect_activate(move |_, _| {
        show_preferences_dialog(
            &window_for_pref,
            &state_for_pref,
            &buffer_for_pref,
            &tag_for_pref,
            &tag_table_for_pref,
            &css_provider_for_pref,
            refresh_pref.clone(),
        );
    });
    actions.add_action(&pref_action);

    let window_for_about = window.clone();
    let about_action = gio::SimpleAction::new("about", None);
    about_action.connect_activate(move |_, _| {
        show_about_dialog(&window_for_about);
    });
    actions.add_action(&about_action);

    window.insert_action_group("win", Some(&actions));

    setup_search(&search_entry, &text_view);

    app.set_accels_for_action("win.save", &["<Control>s"]);
    app.set_accels_for_action("win.save_as", &["<Control><Shift>s"]);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    app.set_accels_for_action("win.toggle_preview", &["<Control>Tab", "<Control>ISO_Left_Tab"]);
    app.set_accels_for_action("win.find", &["<Control>f"]);
    app.set_accels_for_action("win.close", &["<Control>q", "<Control>F4"]);

    window
}

/// Применяет CSS-стилизацию к текстовому редактору для поддержки выбранной цветовой темы
fn apply_editor_theme_css(provider: &gtk4::CssProvider, theme: &str, is_dark: bool) {
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

    let css = format!(
        ".editor-view, textview.editor-view, textview.editor-view text {{
            background-color: {bg} !important;
            color: {fg} !important;
            caret-color: {cursor_color} !important;
        }}"
    );
    provider.load_from_data(&css);
}

/// Отображает модальный диалог подтверждения при наличии несохраненных изменений
fn show_unsaved_changes_dialog<F1, F2>(
    parent: &adw::ApplicationWindow,
    heading: &str,
    body: &str,
    on_discard: F1,
    on_save: F2,
) where
    F1: FnOnce() + 'static,
    F2: FnOnce() + 'static,
{
    let dialog = adw::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .heading(heading)
        .body(body)
        .build();

    dialog.add_response("cancel", "Отмена");
    dialog.add_response("discard", "Не сохранять");
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.add_response("save", "Сохранить");
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    let on_discard = RefCell::new(Some(on_discard));
    let on_save = RefCell::new(Some(on_save));

    dialog.connect_response(None, move |_, response| {
        match response {
            "discard" => {
                if let Some(cb) = on_discard.borrow_mut().take() {
                    cb();
                }
            }
            "save" => {
                if let Some(cb) = on_save.borrow_mut().take() {
                    cb();
                }
            }
            _ => {}
        }
    });

    dialog.present();
}

/// Обновляет заголовок окна: отображает имя текущего файла и звездочку `*` при наличии правок.
fn update_window_title(window: &adw::ApplicationWindow, state: &AppState) {
    let filename = if let Some(ref path) = *state.current_file.borrow() {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.md")
            .to_string()
    } else {
        "Безымянный".to_string()
    };

    let modified_marker = if *state.is_modified.borrow() { "*" } else { "" };
    window.set_title(Some(&format!("{modified_marker}{filename} — rmd-view")));
}

/// Применяет выбранную цветовую схему к Adwaita StyleManager
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

/// Применяет гарнитуру и размер шрифта к текстовому буферу редактора
fn apply_editor_font(buffer: &gtk4::TextBuffer, tag: &gtk4::TextTag, font_family: &str, font_size: u32) {
    let mut desc = pango::FontDescription::new();
    desc.set_family(font_family);
    desc.set_size((font_size as i32) * pango::SCALE);

    tag.set_font_desc(Some(&desc));

    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.apply_tag(tag, &start, &end);
}

/// Настраивает механизм поиска по текстовому буферу `GtkTextBuffer`
fn setup_search(entry: &gtk4::SearchEntry, text_view: &gtk4::TextView) {
    let buffer = text_view.buffer();
    let text_view = text_view.clone();

    let tag_table = buffer.tag_table();
    let highlight_tag = gtk4::TextTag::builder()
        .name("search-match")
        .background("#f5a97f")
        .foreground("#181926")
        .build();
    tag_table.add(&highlight_tag);

    entry.connect_search_changed(move |search_entry| {
        let query = search_entry.text().to_string();
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.remove_tag_by_name("search-match", &start, &end);

        if query.is_empty() {
            return;
        }

        let mut iter = buffer.start_iter();
        let mut first_match = None;

        while let Some((match_start, match_end)) = iter.forward_search(&query, gtk4::TextSearchFlags::CASE_INSENSITIVE, None) {
            buffer.apply_tag_by_name("search-match", &match_start, &match_end);
            if first_match.is_none() {
                first_match = Some(match_start);
            }
            iter = match_end;
        }

        if let Some(pos) = first_match {
            buffer.place_cursor(&pos);
            let mark = buffer.create_mark(None, &pos, false);
            text_view.scroll_to_mark(&mark, 0.1, true, 0.0, 0.5);
            buffer.delete_mark(&mark);
        }
    });
}

/// Отображает модальное окно настроек (`adw::PreferencesWindow`)
fn show_preferences_dialog(
    parent: &adw::ApplicationWindow,
    state: &AppState,
    editor_buffer: &gtk4::TextBuffer,
    editor_font_tag: &gtk4::TextTag,
    tag_table: &gtk4::TextTagTable,
    editor_theme_provider: &gtk4::CssProvider,
    refresh_preview: Rc<dyn Fn()>,
) {
    let window = adw::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .title("Настройки")
        .default_width(480)
        .build();

    let page = adw::PreferencesPage::new();

    // Секция 1: Цветовая схема
    let appearance_group = adw::PreferencesGroup::builder()
        .title("Оформление")
        .description("Выбор цветовой темы приложения, редактора и предпросмотра")
        .build();

    let theme_names = [
        ("Системная", "system"),
        ("Светлая (GitHub Light)", "light"),
        ("Тёмная (GitHub Dark)", "dark"),
        ("Dracula", "dracula"),
        ("Kimbie Dark", "kimbie_dark"),
        ("Solarized Dark", "solarized_dark"),
        ("Solarized Light", "solarized_light"),
    ];

    let theme_labels: Vec<&str> = theme_names.iter().map(|(label, _)| *label).collect();

    let theme_row = adw::ComboRow::builder()
        .title("Цветовая схема")
        .model(&gtk4::StringList::new(&theme_labels))
        .build();

    let current_theme = state.settings.borrow().theme.clone();
    let initial_theme_idx = theme_names
        .iter()
        .position(|(_, code)| *code == current_theme)
        .unwrap_or(0);
    theme_row.set_selected(initial_theme_idx as u32);

    let state_theme = state.clone();
    let refresh_for_theme = refresh_preview.clone();
    let tag_table_theme = tag_table.clone();
    let theme_provider_clone = editor_theme_provider.clone();
    let editor_buf_theme = editor_buffer.clone();

    theme_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if let Some(&(_, selected_code)) = theme_names.get(idx) {
            state_theme.settings.borrow_mut().theme = selected_code.to_string();
            state_theme.settings.borrow().save();
            apply_color_scheme(selected_code);

            let style_mgr = adw::StyleManager::default();
            apply_editor_theme_css(&theme_provider_clone, selected_code, style_mgr.is_dark());
            highlighter::update_highlight_colors(&tag_table_theme, selected_code, style_mgr.is_dark());
            highlighter::highlight_markdown_buffer(&editor_buf_theme);

            refresh_for_theme();
        }
    });

    appearance_group.add(&theme_row);
    page.add(&appearance_group);

    // Секция 2: Разметка и отступы предпросмотра
    let layout_group = adw::PreferencesGroup::builder()
        .title("Разметка предпросмотра")
        .description("Настройка боковых полей документа")
        .build();

    let padding_spin = adw::SpinRow::builder()
        .title("Боковые поля (% от ширины)")
        .subtitle("По умолчанию 5% с каждой стороны")
        .adjustment(&gtk4::Adjustment::new(
            state.settings.borrow().preview_padding_percent as f64,
            0.0,
            30.0,
            1.0,
            5.0,
            0.0,
        ))
        .build();

    let state_padding = state.clone();
    let refresh_for_padding = refresh_preview.clone();
    padding_spin.connect_changed(move |row| {
        let val = row.value() as u32;
        state_padding.settings.borrow_mut().preview_padding_percent = val;
        state_padding.settings.borrow().save();
        refresh_for_padding();
    });
    layout_group.add(&padding_spin);
    page.add(&layout_group);

    // Секция 3: Шрифты редактора и предпросмотра
    let fonts_group = adw::PreferencesGroup::builder()
        .title("Шрифты")
        .description("Настройка гарнитуры и размера текста")
        .build();

    let font_options = [
        "Monospace",
        "JetBrains Mono",
        "Fira Code",
        "Source Code Pro",
        "Ubuntu Mono",
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Cantarell",
        "Sans",
    ];

    let font_family_row = adw::ComboRow::builder()
        .title("Шрифт редактора")
        .subtitle("Выбор гарнитуры (font-family)")
        .model(&gtk4::StringList::new(&font_options))
        .build();

    let saved_font_family = state.settings.borrow().editor_font_family.clone();
    let initial_font_idx = font_options
        .iter()
        .position(|&f| f.eq_ignore_ascii_case(&saved_font_family))
        .unwrap_or(0);
    font_family_row.set_selected(initial_font_idx as u32);

    let state_font_family = state.clone();
    let buffer_for_font_family = editor_buffer.clone();
    let tag_for_font_family = editor_font_tag.clone();
    font_family_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if let Some(&selected_font) = font_options.get(idx) {
            state_font_family.settings.borrow_mut().editor_font_family = selected_font.to_string();
            state_font_family.settings.borrow().save();
            let size = state_font_family.settings.borrow().editor_font_size;
            apply_editor_font(&buffer_for_font_family, &tag_for_font_family, selected_font, size);
        }
    });
    fonts_group.add(&font_family_row);

    let editor_spin = adw::SpinRow::builder()
        .title("Размер шрифта редактора")
        .adjustment(&gtk4::Adjustment::new(
            state.settings.borrow().editor_font_size as f64,
            8.0,
            48.0,
            1.0,
            2.0,
            0.0,
        ))
        .build();

    let state_editor_font = state.clone();
    let buffer_for_spin = editor_buffer.clone();
    let tag_for_spin = editor_font_tag.clone();
    editor_spin.connect_changed(move |row| {
        let size = row.value() as u32;
        state_editor_font.settings.borrow_mut().editor_font_size = size;
        state_editor_font.settings.borrow().save();
        let family = state_editor_font.settings.borrow().editor_font_family.clone();
        apply_editor_font(&buffer_for_spin, &tag_for_spin, &family, size);
    });
    fonts_group.add(&editor_spin);

    let preview_spin = adw::SpinRow::builder()
        .title("Размер шрифта предпросмотра")
        .adjustment(&gtk4::Adjustment::new(
            state.settings.borrow().preview_font_size as f64,
            10.0,
            48.0,
            1.0,
            2.0,
            0.0,
        ))
        .build();

    let state_preview_font = state.clone();
    let refresh_for_font = refresh_preview.clone();
    preview_spin.connect_changed(move |row| {
        let size = row.value() as u32;
        state_preview_font.settings.borrow_mut().preview_font_size = size;
        state_preview_font.settings.borrow().save();
        refresh_for_font();
    });
    fonts_group.add(&preview_spin);

    page.add(&fonts_group);
    window.add(&page);
    window.present();
}

/// Отображает стандартное окно «О программе» (`adw::AboutWindow`).
fn show_about_dialog(parent: &adw::ApplicationWindow) {
    let about = adw::AboutWindow::builder()
        .transient_for(parent)
        .modal(true)
        .application_name("rmd-view")
        .version("0.1.0")
        .developer_name("Developer")
        .comments("Быстрый просмотрщик и редактор Markdown в стиле GitHub на Rust и GTK4 / Libadwaita")
        .issue_url("https://github.com")
        .license_type(gtk4::License::MitX11)
        .build();
    about.present();
}
