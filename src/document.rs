//! Одна вкладка документа: редактор Markdown и его предпросмотр.
//!
//! Модуль вынесен из `window.rs`, чтобы окно занималось только вкладками,
//! меню и действиями приложения, а каждый открытый документ владел своим
//! независимым состоянием:
//! - содержимым `GtkTextBuffer`;
//! - флагом несохраненных изменений;
//! - активным режимом (редактор / предпросмотр);
//! - позициями прокрутки редактора и предпросмотра.
//!
//! Такое разделение особенно важно для вкладок: два документа одновременно
//! имеют собственные буферы, WebKit-представления и позиции курсора.

use adw::prelude::*;
use gtk4::gio;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use webkit6::prelude::*;
use webkit6::{
    UserContentInjectedFrames, UserContentManager, UserScript, UserScriptInjectionTime,
};

use crate::highlighter;
use crate::i18n::Language;
use crate::renderer;
use crate::settings::{AppSettings, FileViewState};

/// Имя обработчика JavaScript-сообщений, через который WebKit сообщает
/// актуальную позицию прокрутки предпросмотра.
const SCROLL_MESSAGE_HANDLER: &str = "rmdPreviewScroll";

/// JavaScript-скрипт, внедряемый в каждую страницу предпросмотра.
///
/// Он слушает событие `scroll` и отправляет `window.scrollY` в нативный код.
/// Благодаря этому позиция предпросмотра запоминается непрерывно, без
/// синхронного опроса WebKit при переключении режимов.
const SCROLL_TRACKER_SCRIPT: &str = r#"
(function () {
    if (window.__rmdScrollTrackerInstalled) {
        return;
    }
    window.__rmdScrollTrackerInstalled = true;

    var scheduled = false;

    function report() {
        scheduled = false;
        try {
            var y = window.scrollY
                || document.documentElement.scrollTop
                || document.body.scrollTop
                || 0;
            if (window.webkit && window.webkit.messageHandlers
                && window.webkit.messageHandlers.rmdPreviewScroll) {
                window.webkit.messageHandlers.rmdPreviewScroll.postMessage(y);
            }
        } catch (e) {
            // Предпросмотр может быть уже выгружен при закрытии вкладки.
        }
    }

    window.addEventListener('scroll', function () {
        if (!scheduled) {
            scheduled = true;
            requestAnimationFrame(report);
        }
    }, { passive: true });
})();
"#;

/// Состояние и виджеты одного открытого документа.
///
/// Поля намеренно открыты для чтения из `window.rs`: окно управляет заголовком,
/// кнопкой сохранения и вкладками, но не дублирует содержимое документа.
pub struct Document {
    /// Корневой виджет вкладки (панель поиска + ViewStack).
    pub root: gtk4::Box,
    /// Страница вкладки в `adw::TabView`. Заполняется окном после добавления.
    pub tab_page: RefCell<Option<adw::TabPage>>,
    /// Путь к файлу на диске (`None` для безымянного документа).
    pub current_file: RefCell<Option<PathBuf>>,
    /// Есть ли несохраненные изменения.
    pub is_modified: Cell<bool>,
    /// Открыт ли сейчас режим предпросмотра.
    pub is_preview_active: Cell<bool>,

    /// Последняя известная позиция прокрутки редактора.
    pub editor_scroll_position: Cell<f64>,
    /// Последняя известная позиция прокрутки предпросмотра.
    pub preview_scroll_position: Rc<Cell<f64>>,
    /// Позиция, которую нужно восстановить после следующей загрузки HTML.
    pub pending_preview_scroll: Rc<Cell<Option<f64>>>,

    /// Общие настройки приложения (тема, шрифты, язык, сессия).
    pub settings: Rc<RefCell<AppSettings>>,

    /// Стек «Редактор / Просмотр» конкретного документа.
    pub view_stack: adw::ViewStack,
    /// Панель поиска, принадлежащая документу.
    pub search_bar: gtk4::SearchBar,
    /// Поле ввода поиска.
    pub search_entry: gtk4::SearchEntry,
    /// Текстовый буфер редактора.
    pub buffer: gtk4::TextBuffer,
    /// Виджет редактора.
    pub text_view: gtk4::TextView,
    /// Прокручиваемая область редактора.
    pub editor_scroll: gtk4::ScrolledWindow,
    /// Колонка с номерами строк.
    pub gutter: gtk4::DrawingArea,
    /// Таблица тегов (нужна для смены цветов подсветки).
    pub tag_table: gtk4::TextTagTable,

