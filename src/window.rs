use adw::prelude::*;
use gtk4::gio;
use gtk4::glib;
use gtk4::pango;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use webkit6::prelude::*;

use crate::highlighter;
use crate::i18n::Language;
use crate::renderer;
use crate::settings::AppSettings;

/// Состояние приложения, разделяемое между различными обработчиками сигналов GTK.
#[derive(Clone)]
pub struct AppState {
    /// Путь к текущему открытому файлу (None, если файл новый / безымянный).
    pub current_file: Rc<RefCell<Option<PathBuf>>>,
    /// Флаг наличия несохраненных изменений в тексте.
    pub is_modified: Rc<RefCell<bool>>,
    /// Текущие пользовательские настройки (тема, шрифты, отступы, состояние окна, язык).
    pub settings: Rc<RefCell<AppSettings>>,
    /// Активен ли сейчас режим предпросмотра (true) или редактор (false).
    pub is_preview_active: Rc<RefCell<bool>>,
    /// Целевой номер строки Markdown-исходника (0-based) для идеальной синхронизации позиции между редактором и предпросмотром
    pub target_line: Rc<RefCell<Option<usize>>>,
}

/// Перечисляет все моноширинные семейства шрифтов, доступные в системе через Pango Context.
///
/// Добавляет стандартный системный псевдоним "Monospace", если он отсутствует,
/// сортирует список по алфавиту без учета регистра и удаляет дубликаты.
pub fn get_system_monospace_fonts() -> Vec<String> {
    // В GTK4 PangoContext создается через любой виджет (например, временный Label)
    let temp_widget = gtk4::Label::new(None);
    let context = temp_widget.create_pango_context();
    let mut families: Vec<String> = context
        .list_families()
        .into_iter()
        .filter(|f| f.is_monospace())
        .map(|f| f.name().to_string())
        .collect();

    // Гарантируем наличие базового системного шрифта Monospace
    if !families.iter().any(|f| f.eq_ignore_ascii_case("Monospace")) {
        families.push("Monospace".to_string());
    }

    // Сортировка по алфавиту без учета регистра
    families.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    families
}

