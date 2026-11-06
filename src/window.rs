use adw::prelude::*;
use gtk4::gio;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use webkit6::prelude::*;

use crate::renderer;
use crate::settings::AppSettings;

/// Состояние приложения, разделяемое между различными обработчиками сигналов GTK.
///
/// В GTK4 обработчики событий (closures) требуют владения захваченными переменными.
/// Связка `Rc<RefCell<T>>` позволяет безопасно разделять и изменять общее состояние
/// в пределах одного потока UI без блокировок.
#[derive(Clone)]
pub struct AppState {
    /// Путь к текущему открытому файлу (None, если файл новый / безымянный).
    pub current_file: Rc<RefCell<Option<PathBuf>>>,
    /// Флаг наличия несохраненных изменений в тексте.
    pub is_modified: Rc<RefCell<bool>>,
    /// Текущие пользовательские настройки (тема, шрифты).
    pub settings: Rc<RefCell<AppSettings>>,
    /// Активен ли сейчас режим предпросмотра (true) или редактор (false).
    pub is_preview_active: Rc<RefCell<bool>>,
}

/// Создает и конфигурирует главное окно приложения.
///
/// # Аргументы
/// * `app` — ссылка на экземпляр `adw::Application`.
/// * `initial_file` — опциональный путь к файлу для открытия при старте.
pub fn build_window(app: &adw::Application, initial_file: Option<PathBuf>) -> adw::ApplicationWindow {
    // Загружаем сохраненные настройки
    let settings = AppSettings::load();
    let app_state = AppState {
        current_file: Rc::new(RefCell::new(initial_file.clone())),
        is_modified: Rc::new(RefCell::new(false)),
        settings: Rc::new(RefCell::new(settings)),
        is_preview_active: Rc::new(RefCell::new(false)),
    };

    // Применяем цветовую схему из настроек
    apply_color_scheme(&app_state.settings.borrow().theme);

    // Создаем главное окно в стиле Libadwaita
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("rmd-view")
        .default_width(960)
        .default_height(720)
        .build();

    // Корневой вертикальный контейнер: Заголовок + Строка поиска + Рабочая область
    let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // --- 1. Заголовочная панель (HeaderBar) ---
    let header_bar = adw::HeaderBar::new();

    // Кнопка открытия файла
    let open_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    open_btn.set_tooltip_text(Some("Открыть файл (Ctrl+O)"));
    header_bar.pack_start(&open_btn);

    // Кнопка сохранения файла
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
        .group(&edit_btn) // Объединяем кнопки в одну группу (радио-режим)
        .build();

    let switch_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    switch_box.add_css_class("linked"); // Стиль GNOME для связанных кнопок
    switch_box.append(&edit_btn);
    switch_box.append(&preview_btn);
    header_bar.set_title_widget(Some(&switch_box));

    // Кнопка активации поиска
    let search_btn = gtk4::ToggleButton::builder()
        .icon_name("edit-find-symbolic")
        .tooltip_text("Поиск подстроки (Ctrl+F)")
        .build();
    header_bar.pack_end(&search_btn);

    // Кнопка главного меню (гамбургер)
    let menu_btn = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Главное меню")
        .build();

    // Описываем модель меню через GMenu
    let menu = gio::Menu::new();
    menu.append(Some("Сохранить как..."), Some("win.save_as"));
    menu.append(Some("Настройки"), Some("win.preferences"));
    menu.append(Some("О программе"), Some("win.about"));
    menu_btn.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_btn);

    root_box.append(&header_bar);

    // --- 2. Панель поиска (GtkSearchBar + GtkSearchEntry) ---
    let search_bar = gtk4::SearchBar::new();
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_hexpand(true);
    let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    search_box.append(&search_entry);
    search_bar.set_child(Some(&search_box));
    search_bar.connect_entry(&search_entry);
    // Связываем состояние кнопки поиска в шапке с видимостью панели поиска
    search_bar.bind_property("search-mode-enabled", &search_btn, "active")
        .bidirectional()
        .sync_create()
        .build();
    root_box.append(&search_bar);

    // --- 3. Рабочая область: Редактор текста ---
    let text_view = gtk4::TextView::builder()
        .wrap_mode(gtk4::WrapMode::Word)
        .left_margin(18)
        .right_margin(18)
        .top_margin(14)
        .bottom_margin(14)
        .monospace(true)
        .build();

    let editor_scroll = gtk4::ScrolledWindow::builder()
        .child(&text_view)
        .hexpand(true)
        .vexpand(true)
        .build();

    view_stack.add_titled(&editor_scroll, Some("editor"), "Редактор");

    // --- 4. Рабочая область: Предпросмотр WebKitGTK ---
    let web_view = webkit6::WebView::new();
    let preview_scroll = gtk4::ScrolledWindow::builder()
        .child(&web_view)
        .hexpand(true)
        .vexpand(true)
        .build();

    view_stack.add_titled(&preview_scroll, Some("preview"), "Просмотр");
    root_box.append(&view_stack);
    window.set_content(Some(&root_box));

    // Применяем размер шрифта редактора из настроек
    apply_editor_font(&text_view, app_state.settings.borrow().editor_font_size);

    // Если был передан файл при запуске — загружаем его содержимое
    let buffer = text_view.buffer();
    if let Some(ref path) = initial_file {
        if let Ok(content) = std::fs::read_to_string(path) {
            buffer.set_text(&content);
            *app_state.is_modified.borrow_mut() = false;
        }
    }
    update_window_title(&window, &app_state);

    // Отслеживаем изменения в буфере текста, чтобы ставить индикатор `*` в заголовке
    let state_clone = app_state.clone();
    let window_weak = window.downgrade();
    buffer.connect_changed(move |_| {
        *state_clone.is_modified.borrow_mut() = true;
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
            let font_size = app_state.settings.borrow().preview_font_size;
            let html = renderer::render_markdown_to_html(text.as_str(), is_dark, font_size);
            web_view.load_html(&html, None);
        })
    };

    // Логика переключения режимов (Редактор <-> Предпросмотр)
    let refresh_for_switch = refresh_preview.clone();
    let view_stack_clone = view_stack.clone();
    let edit_btn_clone = edit_btn.clone();
    let preview_btn_clone = preview_btn.clone();
    let text_view_for_toggle = text_view.clone();
    let state_toggle = app_state.clone();
    let toggle_view = Rc::new(move |to_preview: bool| {
        *state_toggle.is_preview_active.borrow_mut() = to_preview;
        if to_preview {
            // При переходе в просмотр — рендерим актуальный текст из буфера
            refresh_for_switch();
            view_stack_clone.set_visible_child_name("preview");
            preview_btn_clone.set_active(true);
        } else {
            // При переходе в редактор — возвращаем фокус на поле ввода
            view_stack_clone.set_visible_child_name("editor");
            edit_btn_clone.set_active(true);
            text_view_for_toggle.grab_focus();
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

    // --- Обработчик открытия файла (GtkFileDialog) ---
    let window_for_open = window.clone();
    let buffer_for_open = buffer.clone();
    let state_for_open = app_state.clone();
    open_btn.connect_clicked(move |_| {
        let file_dialog = gtk4::FileDialog::builder()
            .title("Открыть Markdown файл")
            .build();

        // Настраиваем фильтр расширений файлов
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

        // Асинхронный вызов системного диалога открытия файла
        file_dialog.open(Some(&window_for_open), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        buffer.set_text(&content);
                        *state.current_file.borrow_mut() = Some(path);
                        *state.is_modified.borrow_mut() = false;
                        update_window_title(&win, &state);
                    }
                }
            }
        });
    });

    // --- Обработчик сохранения файла (Save / Save As) ---
    let save_action_state = app_state.clone();
    let save_action_window = window.clone();
    let save_action_buffer = buffer.clone();
    let do_save = Rc::new(move |force_save_as: bool| {
        let current_path = save_action_state.current_file.borrow().clone();
        if let (false, Some(path)) = (force_save_as, current_path) {
            // Если путь уже известен и это не "Сохранить как..." — пишем сразу в файл
            let start = save_action_buffer.start_iter();
            let end = save_action_buffer.end_iter();
            let text = save_action_buffer.text(&start, &end, false);
            if std::fs::write(&path, text.as_bytes()).is_ok() {
                *save_action_state.is_modified.borrow_mut() = false;
                update_window_title(&save_action_window, &save_action_state);
            }
        } else {
            // Иначе запрашиваем путь через диалог сохранения
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

    // --- Регистрация действий окна (GAction) для меню и шорткатов ---
    let actions = gio::SimpleActionGroup::new();

    // Действие win.save_as
    let do_save_as = do_save.clone();
    let save_as_action = gio::SimpleAction::new("save_as", None);
    save_as_action.connect_activate(move |_, _| {
        do_save_as(true);
    });
    actions.add_action(&save_as_action);

    // Действие win.save
    let do_save_shortcut = do_save.clone();
    let save_action = gio::SimpleAction::new("save", None);
    save_action.connect_activate(move |_, _| {
        do_save_shortcut(false);
    });
    actions.add_action(&save_action);

    // Действие win.open
    let open_btn_clone = open_btn.clone();
    let open_action = gio::SimpleAction::new("open", None);
    open_action.connect_activate(move |_, _| {
        open_btn_clone.emit_clicked();
    });
    actions.add_action(&open_action);

    // Действие win.toggle_preview (Ctrl+P)
    let toggle_state = app_state.clone();
    let toggle_view_shortcut = toggle_view.clone();
    let preview_action = gio::SimpleAction::new("toggle_preview", None);
    preview_action.connect_activate(move |_, _| {
        let is_prev = *toggle_state.is_preview_active.borrow();
        toggle_view_shortcut(!is_prev);
    });
    actions.add_action(&preview_action);

    // Действие win.find (Ctrl+F)
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

    // Действие win.preferences
    let window_for_pref = window.clone();
    let text_view_for_pref = text_view.clone();
    let state_for_pref = app_state.clone();
    let refresh_pref = refresh_preview.clone();
    let pref_action = gio::SimpleAction::new("preferences", None);
    pref_action.connect_activate(move |_, _| {
        show_preferences_dialog(&window_for_pref, &state_for_pref, &text_view_for_pref, refresh_pref.clone());
    });
    actions.add_action(&pref_action);

    // Действие win.about
    let window_for_about = window.clone();
    let about_action = gio::SimpleAction::new("about", None);
    about_action.connect_activate(move |_, _| {
        show_about_dialog(&window_for_about);
    });
    actions.add_action(&about_action);

    window.insert_action_group("win", Some(&actions));

    // Настраиваем логику поиска подстроки
    setup_search(&search_entry, &text_view);

    // Настраиваем глобальные горячие клавиши (акселераторы)
    app.set_accels_for_action("win.save", &["<Control>s"]);
    app.set_accels_for_action("win.save_as", &["<Control><Shift>s"]);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    // Переключение между вкладками Редактор и Просмотр по Ctrl+Tab / Ctrl+Shift+Tab
    app.set_accels_for_action("win.toggle_preview", &["<Control>Tab", "<Control>ISO_Left_Tab"]);
    app.set_accels_for_action("win.find", &["<Control>f"]);

    window
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

/// Применяет выбранную цветовую схему (Adwaita ColorScheme).
fn apply_color_scheme(theme: &str) {
    let style_mgr = adw::StyleManager::default();
    match theme {
        "dark" => style_mgr.set_color_scheme(adw::ColorScheme::ForceDark),
        "light" => style_mgr.set_color_scheme(adw::ColorScheme::ForceLight),
        _ => style_mgr.set_color_scheme(adw::ColorScheme::Default),
    }
}

/// Динамически изменяет размер шрифта в `GtkTextView` с помощью `CssProvider`.
fn apply_editor_font(_text_view: &gtk4::TextView, font_size: u32) {
    let css = format!("textview text {{ font-size: {font_size}pt; }}");
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(&css);

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Настраивает механизм поиска по текстовому буферу `GtkTextBuffer`.
///
/// Для каждого найденного вхождения применяется текстовый тег (`TextTag`) с подсветкой фона.
/// Курсор и прокрутка автоматически переводятся на первое совпадение.
fn setup_search(entry: &gtk4::SearchEntry, text_view: &gtk4::TextView) {
    let buffer = text_view.buffer();
    let text_view = text_view.clone();

    // Создаем тег подсветки найденных фрагментов текста
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
        // Очищаем предыдущую подсветку
        buffer.remove_tag_by_name("search-match", &start, &end);

        if query.is_empty() {
            return;
        }

        let mut iter = buffer.start_iter();
        let mut first_match = None;

        // Ищем все вхождения подстроки без учета регистра
        while let Some((match_start, match_end)) = iter.forward_search(&query, gtk4::TextSearchFlags::CASE_INSENSITIVE, None) {
            buffer.apply_tag_by_name("search-match", &match_start, &match_end);
            if first_match.is_none() {
                first_match = Some(match_start);
            }
            iter = match_end;
        }

        // Прокручиваем редактор к первому совпадению
        if let Some(pos) = first_match {
            buffer.place_cursor(&pos);
            let mark = buffer.create_mark(None, &pos, false);
            text_view.scroll_to_mark(&mark, 0.1, true, 0.0, 0.5);
            buffer.delete_mark(&mark);
        }
    });
}

/// Отображает модальное окно настроек (`adw::PreferencesWindow`) в стиле GNOME.
fn show_preferences_dialog(
    parent: &adw::ApplicationWindow,
    state: &AppState,
    text_view: &gtk4::TextView,
    refresh_preview: Rc<dyn Fn()>,
) {
    let window = adw::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .title("Настройки")
        .default_width(450)
        .build();

    let page = adw::PreferencesPage::new();

    // Секция 1: Цветовая схема
    let appearance_group = adw::PreferencesGroup::builder()
        .title("Оформление")
        .description("Выбор цветовой темы приложения")
        .build();

    let theme_row = adw::ComboRow::builder()
        .title("Цветовая схема")
        .model(&gtk4::StringList::new(&[
            "Системная",
            "Светлая",
            "Тёмная",
        ]))
        .build();

    let current_theme = state.settings.borrow().theme.clone();
    theme_row.set_selected(match current_theme.as_str() {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    });

    let state_theme = state.clone();
    let refresh_for_theme = refresh_preview.clone();
    theme_row.connect_selected_notify(move |row| {
        let selected_theme = match row.selected() {
            1 => "light",
            2 => "dark",
            _ => "system",
        };
        state_theme.settings.borrow_mut().theme = selected_theme.to_string();
        state_theme.settings.borrow().save();
        apply_color_scheme(selected_theme);
        refresh_for_theme();
    });

    appearance_group.add(&theme_row);
    page.add(&appearance_group);

    // Секция 2: Размеры шрифтов
    let fonts_group = adw::PreferencesGroup::builder()
        .title("Шрифты")
        .description("Настройка масштаба текста")
        .build();

    // Размер шрифта текстового редактора
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

    let text_view_clone = text_view.clone();
    let state_editor_font = state.clone();
    editor_spin.connect_changed(move |row| {
        let size = row.value() as u32;
        state_editor_font.settings.borrow_mut().editor_font_size = size;
        state_editor_font.settings.borrow().save();
        apply_editor_font(&text_view_clone, size);
    });
    fonts_group.add(&editor_spin);

    // Размер шрифта предпросмотра Markdown
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