    /// WebKit-представление предпросмотра.
    pub web_view: webkit6::WebView,
    /// Прокручиваемая область предпросмотра.
    pub preview_scroll: gtk4::ScrolledWindow,

    /// Была ли для файла сохранена позиция просмотра (влияет на выбор
    /// между восстановлением прокрутки и прокруткой к курсору).
    had_saved_view_state: Cell<bool>,
}

impl Document {
    /// Создает вкладку документа.
    ///
    /// `restore_session_mode` имеет значение `true` только при восстановлении
    /// прошлой сессии: в этом случае документ может открыться сразу в режиме
    /// предпросмотра. При обычном открытии файла всегда показывается редактор,
    /// но сохраненная позиция прокрутки редактора всё равно восстанавливается.
    pub fn new(
        settings: Rc<RefCell<AppSettings>>,
        initial_path: Option<PathBuf>,
        restore_session_mode: bool,
    ) -> Rc<Self> {
        let lang = Language::from_code(&settings.borrow().language);

        // --- Общие CSS-классы и тема редактора ---
        // Шрифт редактора задается CSS-правилом в `window.rs`
        // (`textview.editor-view text { font-family; font-size; }`), а не
        // текстовым тегом на диапазоне. Это принципиально: тег применялся
        // только к уже существующим символам, поэтому новый текст в пустом
        // документе получал системный шрифт по умолчанию.
        let tag_table = gtk4::TextTagTable::new();
        let style_mgr = adw::StyleManager::default();
        highlighter::register_highlight_tags(
            &tag_table,
            &settings.borrow().theme,
            style_mgr.is_dark(),
        );

        let buffer = gtk4::TextBuffer::builder().tag_table(&tag_table).build();
        let text_view = gtk4::TextView::builder()
            .buffer(&buffer)
            .wrap_mode(gtk4::WrapMode::Word)
            .left_margin(18)
            .right_margin(18)
            .top_margin(14)
            .bottom_margin(14)
            .build();
        text_view.add_css_class("editor-view");

        let gutter = setup_line_numbers_gutter(&text_view, &settings);
        let editor_scroll = gtk4::ScrolledWindow::builder()
            .child(&text_view)
            .hexpand(true)
            .vexpand(true)
            .build();

        // Перерисовываем номера строк при любой прокрутке редактора.
        let gutter_for_scroll = gutter.clone();
        editor_scroll.vadjustment().connect_value_changed(move |_| {
            gutter_for_scroll.queue_draw();
        });

        // --- Панель поиска ---
        let search_bar = gtk4::SearchBar::new();
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_hexpand(true);
        search_entry.set_placeholder_text(Some(lang.search_placeholder()));
        let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        search_box.append(&search_entry);
        search_bar.set_child(Some(&search_box));
        search_bar.connect_entry(&search_entry);

        // --- Стек режимов документа ---
        let view_stack = adw::ViewStack::new();
        view_stack.add_titled(&editor_scroll, Some("editor"), lang.tab_editor());

        // --- Предпросмотр WebKit ---
        let user_content_manager = UserContentManager::new();
        user_content_manager.register_script_message_handler(SCROLL_MESSAGE_HANDLER, None);
        user_content_manager.add_script(&UserScript::new(
            SCROLL_TRACKER_SCRIPT,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::End,
            &[],
            &[],
        ));

        let web_view = webkit6::WebView::builder()
            .user_content_manager(&user_content_manager)
            .build();

        if let Some(web_settings) = WebViewExt::settings(&web_view) {
            web_settings.set_allow_file_access_from_file_urls(true);
            web_settings.set_allow_universal_access_from_file_urls(true);
        }

        let preview_scroll = gtk4::ScrolledWindow::builder()
            .child(&web_view)
            .hexpand(true)
            .vexpand(true)
            .build();

        view_stack.add_titled(&preview_scroll, Some("preview"), lang.tab_preview());

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.append(&search_bar);
        root.append(&view_stack);

        // При восстановлении сессии документ мог быть оставлен в режиме
        // предпросмотра. При обычном открытии файла всегда показываем редактор.
        let initial_preview_active = if restore_session_mode {
            initial_path
                .as_ref()
                .and_then(|path| {
                    settings
                        .borrow()
                        .file_view_states
                        .get(&get_file_canonical_key(path))
                        .map(|state| state.preview_active)
                })
                .unwrap_or(false)
        } else {
            false
        };

        let preview_scroll_position = Rc::new(Cell::new(0.0));
        let pending_preview_scroll = Rc::new(Cell::new(None));

        let doc = Rc::new(Self {
            root,
            tab_page: RefCell::new(None),
            current_file: RefCell::new(None),
            is_modified: Cell::new(false),
            is_preview_active: Cell::new(initial_preview_active),
            editor_scroll_position: Cell::new(0.0),
            preview_scroll_position: preview_scroll_position.clone(),
            pending_preview_scroll: pending_preview_scroll.clone(),
            settings: settings.clone(),
            view_stack,
            search_bar,
            search_entry,
            buffer,
            text_view,
            editor_scroll,
            gutter,
            tag_table,
            web_view,
            preview_scroll,
            had_saved_view_state: Cell::new(false),
        });

        doc.setup_handlers(
            user_content_manager,
            preview_scroll_position,
            pending_preview_scroll,
        );

        // Загружаем начальный файл, если он передан. Позиции курсора и
        // прокрутки берутся из настроек по каноническому пути.
        if let Some(path) = initial_path {
            doc.load_file(&path);
        } else {
            doc.buffer.place_cursor(&doc.buffer.start_iter());
        }

        // Восстанавливаем режим просмотра только при восстановлении сессии.
        if initial_preview_active {
            doc.view_stack.set_visible_child_name("preview");
            doc.refresh_preview();
        }

        doc
    }