/// Создает и конфигурирует главное окно приложения.
pub fn build_window(app: &adw::Application, initial_file: Option<PathBuf>) -> adw::ApplicationWindow {
    let settings = AppSettings::load();
    let app_state = AppState {
        current_file: Rc::new(RefCell::new(initial_file.clone())),
        is_modified: Rc::new(RefCell::new(false)),
        settings: Rc::new(RefCell::new(settings.clone())),
        is_preview_active: Rc::new(RefCell::new(false)),
        target_line: Rc::new(RefCell::new(None)),
    };

    // Отключаем системный звуковой сигнал (error bell) при достижении границ документа
    // в глобальных настройках GTK и для текущего дисплея GDK.
    if let Some(settings) = gtk4::Settings::default() {
        settings.set_gtk_error_bell(false);
    }
    if let Some(display) = gtk4::gdk::Display::default() {
        let settings = gtk4::Settings::for_display(&display);
        settings.set_gtk_error_bell(false);
    }

    // Применяем цветовую схему из настроек
    apply_color_scheme(&app_state.settings.borrow().theme);

    // Получаем текущий выбранный язык
    let current_lang = Language::from_code(&app_state.settings.borrow().language);

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
    open_btn.set_tooltip_text(Some(current_lang.tooltip_open()));
    header_bar.pack_start(&open_btn);

    let save_btn = gtk4::Button::from_icon_name("document-save-symbolic");
    save_btn.set_tooltip_text(Some(current_lang.tooltip_save()));
    header_bar.pack_start(&save_btn);

    // Переключатель вида (Редактор <-> Просмотр) в центре HeaderBar
    let view_stack = adw::ViewStack::new();

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
    menu.append(Some(current_lang.menu_save_as()), Some("win.save_as"));
    menu.append(Some(current_lang.menu_preferences()), Some("win.preferences"));
    menu.append(Some(current_lang.menu_about()), Some("win.about"));
    menu.append(Some(current_lang.menu_close()), Some("win.close"));
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

    // Обработка клавиш в редакторе (Tab/Shift+Tab для отступов, подавление звука границ)
    setup_editor_key_handling(&text_view);

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

    // Создаем виджет нумерации строк (Gutter DrawingArea) для левой колонки редактора
    let line_numbers_gutter = setup_line_numbers_gutter(&text_view, &app_state);

    let editor_scroll = gtk4::ScrolledWindow::builder()
        .child(&text_view)
        .hexpand(true)
        .vexpand(true)
        .build();

    // При прокрутке редактора принудительно перерисовываем номера строк
    let gutter_for_scroll = line_numbers_gutter.clone();
    editor_scroll.vadjustment().connect_value_changed(move |_| {
        gutter_for_scroll.queue_draw();
    });

    let editor_page = view_stack.add_titled(&editor_scroll, Some("editor"), current_lang.tab_editor());

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

    let preview_page = view_stack.add_titled(&preview_scroll, Some("preview"), current_lang.tab_preview());
    root_box.append(&view_stack);
    window.set_content(Some(&root_box));

    // Функция динамического обновления текстов UI при смене языка
    let update_ui_language = {
        let open_btn = open_btn.clone();
        let save_btn = save_btn.clone();
        let edit_btn = edit_btn.clone();
        let preview_btn = preview_btn.clone();
        let search_btn = search_btn.clone();
        let menu_btn = menu_btn.clone();
        let editor_page = editor_page.clone();
        let preview_page = preview_page.clone();
        let menu = menu.clone();
        let window = window.clone();
        let state = app_state.clone();

        Rc::new(move |lang: Language| {
            open_btn.set_tooltip_text(Some(lang.tooltip_open()));
            save_btn.set_tooltip_text(Some(lang.tooltip_save()));
            edit_btn.set_tooltip_text(Some(lang.tooltip_edit_mode()));
            preview_btn.set_tooltip_text(Some(lang.tooltip_preview_mode()));
            search_btn.set_tooltip_text(Some(lang.tooltip_search()));
            menu_btn.set_tooltip_text(Some(lang.tooltip_menu()));

            editor_page.set_title(Some(lang.tab_editor()));
            preview_page.set_title(Some(lang.tab_preview()));

            menu.remove_all();
            menu.append(Some(lang.menu_save_as()), Some("win.save_as"));
            menu.append(Some(lang.menu_preferences()), Some("win.preferences"));
            menu.append(Some(lang.menu_about()), Some("win.about"));
            menu.append(Some(lang.menu_close()), Some("win.close"));

            update_window_title_and_save_status(&window, &save_btn, &state);
        })
    };

    // Применяем семейство и размер шрифта редактора
    apply_editor_font(
        &buffer,
        &editor_font_tag,
        &app_state.settings.borrow().editor_font_family,
        app_state.settings.borrow().editor_font_size,
    );

    // Загрузка начального содержимого, если был передан путь к файлу
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
            // Восстанавливаем сохраненную позицию курсора (или устанавливаем в начало (0, 0)).
            // Это решает проблему «замирания» при старте и прокрутки в конец документа.
            apply_restored_cursor_position(&app_state, &buffer, path);
            schedule_cursor_scroll(&text_view);
        }
    } else {
        // Для нового пустого документа позиционируем курсор в начале буфера (0, 0)
        buffer.place_cursor(&buffer.start_iter());
    }

    // Первоначальное обновление заголовка окна и кнопки «Сохранить»
    update_window_title_and_save_status(&window, &save_btn, &app_state);

    // Отслеживаем изменения текста пользователем
    let state_for_buffer_change = app_state.clone();
    let window_for_buffer_change = window.clone();
    let save_btn_for_buffer_change = save_btn.clone();
    let gutter_for_buffer = line_numbers_gutter.clone();
    buffer.connect_changed(move |buf| {
        if !*state_for_buffer_change.is_modified.borrow() {
            *state_for_buffer_change.is_modified.borrow_mut() = true;
            update_window_title_and_save_status(
                &window_for_buffer_change,
                &save_btn_for_buffer_change,
                &state_for_buffer_change,
            );
        }
        gutter_for_buffer.queue_draw();
        highlighter::highlight_markdown_buffer(buf);
    });

    // Замыкание обновления содержимого WebKit (HTML-рендеринг Markdown)
    let buffer_for_refresh = buffer.clone();
    let state_for_refresh = app_state.clone();
    let web_view_for_refresh = web_view.clone();
    let refresh_preview = Rc::new(move || {
        let start = buffer_for_refresh.start_iter();
        let end = buffer_for_refresh.end_iter();
        let markdown_text = buffer_for_refresh.text(&start, &end, false);

        let style_manager = adw::StyleManager::default();
        let is_dark_system = style_manager.is_dark();

        let base_uri = state_for_refresh.current_file.borrow().as_ref().and_then(|p| {
            p.parent().and_then(|parent_dir| {
                parent_dir.to_str().map(|s| {
                    if !s.ends_with('/') {
                        format!("file://{s}/")
                    } else {
                        format!("file://{s}")
                    }
                })
            })
        });

        let target_line = *state_for_refresh.target_line.borrow();
        let settings = state_for_refresh.settings.borrow();
        let html = renderer::render_markdown_to_html(
            &markdown_text,
            &settings.theme,
            is_dark_system,
            settings.preview_font_size,
            settings.preview_padding_percent,
            base_uri.as_deref(),
            target_line,
        );

        web_view_for_refresh.load_html(&html, base_uri.as_deref());
    });

    // Синхронизация прокрутки при загрузке документа в WebKit
    let web_view_for_load = web_view.clone();
    let state_for_load = app_state.clone();
    web_view.connect_load_changed(move |_, load_event| {
        // При завершении загрузки документа повторно вызываем позиционирование на целевой строке.
        // Это служит надежной страховкой на случай, если движок WebKit выполнил внутренний сброс
        // прокрутки в (0,0) после первоначального разбора HTML.
        if load_event == webkit6::LoadEvent::Finished {
            if let Some(line) = *state_for_load.target_line.borrow() {
                let script = format!(
                    "if (typeof window.__scrollToSourceLine === 'function') {{ window.__scrollToSourceLine({}); }}",
                    line
                );
                web_view_for_load.evaluate_javascript(
                    &script,
                    None,
                    None,
                    Option::<&gio::Cancellable>::None,
                    |_| {},
                );
            }
        }
    });

    // Обработка переключения режимов (Редактор <-> Предпросмотр)
    let view_stack_clone = view_stack.clone();
    let refresh_preview_for_toggle = refresh_preview.clone();
    let state_for_toggle = app_state.clone();
    let edit_btn_clone = edit_btn.clone();
    let preview_btn_clone = preview_btn.clone();
    let editor_scroll_clone = editor_scroll.clone();
    let text_view_clone = text_view.clone();
    let web_view_for_sync = web_view.clone();

    let switch_to_preview = {
        let view_stack_clone = view_stack_clone.clone();
        let refresh_preview_for_toggle = refresh_preview_for_toggle.clone();
        let state_for_toggle = state_for_toggle.clone();
        let editor_scroll_clone = editor_scroll_clone.clone();
        let text_view_clone = text_view_clone.clone();
        let preview_btn_clone = preview_btn_clone.clone();

        Rc::new(move || {
            // Предотвращаем повторный / рекурсивный запуск, если предпросмотр уже активен
            if *state_for_toggle.is_preview_active.borrow() {
                return;
            }

            let vadj = editor_scroll_clone.vadjustment();
            let upper = vadj.upper();
            let page_size = vadj.page_size();
            let visible_top = vadj.value();
            let visible_bottom = visible_top + page_size;

            let buffer = text_view_clone.buffer();
            let insert_mark = buffer.get_insert();
            let cursor_iter = buffer.iter_at_mark(&insert_mark);
            let cursor_line = cursor_iter.line() as usize;

            let (cursor_y, cursor_height) = text_view_clone.line_yrange(&cursor_iter);
            let cursor_mid_y = (cursor_y + cursor_height / 2) as f64;

            // Определяем целевую строку в исходнике:
            // Если курсор находится в видимой области (или документ только открыт),
            // строго позиционируемся по строке курсора, как ожидает пользователь.
            // Если курсор далеко за пределами видимого экрана, берем строку в центре экрана.
            let target_line = if cursor_mid_y >= visible_top && cursor_mid_y <= visible_bottom && cursor_height > 0 {
                cursor_line
            } else if upper > 0.0 && page_size > 0.0 {
                let center_y = (visible_top + page_size / 2.0).clamp(0.0, upper);
                let (it, _) = text_view_clone.line_at_y(center_y as i32);
                it.line() as usize
            } else {
                cursor_line
            };

            *state_for_toggle.target_line.borrow_mut() = Some(target_line);
            *state_for_toggle.is_preview_active.borrow_mut() = true;

            refresh_preview_for_toggle();
            view_stack_clone.set_visible_child_name("preview");
            preview_btn_clone.set_active(true);
        })
    };

    let switch_to_editor = {
        let view_stack_clone = view_stack_clone.clone();
        let state_for_toggle = state_for_toggle.clone();
        let text_view_clone = text_view_clone.clone();
        let edit_btn_clone = edit_btn_clone.clone();
        let web_view_for_sync = web_view_for_sync.clone();

        Rc::new(move || {
            // Предотвращаем повторный / рекурсивный запуск, если редактор уже активен
            if !*state_for_toggle.is_preview_active.borrow() {
                return;
            }

            // Запрашиваем номер строки Markdown-исходника, находящейся в центре текущей области просмотра WebKit
            let script = "if (typeof window.__getCenterSourceLine === 'function') { window.__getCenterSourceLine(); } else { -1; }";
            let text_view_sync = text_view_clone.clone();

            web_view_for_sync.evaluate_javascript(
                script,
                None,
                None,
                Option::<&gio::Cancellable>::None,
                move |result| {
                    if let Ok(js_val) = result {
                        let line = js_val.to_double() as i32;
                        if line >= 0 {
                            let buffer = text_view_sync.buffer();
                            if let Some(mut iter) = buffer.iter_at_line(line) {
                                buffer.place_cursor(&iter);
                                text_view_sync.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.5);
                            }
                        }
                    }
                },
            );

            *state_for_toggle.is_preview_active.borrow_mut() = false;
            view_stack_clone.set_visible_child_name("editor");
            edit_btn_clone.set_active(true);
        })
    };

    let to_preview_clone = switch_to_preview.clone();
    preview_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            to_preview_clone();
        }
    });

    let to_editor_clone = switch_to_editor.clone();
    edit_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            to_editor_clone();
        }
    });

    // Обработчик сохранения файла
    let save_action_buffer = buffer.clone();
    let save_action_state = app_state.clone();
    let save_action_window = window.clone();
    let save_action_btn = save_btn.clone();
    let do_save = Rc::new(move |save_as: bool| {
        let current_path = save_action_state.current_file.borrow().clone();
        let lang = Language::from_code(&save_action_state.settings.borrow().language);

        if !save_as && current_path.is_some() {
            let path = current_path.unwrap();
            let start = save_action_buffer.start_iter();
            let end = save_action_buffer.end_iter();
            let text = save_action_buffer.text(&start, &end, false);
            if std::fs::write(&path, text.as_bytes()).is_ok() {
                *save_action_state.is_modified.borrow_mut() = false;
                update_window_title_and_save_status(&save_action_window, &save_action_btn, &save_action_state);
            }
        } else {
            let file_dialog = gtk4::FileDialog::builder()
                .title(lang.dialog_save_title())
                .initial_name("document.md")
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some(lang.filter_markdown()));
            filter.add_pattern("*.md");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            file_dialog.set_filters(Some(&filters));

            let buffer = save_action_buffer.clone();
            let state = save_action_state.clone();
            let win = save_action_window.clone();
            let btn = save_action_btn.clone();

            file_dialog.save(Some(&save_action_window), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let start = buffer.start_iter();
                        let end = buffer.end_iter();
                        let text = buffer.text(&start, &end, false);
                        if std::fs::write(&path, text.as_bytes()).is_ok() {
                            *state.current_file.borrow_mut() = Some(path);
                            *state.is_modified.borrow_mut() = false;
                            update_window_title_and_save_status(&win, &btn, &state);
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
    let save_btn_for_open = save_btn.clone();
    let buffer_for_open = buffer.clone();
    let state_for_open = app_state.clone();
    let do_save_for_open = do_save.clone();
    let font_tag_for_open = editor_font_tag.clone();
    let text_view_for_open = text_view.clone();
    let trigger_file_open = Rc::new(move || {
        let lang = Language::from_code(&state_for_open.settings.borrow().language);
        let file_dialog = gtk4::FileDialog::builder()
            .title(lang.dialog_open_title())
            .build();

        let filter = gtk4::FileFilter::new();
        filter.set_name(Some(lang.filter_markdown()));
        filter.add_pattern("*.md");
        filter.add_pattern("*.markdown");
        filter.add_pattern("*.rmd");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let buffer = buffer_for_open.clone();
        let state = state_for_open.clone();
        let win = window_for_open.clone();
        let btn = save_btn_for_open.clone();
        let tag = font_tag_for_open.clone();
        let text_view_open = text_view_for_open.clone();

        file_dialog.open(Some(&window_for_open), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    // Сохраняем позицию курсора текущего закрываемого документа перед загрузкой нового
                    save_current_file_cursor_position(&state, &buffer);
                    state.settings.borrow().save();

                    if let Ok(content) = std::fs::read_to_string(&path) {
                        buffer.set_text(&content);
                        let family = state.settings.borrow().editor_font_family.clone();
                        let size = state.settings.borrow().editor_font_size;
                        apply_editor_font(&buffer, &tag, &family, size);
                        highlighter::highlight_markdown_buffer(&buffer);
                        *state.current_file.borrow_mut() = Some(path.clone());
                        *state.is_modified.borrow_mut() = false;
                        update_window_title_and_save_status(&win, &btn, &state);

                        // Восстанавливаем позицию курсора для открытого документа и прокручиваем к ней
                        apply_restored_cursor_position(&state, &buffer, &path);
                        schedule_cursor_scroll(&text_view_open);
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
            let lang = Language::from_code(&state_for_open_check.settings.borrow().language);
            let trigger_open_discard = trigger_file_open.clone();
            let trigger_open_save = trigger_file_open.clone();
            let do_save = do_save_for_open.clone();
            show_unsaved_changes_dialog(
                &window_for_open_check,
                lang.unsaved_open_heading(),
                lang.unsaved_open_body(),
                lang,
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

    // --- Обработчик закрытия окна с сохранением геометрии и позиции курсора ---
    let state_for_close = app_state.clone();
    let do_save_for_close = do_save.clone();
    let is_force_closing = Rc::new(RefCell::new(false));

    let buffer_for_geometry = buffer.clone();
    let save_window_geometry = {
        let state = app_state.clone();
        let buffer = buffer_for_geometry.clone();
        Rc::new(move |win: &adw::ApplicationWindow| {
            // Сохраняем позицию курсора для текущего открытого файла
            save_current_file_cursor_position(&state, &buffer);
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
            let lang = Language::from_code(&state_for_close.settings.borrow().language);
            let win_for_discard = win.clone();
            let win_for_save = win.clone();
            let flag_discard = force_closing_clone.clone();
            let flag_save = force_closing_clone.clone();
            let do_save = do_save_for_close.clone();
            let geom_discard = geometry_saver.clone();
            let geom_save = geometry_saver.clone();

            show_unsaved_changes_dialog(
                win,
                lang.unsaved_close_heading(),
                lang.unsaved_close_body(),
                lang,
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

    let do_save_action = do_save.clone();
    let save_action = gio::SimpleAction::new("save", None);
    save_action.connect_activate(move |_, _| {
        do_save_action(false);
    });
    actions.add_action(&save_action);

    let do_open_action = do_open_dialog.clone();
    let open_action = gio::SimpleAction::new("open", None);
    open_action.connect_activate(move |_, _| {
        do_open_action();
    });
    actions.add_action(&open_action);

    let window_for_close_action = window.clone();
    let close_action = gio::SimpleAction::new("close", None);
    close_action.connect_activate(move |_, _| {
        window_for_close_action.close();
    });
    actions.add_action(&close_action);

    let toggle_preview_switch = {
        let to_preview = switch_to_preview.clone();
        let to_editor = switch_to_editor.clone();
        let state = app_state.clone();
        Rc::new(move || {
            let is_prev = *state.is_preview_active.borrow();
            if is_prev {
                to_editor();
            } else {
                to_preview();
            }
        })
    };

    let toggle_preview_clone = toggle_preview_switch.clone();
    let preview_action = gio::SimpleAction::new("toggle_preview", None);
    preview_action.connect_activate(move |_, _| {
        toggle_preview_clone();
    });
    actions.add_action(&preview_action);

    let search_bar_for_action = search_bar.clone();
    let search_entry_for_action = search_entry.clone();
    let find_action = gio::SimpleAction::new("find", None);
    find_action.connect_activate(move |_, _| {
        let current = search_bar_for_action.is_search_mode();
        search_bar_for_action.set_search_mode(!current);
        if !current {
            search_entry_for_action.grab_focus();
        }
    });
    actions.add_action(&find_action);

    let window_for_pref = window.clone();
    let text_view_for_pref = text_view.clone();
    let line_numbers_gutter_for_pref = line_numbers_gutter.clone();
    let buffer_for_pref = buffer.clone();
    let tag_for_pref = editor_font_tag.clone();
    let tag_table_for_pref = tag_table.clone();
    let css_provider_for_pref = editor_theme_css_provider.clone();
    let state_for_pref = app_state.clone();
    let refresh_pref = refresh_preview.clone();
    let ui_updater_for_pref = update_ui_language.clone();

    let pref_action = gio::SimpleAction::new("preferences", None);
    pref_action.connect_activate(move |_, _| {
        show_preferences_dialog(
            &window_for_pref,
            &state_for_pref,
            &text_view_for_pref,
            &line_numbers_gutter_for_pref,
            &buffer_for_pref,
            &tag_for_pref,
            &tag_table_for_pref,
            &css_provider_for_pref,
            refresh_pref.clone(),
            ui_updater_for_pref.clone(),
        );
    });
    actions.add_action(&pref_action);

    let window_for_about = window.clone();
    let state_for_about = app_state.clone();
    let about_action = gio::SimpleAction::new("about", None);
    about_action.connect_activate(move |_, _| {
        let lang = Language::from_code(&state_for_about.settings.borrow().language);
        show_about_dialog(&window_for_about, lang);
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

/// Настраивает виджет нумерации строк (колонка Gutter слева от редактора `GtkTextView`).
///
/// В GTK4 поддерживается добавление дочерних виджетов в поля `TextWindowType::Left`
/// через метод `set_gutter`. Виджет `gtk4::DrawingArea` выполняет отрисовку
/// номеров строк, используя Cairo:
/// - Автоматически рассчитывает видимый диапазон координат строк с учетом прокрутки (`window_to_buffer_coords`).
/// - Преобразует координаты буфера в локальные координаты окна рисования (`buffer_to_window_coords`).
/// - Динамически подбирает цвет текста номеров строк в зависимости от выбранной темы оформления.
fn setup_line_numbers_gutter(text_view: &gtk4::TextView, state: &AppState) -> gtk4::DrawingArea {
    let gutter = gtk4::DrawingArea::new();
    gutter.set_content_width(48);

    let text_view_clone = text_view.clone();
    let state_for_draw = state.clone();

    // Функция отрисовки номеров строк на поверхности DrawingArea через Cairo
    gutter.set_draw_func(move |area, cr, width, height| {
        let buffer = text_view_clone.buffer();
        let total_lines = buffer.line_count();

        // Динамически подстраиваем ширину колонки номеров строк:
        // рассчитываем необходимое число цифр (минимум 3 символа)
        let digits = format!("{total_lines}").len().max(3);
        let needed_width = (digits as i32 * 9 + 20).max(40);
        if area.content_width() != needed_width {
            area.set_content_width(needed_width);
        }

        let theme = state_for_draw.settings.borrow().theme.clone();
        let style_mgr = adw::StyleManager::default();
        let is_dark = style_mgr.is_dark();

        // Задаем цвет фона колонки и цвет номеров строк в гармонии с темой
        let (bg_r, bg_g, bg_b, num_r, num_g, num_b) = match theme.as_str() {
            "dracula" => (0.15, 0.16, 0.21, 0.40, 0.42, 0.54),
            "kimbie_dark" => (0.11, 0.08, 0.05, 0.50, 0.42, 0.32),
            "solarized_dark" => (0.00, 0.14, 0.18, 0.35, 0.43, 0.46),
            "solarized_light" => (0.93, 0.90, 0.82, 0.58, 0.63, 0.63),
            "dark" => (0.04, 0.05, 0.07, 0.38, 0.42, 0.47),
            "light" => (0.97, 0.98, 0.99, 0.55, 0.58, 0.62),
            _ => {
                if is_dark {
                    (0.04, 0.05, 0.07, 0.38, 0.42, 0.47)
                } else {
                    (0.97, 0.98, 0.99, 0.55, 0.58, 0.62)
                }
            }
        };

        // Заливаем фон колонки номеров
        cr.set_source_rgb(bg_r, bg_g, bg_b);
        let _ = cr.paint();

        // Тонкая вертикальная разделительная линия справа от номеров строк
        cr.set_source_rgba(num_r, num_g, num_b, 0.25);
        cr.set_line_width(1.0);
        cr.move_to((width as f64) - 0.5, 0.0);
        cr.line_to((width as f64) - 0.5, height as f64);
        let _ = cr.stroke();

        // Определяем верхнюю и нижнюю границы видимой области буфера редактора
        let (_, top_buf_y) = text_view_clone.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, 0);
        let (_, bottom_buf_y) = text_view_clone.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, height);

        let top_buf_y = top_buf_y.max(0);
        let (mut iter, _) = text_view_clone.line_at_y(top_buf_y);

        // Настройка параметров шрифта для номеров строк
        let font_size = (state_for_draw.settings.borrow().editor_font_size as f64 * 0.85).max(9.0);
        let font_family = state_for_draw.settings.borrow().editor_font_family.clone();
        cr.select_font_face(&font_family, gtk4::cairo::FontSlant::Normal, gtk4::cairo::FontWeight::Normal);
        cr.set_font_size(font_size);
        cr.set_source_rgb(num_r, num_g, num_b);

        // Цикл прохода по всем видимым на экране строкам документа
        loop {
            let (y_buf, h_buf) = text_view_clone.line_yrange(&iter);
            let (_, win_y) = text_view_clone.buffer_to_window_coords(gtk4::TextWindowType::Left, 0, y_buf);

            let line_num = iter.line() + 1;
            let num_str = format!("{line_num}");

            // Выравнивание номера строки по правому краю с отступом 8px от линии
            if let Ok(extents) = cr.text_extents(&num_str) {
                let x = (width as f64) - 8.0 - extents.width() - extents.x_bearing();
                let y = (win_y as f64) + font_size + 2.0;

                cr.move_to(x, y);
                let _ = cr.show_text(&num_str);
            }

            // Завершаем отрисовку, когда вышли за пределы видимой по вертикали зоны или строк больше нет
            if y_buf + h_buf > bottom_buf_y || !iter.forward_line() {
                break;
            }
        }
    });

    // Устанавливаем или скрываем gutter согласно сохраненной настройке пользователя
    if state.settings.borrow().show_line_numbers {
        text_view.set_gutter(gtk4::TextWindowType::Left, Some(&gutter));
    } else {
        text_view.set_gutter(gtk4::TextWindowType::Left, None::<&gtk4::DrawingArea>);
    }

    gutter
}

/// Отображает модальный диалог подтверждения при наличии несохраненных изменений
fn show_unsaved_changes_dialog<F1, F2>(
    parent: &adw::ApplicationWindow,
    heading: &str,
    body: &str,
    lang: Language,
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

    dialog.add_response("cancel", lang.btn_cancel());
    dialog.add_response("discard", lang.btn_discard());
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.add_response("save", lang.btn_save());
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

/// Обновляет заголовок окна и состояние кнопки сохранения:
/// - Отображает имя текущего файла и элегантный маркер `●` при наличии правок (в стиле GNOME Text Editor).
/// - Подсвечивает кнопку сохранения стилем `.suggested-action` (акцентный цвет) только когда есть изменения.
/// - Динамически обновляет тултип кнопки («Сохранить (Ctrl+S) — Есть несохраненные изменения»).
fn update_window_title_and_save_status(
    window: &adw::ApplicationWindow,
    save_btn: &gtk4::Button,
    state: &AppState,
) {
    let lang = Language::from_code(&state.settings.borrow().language);
    let default_name = match lang {
        Language::Russian => "Безымянный",
        Language::English => "Untitled",
    };

    let filename = if let Some(ref path) = *state.current_file.borrow() {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.md")
            .to_string()
    } else {
        default_name.to_string()
    };

    let is_modified = *state.is_modified.borrow();

    // Маркер в заголовке: аккуратная точка '● ' перед именем файла при наличии изменений
    let modified_marker = if is_modified { "● " } else { "" };
    window.set_title(Some(&format!("{modified_marker}{filename} — rmd-view")));

    // Управление стилем и тултипом кнопки «Сохранить»
    if is_modified {
        save_btn.add_css_class("suggested-action");
        save_btn.set_tooltip_text(Some(lang.tooltip_save_unsaved()));
    } else {
        save_btn.remove_css_class("suggested-action");
        save_btn.set_tooltip_text(Some(lang.tooltip_save()));
    }
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

/// Выполняет добавление отступа (двух пробелов):
/// - Если есть выделенный участок текста: добавляет по 2 пробела в начало каждой выделенной строки
///   и сохраняет границы выделения с учетом смещения.
/// - Если выделения нет: вставляет 2 пробела в позицию каретки.
///
/// Все операции обрамляются вызовами `buffer.begin_user_action()` и `buffer.end_user_action()`,
/// обеспечивая единое действие для механизма Undo/Redo (`Ctrl+Z`).
pub fn indent_selection_or_cursor(buffer: &gtk4::TextBuffer) {
    if let Some((start_iter, end_iter)) = buffer.selection_bounds() {
        if start_iter != end_iter {
            // ВАЖНО: Извлекаем все свойства (номера строк и смещения колонок) ДО изменения буфера,
            // так как любые вставки/удаления в GtkTextBuffer делают существующие GtkTextIter невалидными.
            let start_line = start_iter.line();
            let mut end_line = end_iter.line();
            let original_end_line = end_iter.line();
            // Если выделение заканчивается ровно в начале строки (offset 0), последняя строка не входит в выделенный блок
            if end_line > start_line && end_iter.starts_line() {
                end_line -= 1;
            }

            let start_col = start_iter.line_offset();
            let end_col = end_iter.line_offset();

            buffer.begin_user_action();

            for line_idx in start_line..=end_line {
                if let Some(mut line_iter) = buffer.iter_at_line(line_idx) {
                    buffer.insert(&mut line_iter, "  ");
                }
            }

            // Восстанавливаем границы выделения со смещением на 2 пробела через свежие итераторы
            let new_start_col = if start_col > 0 { start_col + 2 } else { 0 };
            let new_end_col = if original_end_line == end_line && end_col > 0 {
                end_col + 2
            } else {
                end_col
            };

            if let Some(mut new_start) = buffer.iter_at_line(start_line) {
                let mut line_end = new_start.clone();
                line_end.forward_to_line_end();
                new_start.set_line_offset(new_start_col.min(line_end.line_offset()).max(0));

                if let Some(mut new_end) = buffer.iter_at_line(original_end_line) {
                    let mut end_line_end = new_end.clone();
                    end_line_end.forward_to_line_end();
                    new_end.set_line_offset(new_end_col.min(end_line_end.line_offset()).max(0));

                    buffer.select_range(&new_start, &new_end);
                }
            }

            buffer.end_user_action();
            return;
        }
    }

    // Если выделения нет: вставляем 2 пробела в позицию каретки
    buffer.begin_user_action();
    buffer.insert_at_cursor("  ");
    buffer.end_user_action();
}

/// Выполняет снятие отступа (удаление до двух начальных пробелов):
/// - Если есть выделенный участок текста: удаляет до 2 начальных пробелов (или 1 символ табуляции)
///   с каждой выделенной строки и адаптирует границы выделения.
/// - Если выделения нет: удаляет до 2 начальных пробелов с начала текущей строки курсора,
///   не приводя к потере фокуса виджетом.
pub fn unindent_selection_or_cursor(buffer: &gtk4::TextBuffer) {
    if let Some((start_iter, end_iter)) = buffer.selection_bounds() {
        if start_iter != end_iter {
            // ВАЖНО: Извлекаем свойства итераторов до изменения буфера
            let start_line = start_iter.line();
            let mut end_line = end_iter.line();
            let original_end_line = end_iter.line();
            // Если выделение заканчивается ровно в начале строки (offset 0), последняя строка не входит в выделенный блок
            if end_line > start_line && end_iter.starts_line() {
                end_line -= 1;
            }

            let mut start_col = start_iter.line_offset();
            let mut end_col = end_iter.line_offset();

            buffer.begin_user_action();

            for line_idx in start_line..=end_line {
                if let Some(mut line_iter) = buffer.iter_at_line(line_idx) {
                    let mut chars_to_del = 0;
                    let c1 = line_iter.char();
                    if c1 == ' ' {
                        chars_to_del += 1;
                        let mut next = line_iter.clone();
                        next.forward_char();
                        let c2 = next.char();
                        if c2 == ' ' {
                            chars_to_del += 1;
                        }
                    } else if c1 == '\t' {
                        chars_to_del = 1;
                    }

                    if chars_to_del > 0 {
                        let mut del_end = line_iter.clone();
                        del_end.forward_chars(chars_to_del);
                        buffer.delete(&mut line_iter, &mut del_end);

                        // Для знаковых i32 используем .max(0), чтобы предотвратить отрицательные оффсеты
                        if line_idx == start_line {
                            start_col = (start_col - chars_to_del).max(0);
                        }
                        if line_idx == original_end_line {
                            end_col = (end_col - chars_to_del).max(0);
                        }
                    }
                }
            }

            // Восстанавливаем границы выделения с учетом удаленных пробелов через свежие итераторы
            if let Some(mut new_start) = buffer.iter_at_line(start_line) {
                let mut line_end = new_start.clone();
                line_end.forward_to_line_end();
                new_start.set_line_offset(start_col.min(line_end.line_offset()).max(0));

                if let Some(mut new_end) = buffer.iter_at_line(original_end_line) {
                    let mut end_line_end = new_end.clone();
                    end_line_end.forward_to_line_end();
                    new_end.set_line_offset(end_col.min(end_line_end.line_offset()).max(0));

                    buffer.select_range(&new_start, &new_end);
                }
            }

            buffer.end_user_action();
            return;
        }
    }

    // Если выделения нет: убираем до двух пробелов с начала текущей строки курсора
    let insert_mark = buffer.get_insert();
    let cursor_iter = buffer.iter_at_mark(&insert_mark);
    let cur_line = cursor_iter.line();
    let cur_col = cursor_iter.line_offset();

    buffer.begin_user_action();

    let mut new_col = cur_col;
    if let Some(mut line_iter) = buffer.iter_at_line(cur_line) {
        let mut chars_to_del = 0;
        let c1 = line_iter.char();
        if c1 == ' ' {
            chars_to_del += 1;
            let mut next = line_iter.clone();
            next.forward_char();
            let c2 = next.char();
            if c2 == ' ' {
                chars_to_del += 1;
            }
        } else if c1 == '\t' {
            chars_to_del = 1;
        }

        if chars_to_del > 0 {
            let mut del_end = line_iter.clone();
            del_end.forward_chars(chars_to_del);
            buffer.delete(&mut line_iter, &mut del_end);
            new_col = (cur_col - chars_to_del).max(0);
        }
    }

    if let Some(mut new_iter) = buffer.iter_at_line(cur_line) {
        let mut line_end = new_iter.clone();
        line_end.forward_to_line_end();
        new_iter.set_line_offset(new_col.min(line_end.line_offset()).max(0));
        buffer.place_cursor(&new_iter);
    }

    buffer.end_user_action();
}

/// Возвращает канонический путь к файлу для надежного поиска в словаре сохраненных позиций курсора.
///
/// Канонизация приводит относительные пути и символические ссылки к единому абсолютному пути,
/// благодаря чему файл, открытый через консоль (`./doc.md`) или через диалоговое окно (`/home/.../doc.md`),
/// будет сопоставлен с одной и той же записью в настройках `AppSettings`.
fn get_file_canonical_key(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Сохраняет позицию курсора (номер строки и смещение символа) для текущего открытого файла в настройках.
///
/// В однопоточном GUI GTK позиция берется из встроенной метки вставки (`insert` mark) буфера.
/// Настройки модифицируются через разделяемую мутабельность `state.settings.borrow_mut()`.
/// Количество запоминаемых позиций ограничивается (максимум 200 файлов), чтобы избежать
/// бесконечного разрастания JSON-файла конфигурации в XDG_CONFIG_HOME.
fn save_current_file_cursor_position(state: &AppState, buffer: &gtk4::TextBuffer) {
    if let Some(ref path) = *state.current_file.borrow() {
        let insert_mark = buffer.get_insert();
        let iter = buffer.iter_at_mark(&insert_mark);
        let line = iter.line() as usize;
        let col = iter.line_offset() as usize;
        let key = get_file_canonical_key(path);

        let mut settings = state.settings.borrow_mut();
        if settings.file_cursor_positions.len() > 200 {
            settings.file_cursor_positions.clear();
        }
        settings.file_cursor_positions.insert(key, (line, col));
    }
}

/// Восстанавливает сохраненную позицию курсора для указанного файла или выставляет начало (0, 0).
///
/// **Почему это решает проблему зависания/замирания при открытии:**
/// При вызове `buffer.set_text(...)` GTK по умолчанию оставляет курсор в самом конце текста.
/// Это заставляет виджет `GtkTextView` вычислять разметку и прокручивать весь документ до конца,
/// что на больших файлах вызывает ощутимый фриз UI.
/// Немедленная установка курсора на сохраненную строку (или в начало (0, 0)) гарантирует,
/// что GTK форматирует только видимую начальную область текста.
fn apply_restored_cursor_position(
    state: &AppState,
    buffer: &gtk4::TextBuffer,
    path: &std::path::Path,
) {
    let key = get_file_canonical_key(path);
    let (line, col) = state
        .settings
        .borrow()
        .file_cursor_positions
        .get(&key)
        .copied()
        .unwrap_or((0, 0));

    let line_count = buffer.line_count();
    let target_line = (line as i32).min(line_count.saturating_sub(1)).max(0);
    let mut iter = buffer
        .iter_at_line(target_line)
        .unwrap_or_else(|| buffer.start_iter());
    let chars_in_line = iter.chars_in_line();
    let valid_col = (col as i32).min(chars_in_line).max(0);
    iter.set_line_offset(valid_col);

    buffer.place_cursor(&iter);
}

/// Планирует плавную прокрутку редактора к текущей позиции курсора.
///
/// Используется `glib::idle_add_local_once`, так как на момент вызова окно еще может
/// находиться в процессе компоновки/разметки виджетов GTK. Запуск прокрутки в следующем
/// цикле главного потока событий гарантирует корректный расчет геометрии скролла.
fn schedule_cursor_scroll(text_view: &gtk4::TextView) {
    let text_view_clone = text_view.clone();
    glib::idle_add_local_once(move || {
        let buf = text_view_clone.buffer();
        let insert_mark = buf.get_insert();
        let mut iter = buf.iter_at_mark(&insert_mark);
        if iter.line() == 0 {
            text_view_clone.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.0);
        } else {
            text_view_clone.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.5);
        }
    });
}

/// Настраивает обработку горячих клавиш в редакторе текста `GtkTextView`.
///
/// Контроллер `gtk4::EventControllerKey` работает в фазе `PropagationPhase::Capture`:
/// 1. **Подавление системного звукового сигнала (error bell)**:
///    В GTK попытка перемещения курсора за пределы буфера (стрелка «Вниз» в конце документа
///    или стрелка «Вверх» в начале документа) вызывает `gtk_widget_error_bell`, издавая системный звук.
///    Перехватывая эти события и возвращая `Propagation::Stop`, мы предотвращаем вызов стандартного обработчика.
/// 2. **Отступы строк (Tab / Shift+Tab)**:
///    - Если выделен блок текста: добавляет/удаляет по 2 пробела в начале каждой строки блока.
///    - Если курсор без выделения: вставляет 2 пробела или удаляет отступ строки без потери фокуса.
///    - Действия оборачиваются в `buffer.begin_user_action()`, обеспечивая атомарный Undo/Redo.
fn setup_editor_key_handling(text_view: &gtk4::TextView) {
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let text_view_clone = text_view.clone();

    key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
        // Пропускаем комбинации с клавишами Control или Alt (например, Ctrl+Tab для переключения вкладок)
        if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
            || state.contains(gtk4::gdk::ModifierType::ALT_MASK)
        {
            return glib::Propagation::Proceed;
        }

        // 1. Подавление системного звукового сигнала на границах документа
        let is_forward_boundary = keyval == gtk4::gdk::Key::Down
            || keyval == gtk4::gdk::Key::KP_Down
            || keyval == gtk4::gdk::Key::Page_Down
            || keyval == gtk4::gdk::Key::KP_Page_Down
            || keyval == gtk4::gdk::Key::Right
            || keyval == gtk4::gdk::Key::KP_Right;
        let is_backward_boundary = keyval == gtk4::gdk::Key::Up
            || keyval == gtk4::gdk::Key::KP_Up
            || keyval == gtk4::gdk::Key::Page_Up
            || keyval == gtk4::gdk::Key::KP_Page_Up
            || keyval == gtk4::gdk::Key::Left
            || keyval == gtk4::gdk::Key::KP_Left;

        if is_forward_boundary || is_backward_boundary {
            let buffer = text_view_clone.buffer();
            let insert_mark = buffer.get_insert();
            let iter = buffer.iter_at_mark(&insert_mark);

            // Если курсор в самом конце документа и нажимается клавиша движения вперед
            if is_forward_boundary && iter.is_end() {
                return glib::Propagation::Stop;
            }
            // Если курсор в самом начале документа и нажимается клавиша движения назад
            if is_backward_boundary && iter.is_start() {
                return glib::Propagation::Stop;
            }
        }

        let is_shift = state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
        let is_tab = keyval == gtk4::gdk::Key::Tab;
        let is_iso_left_tab = keyval == gtk4::gdk::Key::ISO_Left_Tab;

        if !is_tab && !is_iso_left_tab {
            return glib::Propagation::Proceed;
        }

        let buffer = text_view_clone.buffer();

        // 2. Shift + Tab (обратный отступ / unindent)
        if is_iso_left_tab || (is_tab && is_shift) {
            unindent_selection_or_cursor(&buffer);
            glib::Propagation::Stop
        } else if is_tab && !is_shift {
            // 3. Tab (прямой отступ / indent)
            indent_selection_or_cursor(&buffer);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });

    text_view.add_controller(key_controller);
}

/// Отображает модальное окно настроек (`adw::PreferencesWindow`)
fn show_preferences_dialog(
    parent: &adw::ApplicationWindow,
    state: &AppState,
    text_view: &gtk4::TextView,
    line_numbers_gutter: &gtk4::DrawingArea,
    editor_buffer: &gtk4::TextBuffer,
    editor_font_tag: &gtk4::TextTag,
    tag_table: &gtk4::TextTagTable,
    editor_theme_provider: &gtk4::CssProvider,
    refresh_preview: Rc<dyn Fn()>,
    update_ui_language: Rc<dyn Fn(Language)>,
) {
    let current_lang = Language::from_code(&state.settings.borrow().language);

    let window = adw::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .title(current_lang.pref_window_title())
        .default_width(480)
        .build();

    let page = adw::PreferencesPage::new();

    // Секция 1: Язык интерфейса
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

    let initial_lang_idx = match current_lang {
        Language::Russian => 0,
        Language::English => 1,
    };
    lang_row.set_selected(initial_lang_idx);

    let state_lang = state.clone();
    let pref_win_clone = window.clone();
    let ui_updater = update_ui_language.clone();

    lang_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if let Some(&(_, selected_lang)) = languages.get(idx) {
            state_lang.settings.borrow_mut().language = selected_lang.as_code().to_string();
            state_lang.settings.borrow().save();

            pref_win_clone.set_title(Some(selected_lang.pref_window_title()));
            ui_updater(selected_lang);
        }
    });

    language_group.add(&lang_row);
    page.add(&language_group);

    // Секция 2: Цветовая схема
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
    let gutter_for_theme = line_numbers_gutter.clone();

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
            gutter_for_theme.queue_draw();

            refresh_for_theme();
        }
    });

    appearance_group.add(&theme_row);
    page.add(&appearance_group);

    // Секция 3: Разметка и отступы предпросмотра
    let layout_group = adw::PreferencesGroup::builder()
        .title(current_lang.pref_layout_group_title())
        .description(current_lang.pref_layout_group_desc())
        .build();

    let padding_spin = adw::SpinRow::builder()
        .title(current_lang.pref_padding_row_title())
        .subtitle(current_lang.pref_padding_row_subtitle())
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

    // Секция 4: Шрифты редактора и предпросмотра
    let fonts_group = adw::PreferencesGroup::builder()
        .title(current_lang.pref_fonts_group_title())
        .description(current_lang.pref_fonts_group_desc())
        .build();

    // Получаем ВСЕ моноширинные шрифты, установленные в системе
    let font_options = get_system_monospace_fonts();
    let font_options_refs: Vec<&str> = font_options.iter().map(|s| s.as_str()).collect();

    let font_family_row = adw::ComboRow::builder()
        .title(current_lang.pref_font_family_title())
        .subtitle(current_lang.pref_font_family_subtitle())
        .model(&gtk4::StringList::new(&font_options_refs))
        .build();

    let saved_font_family = state.settings.borrow().editor_font_family.clone();
    let initial_font_idx = font_options
        .iter()
        .position(|f| f.eq_ignore_ascii_case(&saved_font_family))
        .unwrap_or(0);
    font_family_row.set_selected(initial_font_idx as u32);

    let state_font_family = state.clone();
    let buffer_for_font_family = editor_buffer.clone();
    let tag_for_font_family = editor_font_tag.clone();
    let font_options_clone = font_options.clone();
    let gutter_for_font_family = line_numbers_gutter.clone();

    font_family_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if let Some(selected_font) = font_options_clone.get(idx) {
            state_font_family.settings.borrow_mut().editor_font_family = selected_font.clone();
            state_font_family.settings.borrow().save();
            let size = state_font_family.settings.borrow().editor_font_size;
            apply_editor_font(&buffer_for_font_family, &tag_for_font_family, selected_font, size);
            gutter_for_font_family.queue_draw();
        }
    });
    fonts_group.add(&font_family_row);

    let editor_spin = adw::SpinRow::builder()
        .title(current_lang.pref_editor_size_title())
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
    let gutter_for_editor_size = line_numbers_gutter.clone();
    editor_spin.connect_changed(move |row| {
        let size = row.value() as u32;
        state_editor_font.settings.borrow_mut().editor_font_size = size;
        state_editor_font.settings.borrow().save();
        let family = state_editor_font.settings.borrow().editor_font_family.clone();
        apply_editor_font(&buffer_for_spin, &tag_for_spin, &family, size);
        gutter_for_editor_size.queue_draw();
    });
    fonts_group.add(&editor_spin);

    let preview_spin = adw::SpinRow::builder()
        .title(current_lang.pref_preview_size_title())
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

    // Переключатель отображения номеров строк в режиме редактирования
    let line_numbers_row = adw::SwitchRow::builder()
        .title(current_lang.pref_line_numbers_title())
        .subtitle(current_lang.pref_line_numbers_subtitle())
        .active(state.settings.borrow().show_line_numbers)
        .build();

    let state_line_numbers = state.clone();
    let text_view_for_toggle = text_view.clone();
    let gutter_for_toggle = line_numbers_gutter.clone();

    line_numbers_row.connect_active_notify(move |row| {
        let is_active = row.is_active();
        state_line_numbers.settings.borrow_mut().show_line_numbers = is_active;
        state_line_numbers.settings.borrow().save();

        // Динамически подключаем или отключаем DrawingArea gutter
        if is_active {
            text_view_for_toggle.set_gutter(gtk4::TextWindowType::Left, Some(&gutter_for_toggle));
            gutter_for_toggle.queue_draw();
        } else {
            text_view_for_toggle.set_gutter(gtk4::TextWindowType::Left, None::<&gtk4::DrawingArea>);
        }
    });
    fonts_group.add(&line_numbers_row);

    page.add(&fonts_group);
    window.add(&page);
    window.present();
}

/// Отображает стандартное окно «О программе» (`adw::AboutWindow`).
fn show_about_dialog(parent: &adw::ApplicationWindow, lang: Language) {
    let about = adw::AboutWindow::builder()
        .transient_for(parent)
        .modal(true)
        .application_name("rmd-view")
        .version("0.1.0")
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
    fn test_indent_and_unindent() {
        if gtk4::init().is_err() {
            return;
        }

        // 1. Тест многострочного отступа и обратного отступа
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("line 1\nline 2\nline 3\n");

        let start = buffer.iter_at_line(0).unwrap();
        let end = buffer.iter_at_line_offset(1, 3).unwrap();
        buffer.select_range(&start, &end);

        indent_selection_or_cursor(&buffer);
        let full_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(full_text.as_str(), "  line 1\n  line 2\nline 3\n");

        unindent_selection_or_cursor(&buffer);
        let text_after_unindent = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text_after_unindent.as_str(), "line 1\nline 2\nline 3\n");

        // 2. Тест удаления одиночного пробела и отсутствия пробелов
        let buffer2 = gtk4::TextBuffer::new(None);
        buffer2.set_text(" line 1\nline 2\n");
        let start2 = buffer2.iter_at_line(0).unwrap();
        let end2 = buffer2.iter_at_line_offset(1, 4).unwrap();
        buffer2.select_range(&start2, &end2);

        unindent_selection_or_cursor(&buffer2);
        let full_text2 = buffer2.text(&buffer2.start_iter(), &buffer2.end_iter(), false);
        assert_eq!(full_text2.as_str(), "line 1\nline 2\n");

        // 3. Тест выделения, заканчивающегося ровно в начале следующей строки (starts_line())
        let buffer3 = gtk4::TextBuffer::new(None);
        buffer3.set_text("line 1\nline 2\nline 3\n");
        let start3 = buffer3.iter_at_line(0).unwrap();
        let end3 = buffer3.iter_at_line(2).unwrap();
        buffer3.select_range(&start3, &end3);

        indent_selection_or_cursor(&buffer3);
        let full_text3 = buffer3.text(&buffer3.start_iter(), &buffer3.end_iter(), false);
        assert_eq!(full_text3.as_str(), "  line 1\n  line 2\nline 3\n");

        // 4. Тест сохранения и восстановления позиции курсора для открытого файла
        let buffer_cur = gtk4::TextBuffer::new(None);
        buffer_cur.set_text("First line\nSecond line\nThird line\n");

        let test_path = std::path::PathBuf::from("/test/sample_doc.md");
        let app_state = AppState {
            current_file: Rc::new(RefCell::new(Some(test_path.clone()))),
            is_modified: Rc::new(RefCell::new(false)),
            settings: Rc::new(RefCell::new(AppSettings::default())),
            is_preview_active: Rc::new(RefCell::new(false)),
            target_line: Rc::new(RefCell::new(None)),
        };

        // Устанавливаем курсор на строку 1, символ 5 ("d" в "Second")
        let target_iter = buffer_cur.iter_at_line_offset(1, 5).unwrap();
        buffer_cur.place_cursor(&target_iter);

        // Сохраняем позицию курсора текущего файла
        save_current_file_cursor_position(&app_state, &buffer_cur);

        // Проверяем, что в словаре сохраненных позиций настроек появилась запись (1, 5)
        let key = get_file_canonical_key(&test_path);
        assert_eq!(
            app_state.settings.borrow().file_cursor_positions.get(&key),
            Some(&(1, 5))
        );

        // Сбрасываем курсор в начало документа
        buffer_cur.place_cursor(&buffer_cur.start_iter());
        let reset_iter = buffer_cur.iter_at_mark(&buffer_cur.get_insert());
        assert_eq!(reset_iter.line(), 0);
        assert_eq!(reset_iter.line_offset(), 0);

        // Восстанавливаем позицию курсора из настроек
        apply_restored_cursor_position(&app_state, &buffer_cur, &test_path);
        let restored_iter = buffer_cur.iter_at_mark(&buffer_cur.get_insert());
        assert_eq!(restored_iter.line(), 1);
        assert_eq!(restored_iter.line_offset(), 5);
    }
}