    /// Подключает все внутренние обработчики документа.
    fn setup_handlers(
        self: &Rc<Self>,
        user_content_manager: UserContentManager,
        preview_scroll_position: Rc<Cell<f64>>,
        pending_preview_scroll: Rc<Cell<Option<f64>>>,
    ) {
        // 1. Отслеживаем изменения текста: помечаем документ измененным,
        //    обновляем заголовок вкладки, номера строк и подсветку Markdown.
        let weak_doc = Rc::downgrade(self);
        self.buffer.connect_changed(move |buf| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.is_modified.set(true);
                doc.gutter.queue_draw();
                highlighter::highlight_markdown_buffer(buf);
                doc.update_tab_title();
            }
        });

        // 2. Запоминаем позицию прокрутки редактора при каждом изменении.
        let editor_scroll_cell = self.editor_scroll_position.clone();
        self.editor_scroll
            .vadjustment()
            .connect_value_changed(move |adj| {
                editor_scroll_cell.set(adj.value());
            });

        // 3. WebKit присылает scrollY через message handler. Сохраняем значение
        //    в ячейку документа, чтобы при возврате в предпросмотр восстановить его.
        let preview_cell_for_message = preview_scroll_position.clone();
        user_content_manager.connect_script_message_received(
            Some(SCROLL_MESSAGE_HANDLER),
            move |_, value| {
                if value.is_number() {
                    preview_cell_for_message.set(value.to_double().max(0.0));
                }
            },
        );

        // 4. После загрузки HTML восстанавливаем позицию прокрутки предпросмотра.
        //    `pending_preview_scroll` защищает от гонки со скриптом отслеживания,
        //    который может успеть сообщить 0 до программной прокрутки.
        let web_view_for_load = self.web_view.clone();
        let pending_for_load = pending_preview_scroll.clone();
        self.web_view.connect_load_changed(move |_view, event| {
            if event == webkit6::LoadEvent::Finished {
                if let Some(y) = pending_for_load.take() {
                    let script = format!(
                        "(function() {{ \
                            var y = {:.2}; \
                            function restore() {{ window.scrollTo(0, y); }} \
                            restore(); \
                            requestAnimationFrame(restore); \
                            setTimeout(restore, 60); \
                        }})();",
                        y.max(0.0)
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

        // 5. Клавиатура: Tab/Shift+Tab для отступов и подавление звука
        //    на границах документа.
        setup_editor_key_handling(&self.text_view);

        // 6. Поиск по буферу текущего документа.
        self.setup_search();
    }

    /// Загружает файл в вкладку, восстанавливая курсор и прокрутку.
    pub fn load_file(&self, path: &Path) {
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };

        let key = get_file_canonical_key(path);
        let view_state = self
            .settings
            .borrow()
            .file_view_states
            .get(&key)
            .copied();

        self.buffer.set_text(&content);
        highlighter::highlight_markdown_buffer(&self.buffer);
        *self.current_file.borrow_mut() = Some(path.to_path_buf());
        self.is_modified.set(false);

        // Восстанавливаем сохраненные позиции прокрутки. Предпросмотр
        // перерисуется лениво — при первом переключении в этот режим.
        if let Some(state) = view_state {
            self.editor_scroll_position.set(state.editor_scroll);
            self.preview_scroll_position.set(state.preview_scroll);
            self.had_saved_view_state.set(true);

        } else {
            self.editor_scroll_position.set(0.0);
            self.preview_scroll_position.set(0.0);
            self.had_saved_view_state.set(false);
        }

        apply_restored_cursor_position(&self.settings, &self.buffer, path);
        self.update_tab_title();

        // Если для файла уже была сохранена прокрутка, возвращаемся именно
        // в нее. Иначе прокручиваем к восстановленному курсору.
        if self.had_saved_view_state.get() {
            self.schedule_editor_scroll();
        } else {
            schedule_cursor_scroll(&self.text_view);
        }
    }

    /// Устанавливает режим документа. Используется окном при переключении
    /// заголовочных кнопок и вкладок.
    pub fn set_preview_mode(self: &Rc<Self>, preview: bool) {
        if preview {
            self.switch_to_preview();
        } else {
            self.switch_to_editor();
        }
    }

    /// Переключает документ в режим предпросмотра.
    pub fn switch_to_preview(self: &Rc<Self>) {
        if self.is_preview_active.get() {
            return;
        }

        // Сохраняем актуальную прокрутку редактора (она и так обновляется
        // обработчиком GtkAdjustment) и запоминаем, что режим изменился.
        self.is_preview_active.set(true);
        self.refresh_preview();
        self.view_stack.set_visible_child_name("preview");
    }

    /// Переключает документ в режим редактора.
    pub fn switch_to_editor(self: &Rc<Self>) {
        if !self.is_preview_active.get() {
            return;
        }

        self.is_preview_active.set(false);
        self.view_stack.set_visible_child_name("editor");

        // Возвращаем ровно ту позицию прокрутки, которую пользователь
        // оставил в редакторе. Никаких вычислений по DOM не требуется.
        self.schedule_editor_scroll();
    }

    /// Перерисовывает HTML-предпросмотр из текущего содержимого буфера.
    pub fn refresh_preview(&self) {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        let markdown_text = self.buffer.text(&start, &end, false);

        let style_manager = adw::StyleManager::default();
        let is_dark_system = style_manager.is_dark();

        let base_uri = self.current_file.borrow().as_ref().and_then(|p| {
            p.parent().and_then(|parent_dir| {
                parent_dir.to_str().map(|s| {
                    if s.ends_with('/') {
                        format!("file://{s}")
                    } else {
                        format!("file://{s}/")
                    }
                })
            })
        });

        let settings = self.settings.borrow();
        let html = renderer::render_markdown_to_html(
            &markdown_text,
            &settings.theme,
            is_dark_system,
            settings.preview_font_size,
            settings.preview_padding_percent,
            base_uri.as_deref(),
        );
        drop(settings);

        // Запоминаем желаемую позицию до загрузки: загрузка начнется
        // асинхронно, и скрипт отслеживания не должен успеть обнулить ее.
        self.pending_preview_scroll
            .set(Some(self.preview_scroll_position.get()));
        self.web_view.load_html(&html, base_uri.as_deref());
    }

    /// Прокручивает редактор к сохраненной позиции после перекомпоновки виджетов.
    fn schedule_editor_scroll(&self) {
        let adjustment = self.editor_scroll.vadjustment();
        let target = self.editor_scroll_position.get();
        glib::idle_add_local_once(move || {
            let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
            adjustment.set_value(target.clamp(0.0, max));
        });
    }

    /// Обновляет заголовок вкладки с учетом имени файла и несохраненных правок.
    pub fn update_tab_title(&self) {
        let lang = Language::from_code(&self.settings.borrow().language);
        let name = match self.current_file.borrow().as_ref() {
            Some(path) => path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("document.md")
                .to_string(),
            None => lang.untitled_document().to_string(),
        };

        let title = if self.is_modified.get() {
            format!("● {name}")
        } else {
            name
        };

        if let Some(page) = self.tab_page.borrow().as_ref() {
            page.set_title(&title);
            if let Some(path) = self.current_file.borrow().as_ref() {
                page.set_tooltip(&path.to_string_lossy());
            } else {
                page.set_tooltip(&title);
            }
        }
    }

    /// Возвращает заголовок документа для строки заголовка окна.
    pub fn window_title(&self, lang: Language) -> String {
        let name = match self.current_file.borrow().as_ref() {
            Some(path) => path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("document.md")
                .to_string(),
            None => lang.untitled_document().to_string(),
        };
        if self.is_modified.get() {
            format!("● {name}")
        } else {
            name
        }
    }

    /// Сохраняет документ по уже известному пути.
    pub fn save_existing(&self) -> bool {
        let Some(path) = self.current_file.borrow().clone() else {
            return false;
        };
        self.save_to(&path)
    }

    /// Сохраняет документ по указанному пути и делает его текущим файлом.
    pub fn save_to(&self, path: &Path) -> bool {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        let text = self.buffer.text(&start, &end, false);

        if std::fs::write(path, text.as_bytes()).is_err() {
            return false;
        }

        *self.current_file.borrow_mut() = Some(path.to_path_buf());
        self.is_modified.set(false);
        self.update_tab_title();
        true
    }

    /// Экспортирует позицию курсора и прокрутки в общие настройки.
    ///
    /// Вызывается при переключении вкладок, закрытии документа и завершении
    /// приложения, чтобы не записывать JSON на диск на каждый символ.
    pub fn export_state(&self, settings: &mut AppSettings) {
        let Some(path) = self.current_file.borrow().clone() else {
            return;
        };

        let key = get_file_canonical_key(&path);
        let insert_mark = self.buffer.get_insert();
        let iter = self.buffer.iter_at_mark(&insert_mark);
        let line = iter.line().max(0) as usize;
        let col = iter.line_offset().max(0) as usize;

        if settings.file_cursor_positions.len() > 500 {
            settings.file_cursor_positions.clear();
        }
        if settings.file_view_states.len() > 500 {
            settings.file_view_states.clear();
        }

        settings.file_cursor_positions.insert(key.clone(), (line, col));
        settings.file_view_states.insert(
            key,
            FileViewState {
                editor_scroll: self.editor_scroll_position.get(),
                preview_scroll: self.preview_scroll_position.get(),
                preview_active: self.is_preview_active.get(),
            },
        );
    }

    /// Возвращает `true`, если переданная страница вкладки принадлежит документу.
    pub fn matches_page(&self, page: &adw::TabPage) -> bool {
        self.tab_page
            .borrow()
            .as_ref()
            .map(|own| own == page)
            .unwrap_or(false)
    }

    /// Канонический ключ текущего файла (используется для сессии).
    pub fn canonical_key(&self) -> Option<String> {
        self.current_file
            .borrow()
            .as_ref()
            .map(|p| get_file_canonical_key(p))
    }

    /// Обновляет подписи документа при смене языка интерфейса.
    pub fn apply_language(&self, lang: Language) {
        self.view_stack
            .page(&self.editor_scroll)
            .set_title(Some(lang.tab_editor()));
        self.view_stack
            .page(&self.preview_scroll)
            .set_title(Some(lang.tab_preview()));
        self.search_entry
            .set_placeholder_text(Some(lang.search_placeholder()));
        self.update_tab_title();
    }

    /// Применяет новую цветовую тему к подсветке и номерам строк.
    pub fn apply_theme(&self, theme: &str, is_dark: bool) {
        highlighter::update_highlight_colors(&self.tag_table, theme, is_dark);
        highlighter::highlight_markdown_buffer(&self.buffer);
        self.gutter.queue_draw();
        if self.is_preview_active.get() {
            self.refresh_preview();
        }
    }

    /// Показывает или скрывает колонку номеров строк.
    pub fn set_line_numbers_visible(&self, visible: bool) {
        if visible {
            self.text_view
                .set_gutter(gtk4::TextWindowType::Left, Some(&self.gutter));
            self.gutter.queue_draw();
        } else {
            self.text_view
                .set_gutter(gtk4::TextWindowType::Left, None::<&gtk4::DrawingArea>);
        }
    }

    /// Подключает поиск по буферу документа.
    fn setup_search(&self) {
        let buffer = self.buffer.clone();
        let text_view = self.text_view.clone();
        let tag_table = self.tag_table.clone();

        let highlight_tag = gtk4::TextTag::builder()
            .name("search-match")
            .background("#f5a97f")
            .foreground("#181926")
            .build();
        tag_table.add(&highlight_tag);

        self.search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            buffer.remove_tag_by_name("search-match", &start, &end);

            if query.is_empty() {
                return;
            }

            let mut iter = buffer.start_iter();
            let mut first_match = None;

            while let Some((match_start, match_end)) = iter.forward_search(
                &query,
                gtk4::TextSearchFlags::CASE_INSENSITIVE,
                None,
            ) {
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
}

/// Настраивает колонку с номерами строк (Gutter).
///
/// Использует `GtkDrawingArea` и Cairo. Цвета и шрифт берутся из общих
/// настроек, поэтому при смене темы достаточно вызвать `queue_draw`.
fn setup_line_numbers_gutter(
    text_view: &gtk4::TextView,
    settings: &Rc<RefCell<AppSettings>>,
) -> gtk4::DrawingArea {
    let gutter = gtk4::DrawingArea::new();
    gutter.set_content_width(48);

    let text_view_clone = text_view.clone();
    let settings_for_draw = settings.clone();

    gutter.set_draw_func(move |area, cr, width, height| {
        let buffer = text_view_clone.buffer();
        let total_lines = buffer.line_count();

        // Ширина колонки зависит от количества цифр в номере строки.
        let digits = format!("{total_lines}").len().max(3);
        let needed_width = (digits as i32 * 9 + 20).max(40);
        if area.content_width() != needed_width {
            area.set_content_width(needed_width);
        }

        let settings = settings_for_draw.borrow();
        let theme = settings.theme.clone();
        let style_mgr = adw::StyleManager::default();
        let is_dark = style_mgr.is_dark();

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

        cr.set_source_rgb(bg_r, bg_g, bg_b);
        let _ = cr.paint();

        // Тонкая вертикальная линия между номерами и текстом.
        cr.set_source_rgba(num_r, num_g, num_b, 0.25);
        cr.set_line_width(1.0);
        cr.move_to((width as f64) - 0.5, 0.0);
        cr.line_to((width as f64) - 0.5, height as f64);
        let _ = cr.stroke();

        let (_, top_buf_y) =
            text_view_clone.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, 0);
        let (_, bottom_buf_y) =
            text_view_clone.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, height);

        let top_buf_y = top_buf_y.max(0);
        let (mut iter, _) = text_view_clone.line_at_y(top_buf_y);

        let font_size = (settings.editor_font_size as f64 * 0.85).max(9.0);
        let font_family = settings.editor_font_family.clone();
        cr.select_font_face(
            &font_family,
            gtk4::cairo::FontSlant::Normal,
            gtk4::cairo::FontWeight::Normal,
        );
        cr.set_font_size(font_size);
        cr.set_source_rgb(num_r, num_g, num_b);

        loop {
            let (y_buf, h_buf) = text_view_clone.line_yrange(&iter);
            let (_, win_y) =
                text_view_clone.buffer_to_window_coords(gtk4::TextWindowType::Left, 0, y_buf);

            let line_num = iter.line() + 1;
            let num_str = format!("{line_num}");

            if let Ok(extents) = cr.text_extents(&num_str) {
                let x = (width as f64) - 8.0 - extents.width() - extents.x_bearing();
                let y = (win_y as f64) + font_size + 2.0;
                cr.move_to(x, y);
                let _ = cr.show_text(&num_str);
            }

            if y_buf + h_buf > bottom_buf_y || !iter.forward_line() {
                break;
            }
        }
    });

    if settings.borrow().show_line_numbers {
        text_view.set_gutter(gtk4::TextWindowType::Left, Some(&gutter));
    } else {
        text_view.set_gutter(gtk4::TextWindowType::Left, None::<&gtk4::DrawingArea>);
    }

    gutter
}

/// Возвращает канонический путь для использования в качестве ключа настроек.
///
/// Символические ссылки и относительные пути приводятся к одному абсолютному
/// пути, чтобы файл, открытый из терминала и из диалога, имел одну запись.
pub fn get_file_canonical_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Восстанавливает позицию курсора для указанного файла.
fn apply_restored_cursor_position(
    settings: &Rc<RefCell<AppSettings>>,
    buffer: &gtk4::TextBuffer,
    path: &Path,
) {
    let key = get_file_canonical_key(path);
    let (line, col) = settings
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

/// Прокручивает редактор к курсору в следующем цикле главного потока.
///
/// На момент загрузки файла окно еще может не иметь финальной разметки,
/// поэтому прокрутка выполняется через `idle_add_local_once`.
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

/// Настраивает Tab/Shift+Tab и подавляет системный звук на границах текста.
fn setup_editor_key_handling(text_view: &gtk4::TextView) {
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let text_view_clone = text_view.clone();

    key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
        // Комбинации с Ctrl/Alt (например, Ctrl+Tab) не перехватываем.
        if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
            || state.contains(gtk4::gdk::ModifierType::ALT_MASK)
        {
            return glib::Propagation::Proceed;
        }

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

            if is_forward_boundary && iter.is_end() {
                return glib::Propagation::Stop;
            }
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

        if is_iso_left_tab || (is_tab && is_shift) {
            unindent_selection_or_cursor(&buffer);
            glib::Propagation::Stop
        } else if is_tab && !is_shift {
            indent_selection_or_cursor(&buffer);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });

    text_view.add_controller(key_controller);
}

/// Добавляет два пробела в начало каждой выделенной строки или в позицию курсора.
pub fn indent_selection_or_cursor(buffer: &gtk4::TextBuffer) {
    if let Some((start_iter, end_iter)) = buffer.selection_bounds() {
        if start_iter != end_iter {
            let start_line = start_iter.line();
            let mut end_line = end_iter.line();
            let original_end_line = end_iter.line();
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

    buffer.begin_user_action();
    buffer.insert_at_cursor("  ");
    buffer.end_user_action();
}

/// Удаляет до двух начальных пробелов (или один Tab) с выделенных строк / строки курсора.
pub fn unindent_selection_or_cursor(buffer: &gtk4::TextBuffer) {
    if let Some((start_iter, end_iter)) = buffer.selection_bounds() {
        if start_iter != end_iter {
            let start_line = start_iter.line();
            let mut end_line = end_iter.line();
            let original_end_line = end_iter.line();
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
                        if next.char() == ' ' {
                            chars_to_del += 1;
                        }
                    } else if c1 == '\t' {
                        chars_to_del = 1;
                    }

                    if chars_to_del > 0 {
                        let mut del_end = line_iter.clone();
                        del_end.forward_chars(chars_to_del);
                        buffer.delete(&mut line_iter, &mut del_end);

                        if line_idx == start_line {
                            start_col = (start_col - chars_to_del).max(0);
                        }
                        if line_idx == original_end_line {
                            end_col = (end_col - chars_to_del).max(0);
                        }
                    }
                }
            }

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
            if next.char() == ' ' {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indent_and_unindent() {
        if gtk4::init().is_err() {
            return;
        }

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
    }

    #[test]
    fn test_cursor_position_restore() {
        if gtk4::init().is_err() {
            return;
        }

        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("First line\nSecond line\nThird line\n");

        let test_path = std::path::PathBuf::from("/test/sample_doc.md");
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let key = get_file_canonical_key(&test_path);

        let target_iter = buffer.iter_at_line_offset(1, 5).unwrap();
        buffer.place_cursor(&target_iter);

        let insert_mark = buffer.get_insert();
        let iter = buffer.iter_at_mark(&insert_mark);
        settings
            .borrow_mut()
            .file_cursor_positions
            .insert(key, (iter.line() as usize, iter.line_offset() as usize));

        buffer.place_cursor(&buffer.start_iter());
        apply_restored_cursor_position(&settings, &buffer, &test_path);

        let restored_iter = buffer.iter_at_mark(&buffer.get_insert());
        assert_eq!(restored_iter.line(), 1);
        assert_eq!(restored_iter.line_offset(), 5);
    }
}
