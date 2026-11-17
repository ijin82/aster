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
const SCROLL_MESSAGE_HANDLER: &str = "asterPreviewScroll";

/// Максимум одновременно подсвеченных обычных совпадений в редакторе.
///
/// GtkTextBuffer применяет каждый TextTag отдельно, и на файле с тысячами
/// вхождений это занимает секунды. Текущее совпадение подсвечивается всегда,
/// а счётчик и навигация продолжают работать по полному списку.
const MAX_SEARCH_HIGHLIGHTS: usize = 200;

/// JavaScript-скрипт, внедряемый в каждую страницу предпросмотра.
///
/// Он слушает событие `scroll` и отправляет `window.scrollY` в нативный код.
/// Благодаря этому позиция предпросмотра запоминается непрерывно, без
/// синхронного опроса WebKit при переключении режимов.
const SCROLL_TRACKER_SCRIPT: &str = r#"
(function () {
    if (window.__asterScrollTrackerInstalled) {
        return;
    }
    window.__asterScrollTrackerInstalled = true;

    // Добавляем GitHub-подобные id заголовкам, чтобы ссылки вида
    // [текст](#заголовок) работали и внутри Markdown-предпросмотра.
    // Скрипт выполняется как WebKit user script, поэтому CSP его не блокирует.
    function installHeadingAnchors() {
        var used = {};
        var headings = document.querySelectorAll('h1,h2,h3,h4,h5,h6');

        for (var i = 0; i < headings.length; i++) {
            var heading = headings[i];
            if (heading.id) {
                used[heading.id] = true;
                continue;
            }

            var base = (heading.textContent || '')
                .trim()
                .toLowerCase()
                .replace(/[^\p{L}\p{N}\s-]/gu, '')
                .replace(/\s+/g, '-');

            if (!base) {
                continue;
            }

            var id = base;
            var suffix = 2;
            while (used[id] || document.getElementById(id)) {
                id = base + '-' + suffix;
                suffix++;
            }

            used[id] = true;
            heading.id = id;
        }
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', installHeadingAnchors);
    } else {
        installHeadingAnchors();
    }

    // Поиск по видимому тексту предпросмотра. Mark-элементы создаются
    // только в DOM, исходный Markdown и GtkTextBuffer не меняются.
    window.__asterSearch = (function () {
        var marks = [];
        var current = -1;

        function result() {
            return JSON.stringify({ count: marks.length, index: current });
        }

        function updateCurrent(scrollToCurrent) {
            for (var i = 0; i < marks.length; i++) {
                marks[i].className = 'aster-search-match';
            }
            if (current >= 0 && marks[current]) {
                marks[current].className = 'aster-search-current';
                if (scrollToCurrent) {
                    marks[current].scrollIntoView({ block: 'center', inline: 'nearest' });
                }
            }
        }

        function clear() {
            var found = document.querySelectorAll(
                'mark.aster-search-match, mark.aster-search-current'
            );
            var parents = [];

            for (var i = found.length - 1; i >= 0; i--) {
                var mark = found[i];
                var parent = mark.parentNode;
                if (!parent) {
                    continue;
                }
                while (mark.firstChild) {
                    parent.insertBefore(mark.firstChild, mark);
                }
                parent.removeChild(mark);
                if (parents.indexOf(parent) === -1) {
                    parents.push(parent);
                }
            }

            for (var j = 0; j < parents.length; j++) {
                parents[j].normalize();
            }

            marks = [];
            current = -1;
        }

        function find(query) {
            clear();
            if (!query) {
                return result();
            }

            var lowerQuery = query.toLowerCase();
            var nodes = [];
            var walker = document.createTreeWalker(
                document.body,
                NodeFilter.SHOW_TEXT,
                {
                    acceptNode: function (node) {
                        var parent = node.parentElement;
                        if (!parent) {
                            return NodeFilter.FILTER_REJECT;
                        }
                        if (parent.closest('script,style,noscript,textarea')) {
                            return NodeFilter.FILTER_REJECT;
                        }
                        var style = window.getComputedStyle(parent);
                        if (style.display === 'none' || style.visibility === 'hidden') {
                            return NodeFilter.FILTER_REJECT;
                        }
                        if (!node.nodeValue || !node.nodeValue.trim()) {
                            return NodeFilter.FILTER_REJECT;
                        }
                        return NodeFilter.FILTER_ACCEPT;
                    }
                }
            );

            while (walker.nextNode()) {
                nodes.push(walker.currentNode);
            }

            for (var n = 0; n < nodes.length; n++) {
                var node = nodes[n];
                var text = node.nodeValue || '';
                var lowerText = text.toLowerCase();
                var positions = [];
                var from = 0;

                while (true) {
                    var index = lowerText.indexOf(lowerQuery, from);
                    if (index === -1) {
                        break;
                    }
                    positions.push(index);
                    from = index + Math.max(1, lowerQuery.length);
                }

                if (!positions.length) {
                    continue;
                }

                var nodeMarks = new Array(positions.length);
                for (var i = positions.length - 1; i >= 0; i--) {
                    var range = document.createRange();
                    range.setStart(node, positions[i]);
                    range.setEnd(node, positions[i] + query.length);
                    var mark = document.createElement('mark');
                    mark.className = 'aster-search-match';
                    mark.appendChild(range.extractContents());
                    range.insertNode(mark);
                    nodeMarks[i] = mark;
                }

                for (var k = 0; k < nodeMarks.length; k++) {
                    if (nodeMarks[k]) {
                        marks.push(nodeMarks[k]);
                    }
                }
            }

            current = marks.length ? 0 : -1;
            updateCurrent(true);
            return result();
        }

        function next(forward) {
            if (!marks.length) {
                return result();
            }
            current = (current + (forward ? 1 : -1) + marks.length) % marks.length;
            updateCurrent(true);
            return result();
        }

        return {
            find: find,
            next: next,
            clear: clear
        };
    })();

    var scheduled = false;

    function report() {
        scheduled = false;
        try {
            var y = window.scrollY
                || document.documentElement.scrollTop
                || document.body.scrollTop
                || 0;
            if (window.webkit && window.webkit.messageHandlers
                && window.webkit.messageHandlers.asterPreviewScroll) {
                window.webkit.messageHandlers.asterPreviewScroll.postMessage(y);
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
    /// Пока true, изменения GtkAdjustment игнорируются: идет восстановление
    /// позиции после показа редактора, и промежуточные сбросы в 0 не должны
    /// попасть ни в память документа, ни в сохраняемую сессию.
    suppress_editor_scroll_updates: Rc<Cell<bool>>,
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
    /// Кнопка перехода к предыдущему совпадению.
    pub search_prev_btn: gtk4::Button,
    /// Кнопка перехода к следующему совпадению.
    pub search_next_btn: gtk4::Button,
    /// Счетчик вида «3 из 17» или «Нет совпадений».
    pub search_status: gtk4::Label,
    /// Поле «Заменить на…».
    pub replace_entry: gtk4::Entry,
    /// Переключатель показа строки замены.
    pub replace_toggle: gtk4::ToggleButton,
    /// Кнопка замены текущего совпадения.
    pub replace_btn: gtk4::Button,
    /// Кнопка замены всех совпадений.
    pub replace_all_btn: gtk4::Button,
    /// Строка замены, скрываемая, пока замена не включена.
    replace_row: gtk4::Box,
    /// Пока true, изменение буфера не должно само пересчитывать поиск:
    /// это делает replace_current/replace_all после завершения правки.
    suppress_search_refresh: Cell<bool>,
    /// Позиции найденных совпадений в виде (начало, конец) по offset буфера.
    search_matches: RefCell<Vec<(i32, i32)>>,
    /// Индекс текущего совпадения в `search_matches`.
    search_index: Cell<usize>,
    /// Количество совпадений в предпросмотре (приходит из JavaScript).
    preview_search_count: Cell<usize>,
    /// Индекс текущего совпадения в предпросмотре.
    preview_search_index: Cell<usize>,
    /// Номер поколения preview-поиска: защищает от запоздавших JS-ответов.
    preview_search_generation: Cell<u64>,
    /// Режим массовой правки: тяжёлая подсветка и поиск откладываются
    /// до окончания серии изменений буфера.
    bulk_editing: Cell<bool>,
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
    ) -> Result<Rc<Self>, String> {
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

        let buffer = gtk4::TextBuffer::builder()
            .tag_table(&tag_table)
            .enable_undo(true)
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

        let search_prev_btn = gtk4::Button::from_icon_name("go-up-symbolic");
        search_prev_btn.add_css_class("flat");
        search_prev_btn.set_tooltip_text(Some(lang.tooltip_search_previous()));
        search_prev_btn.set_sensitive(false);

        let search_next_btn = gtk4::Button::from_icon_name("go-down-symbolic");
        search_next_btn.add_css_class("flat");
        search_next_btn.set_tooltip_text(Some(lang.tooltip_search_next()));
        search_next_btn.set_sensitive(false);

        let search_status = gtk4::Label::new(None);
        search_status.add_css_class("dim-label");
        search_status.add_css_class("numeric");
        search_status.set_width_chars(9);
        search_status.set_xalign(0.5);

        let replace_toggle = gtk4::ToggleButton::builder()
            .icon_name("edit-find-replace-symbolic")
            .tooltip_text(lang.tooltip_replace())
            .build();
        replace_toggle.add_css_class("flat");
        replace_toggle.set_active(false);

        let replace_entry = gtk4::Entry::new();
        replace_entry.set_hexpand(true);
        replace_entry.set_placeholder_text(Some(lang.replace_placeholder()));
        replace_entry.set_visible(false);

        let replace_btn = gtk4::Button::with_label(lang.btn_replace());
        replace_btn.add_css_class("flat");
        replace_btn.set_visible(false);
        replace_btn.set_sensitive(false);

        let replace_all_btn = gtk4::Button::with_label(lang.btn_replace_all());
        replace_all_btn.add_css_class("flat");
        replace_all_btn.set_visible(false);
        replace_all_btn.set_sensitive(false);

        let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        search_row.append(&search_entry);
        search_row.append(&search_status);
        search_row.append(&search_prev_btn);
        search_row.append(&search_next_btn);
        search_row.append(&replace_toggle);

        let replace_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        replace_row.append(&replace_entry);
        replace_row.append(&replace_btn);
        replace_row.append(&replace_all_btn);
        replace_row.set_visible(false);

        let search_rows = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        search_rows.append(&search_row);
        search_rows.append(&replace_row);
        search_bar.set_child(Some(&search_rows));
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
            // Файловому документу не нужен универсальный доступ: он должен
            // читать только собственные относительные ресурсы (картинки),
            // а не произвольные file:// URL.
            web_settings.set_allow_universal_access_from_file_urls(false);

            // Markdown-предпросмотр не должен издавать звуки. Отключаем
            // воспроизведение медиа, Web Audio, захват медиапотоков и EME:
            // этого достаточно, чтобы ни встроенный <audio>/<video>, ни
            // скрипт из HTML не могли включить звук.
            web_settings.set_enable_media(false);
            web_settings.set_enable_webaudio(false);
            web_settings.set_enable_media_stream(false);
            web_settings.set_enable_mediasource(false);
            web_settings.set_enable_encrypted_media(false);

            // Предпросмотр — не браузер: страница не должна открывать окна,
            // читать буфер обмена, показывать модальные диалоги и сохранять
            // данные в локальное хранилище.
            web_settings.set_javascript_can_open_windows_automatically(false);
            web_settings.set_javascript_can_access_clipboard(false);
            web_settings.set_allow_modal_dialogs(false);
            web_settings.set_enable_html5_local_storage(false);
            web_settings.set_enable_html5_database(false);
            web_settings.set_enable_javascript_markup(false);
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
            suppress_editor_scroll_updates: Rc::new(Cell::new(false)),
            preview_scroll_position: preview_scroll_position.clone(),
            pending_preview_scroll: pending_preview_scroll.clone(),
            settings: settings.clone(),
            view_stack,
            search_bar,
            search_entry,
            search_prev_btn,
            search_next_btn,
            search_status,
            replace_entry,
            replace_toggle,
            replace_btn,
            replace_all_btn,
            replace_row,
            suppress_search_refresh: Cell::new(false),
            search_matches: RefCell::new(Vec::new()),
            search_index: Cell::new(0),
            preview_search_count: Cell::new(0),
            preview_search_index: Cell::new(0),
            preview_search_generation: Cell::new(0),
            bulk_editing: Cell::new(false),
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
            doc.load_file(&path)?;
        } else {
            doc.buffer.place_cursor(&doc.buffer.start_iter());
        }

        // Восстанавливаем режим просмотра только при восстановлении сессии.
        if initial_preview_active {
            doc.view_stack.set_visible_child_name("preview");
            doc.refresh_preview();
        }
        // В предпросмотре замена не имеет смысла — оставляем только поиск.
        doc.replace_toggle.set_visible(!initial_preview_active);

        Ok(doc)
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

                // Во время массовой замены changed срабатывает на каждую
                // правку. Полная подсветка 1.7-МБ файла на каждой из тысяч
                // замен — главная причина зависания. Делаем это один раз
                // в end_bulk_edit().
                if doc.bulk_editing.get() {
                    return;
                }

                doc.gutter.queue_draw();
                highlighter::highlight_markdown_buffer(buf);
                if !doc.suppress_search_refresh.get() {
                    doc.refresh_search_matches(false);
                }
                doc.update_tab_title();
            }
        });

        // 2. Запоминаем позицию прокрутки редактора при каждом изменении.
        //
        // Важно: когда открыт предпросмотр, редактор скрыт и его
        // GtkAdjustment может сброситься в 0. Если записывать это значение,
        // сохраненная позиция редактора потеряется и после возврата документ
        // «отскочит» в начало. Поэтому обновляем ячейку только в режиме
        // редактора. Целевое значение при переключении захватывается заранее
        // в `switch_to_editor`.
        let weak_doc = Rc::downgrade(self);
        let suppress_editor_scroll_updates = self.suppress_editor_scroll_updates.clone();
        self.editor_scroll
            .vadjustment()
            .connect_value_changed(move |adj| {
                if suppress_editor_scroll_updates.get() {
                    return;
                }
                if let Some(doc) = weak_doc.upgrade() {
                    if !doc.is_preview_active.get() {
                        doc.editor_scroll_position.set(adj.value());
                    }
                }
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
        let pending_for_load = pending_preview_scroll.clone();
        let weak_doc_for_load = Rc::downgrade(self);
        self.web_view.connect_load_changed(move |view, event| {
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
                    view.evaluate_javascript(
                        &script,
                        None,
                        None,
                        Option::<&gio::Cancellable>::None,
                        |_| {},
                    );
                }

                // После перезагрузки HTML нужно заново наложить подсветку
                // поиска: старые mark-элементы исчезли вместе с DOM.
                if let Some(doc) = weak_doc_for_load.upgrade() {
                    if doc.is_preview_active.get()
                        && doc.search_bar.is_search_mode()
                        && !doc.search_entry.text().is_empty()
                    {
                        doc.refresh_preview_search(true);
                    }
                }
            }
        });

        // 5. Клавиатура: Tab/Shift+Tab для отступов и подавление звука
        //    на границах документа.
        setup_editor_key_handling(&self.text_view);

        // 6. Поиск по буферу текущего документа.
        self.setup_search();

        // 7. Политика ссылок и запрет навигации самого WebView.
        self.setup_link_policy();
    }

    /// Настраивает безопасное поведение ссылок в предпросмотре.
    ///
    /// Внутренние якоря прокручиваются внутри страницы, внешние URI
    /// открываются системным обработчиком, а сам WebView не уходит
    /// с текущего HTML-документа.
    fn setup_link_policy(self: &Rc<Self>) {
        let weak_doc = Rc::downgrade(self);

        self.web_view.connect_decide_policy(
            move |web_view, decision, decision_type| {
                let is_navigation = decision_type
                    == webkit6::PolicyDecisionType::NavigationAction
                    || decision_type == webkit6::PolicyDecisionType::NewWindowAction;

                if !is_navigation {
                    return false;
                }

                let Some(nav_decision) =
                    decision.downcast_ref::<webkit6::NavigationPolicyDecision>()
                else {
                    return false;
                };
                let Some(mut action) = nav_decision.navigation_action() else {
                    return false;
                };

                // Начальная загрузка нашего HTML и перезагрузка страницы
                // должны идти стандартным путем.
                let navigation_type = action.navigation_type();
                if navigation_type == webkit6::NavigationType::Other
                    || navigation_type == webkit6::NavigationType::Reload
                {
                    return false;
                }

                let uri = action
                    .request()
                    .and_then(|request| request.uri())
                    .unwrap_or_default()
                    .to_string();

                // Для ссылок #fragment WebKit с base href превращает URI
                // в file:///каталог/#fragment. Сравниваем базу с текущим
                // файлом, чтобы не отдавать такой якорь Nautilus.
                let current_file = weak_doc
                    .upgrade()
                    .and_then(|doc| doc.current_file.borrow().clone());

                match classify_link_uri(&uri, current_file.as_deref()) {
                    LinkTarget::Anchor(id) => {
                        let id_json = serde_json::to_string(&id)
                            .unwrap_or_else(|_| "\"\"".to_string());
                        let script = format!(
                            "(function() {{ \
                                var id = {id_json}; \
                                try {{ id = decodeURIComponent(id); }} catch (e) {{}} \
                                if (!id) {{ window.scrollTo(0, 0); return; }} \
                                var el = document.getElementById(id) \
                                    || document.getElementsByName(id)[0]; \
                                if (el) {{ el.scrollIntoView(true); }} \
                            }})();"
                        );
                        web_view.evaluate_javascript(
                            &script,
                            None,
                            None,
                            Option::<&gio::Cancellable>::None,
                            |_| {},
                        );
                        decision.ignore();
                        true
                    }
                    LinkTarget::External(uri) => {
                        // Асинхронный запуск не блокирует UI-поток, пока
                        // система запускает браузер или другой обработчик.
                        let uri_for_callback = uri.clone();
                        gio::AppInfo::launch_default_for_uri_async(
                            &uri,
                            Option::<&gio::AppLaunchContext>::None,
                            Option::<&gio::Cancellable>::None,
                            move |result| {
                                if let Err(error) = result {
                                    glib::g_warning!(
                                        "Aster",
                                        "Failed to open external link {}: {}",
                                        uri_for_callback,
                                        error
                                    );
                                }
                            },
                        );
                        decision.ignore();
                        true
                    }
                    LinkTarget::Blocked => {
                        decision.ignore();
                        true
                    }
                }
            },
        );
    }

    /// Загружает файл в вкладку, восстанавливая курсор и прокрутку.
    ///
    /// Возвращает текст ошибки с путем к файлу, чтобы окно могло показать
    /// пользователю внятное уведомление вместо пустой вкладки.
    pub fn load_file(&self, path: &Path) -> Result<(), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;

        let key = get_file_canonical_key(path);
        let view_state = self
            .settings
            .borrow()
            .file_view_states
            .get(&key)
            .copied();

        self.bulk_editing.set(true);
        self.buffer.set_text(&content);
        self.bulk_editing.set(false);
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
            schedule_cursor_scroll(&self.text_view, &self.editor_scroll);
        }

        Ok(())
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

        // Явно считываем текущую прокрутку, пока редактор еще видим.
        // Полагаться только на `value_changed` нельзя: при скрытии
        // ScrolledWindow может изменить adjustment, и это не должно
        // перезаписать пользовательскую позицию.
        self.editor_scroll_position
            .set(self.editor_scroll.vadjustment().value());

        // В предпросмотре доступен поиск, но не замена.
        self.replace_toggle.set_active(false);
        self.replace_row.set_visible(false);
        self.replace_toggle.set_visible(false);

        self.is_preview_active.set(true);
        self.refresh_preview();
        self.view_stack.set_visible_child_name("preview");
    }

    /// Переключает документ в режим редактора.
    pub fn switch_to_editor(self: &Rc<Self>) {
        if !self.is_preview_active.get() {
            return;
        }

        // Захватываем цель ДО смены видимого ребенка: показ редактора
        // может синхронно изменить GtkAdjustment сбросом в 0. Пока идет
        // восстановление, обработчик adjustment не должен обновлять ячейку.
        let target = self.editor_scroll_position.get();
        self.suppress_editor_scroll_updates.set(true);
        self.is_preview_active.set(false);
        self.view_stack.set_visible_child_name("editor");
        self.replace_toggle.set_visible(true);
        self.clear_preview_search();

        // Возвращаем ровно ту позицию прокрутки, которую пользователь
        // оставил в редакторе. Никаких вычислений по DOM не требуется.
        self.schedule_editor_scroll_to(target);

        // Если поиск был открыт в предпросмотре, применяем тот же запрос
        // к редактору.
        if self.search_bar.is_search_mode() {
            self.refresh_search_matches(true);
        } else {
            self.update_search_status();
        }
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
        self.schedule_editor_scroll_to(self.editor_scroll_position.get());
    }

    /// Прокручивает редактор к явно переданной позиции в следующем idle-цикле.
    ///
    /// Значение передается аргументом, а не читается из ячейки позже: за время
    /// до idle-цикла GTK может успеть пересчитать adjustment скрытого/только что
    /// показанного ScrolledWindow.
    fn schedule_editor_scroll_to(&self, target: f64) {
        let adjustment = self.editor_scroll.vadjustment();
        let suppress_editor_scroll_updates = self.suppress_editor_scroll_updates.clone();
        self.suppress_editor_scroll_updates.set(true);

        glib::idle_add_local_once(move || {
            let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
            adjustment.set_value(target.clamp(0.0, max));
            suppress_editor_scroll_updates.set(false);
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
    pub fn save_existing(&self) -> Result<(), String> {
        let path = self
            .current_file
            .borrow()
            .clone()
            .ok_or_else(|| "document has no file path".to_string())?;
        self.save_to(&path)
    }

    /// Сохраняет документ по указанному пути и делает его текущим файлом.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        let text = self.buffer.text(&start, &end, false);

        std::fs::write(path, text.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))?;

        *self.current_file.borrow_mut() = Some(path.to_path_buf());
        self.is_modified.set(false);
        self.update_tab_title();
        Ok(())
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
        self.search_prev_btn
            .set_tooltip_text(Some(lang.tooltip_search_previous()));
        self.search_next_btn
            .set_tooltip_text(Some(lang.tooltip_search_next()));
        self.replace_toggle.set_tooltip_text(Some(lang.tooltip_replace()));
        self.replace_entry
            .set_placeholder_text(Some(lang.replace_placeholder()));
        self.replace_btn.set_label(lang.btn_replace());
        self.replace_all_btn.set_label(lang.btn_replace_all());
        self.update_search_status();
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
    ///
    /// Все совпадения подсвечиваются тегом `search-match`, текущее — более
    /// контрастным `search-current`. Состояние хранится в offset'ах буфера,
    /// поэтому после редактирования список пересчитывается, а не ссылается
    /// на устаревшие `TextIter`.
    fn setup_search(self: &Rc<Self>) {
        let tag_table = self.tag_table.clone();

        let highlight_tag = gtk4::TextTag::builder()
            .name("search-match")
            .background("#f5a97f")
            .foreground("#181926")
            .build();
        tag_table.add(&highlight_tag);

        let current_tag = gtk4::TextTag::builder()
            .name("search-current")
            .background("#f97316")
            .foreground("#fff7ed")
            .weight(700)
            .build();
        tag_table.add(&current_tag);
        current_tag.set_priority(highlight_tag.priority() + 1);

        // Ввод запроса: пересчитываем совпадения и начинаем с первого.
        let weak_doc = Rc::downgrade(self);
        self.search_entry.connect_search_changed(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.refresh_search_matches(true);
            }
        });

        // Enter — следующее совпадение, Shift+Enter — предыдущее.
        let weak_doc = Rc::downgrade(self);
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key_controller.connect_key_pressed(move |_, keyval, _, state| {
            if keyval == gtk4::gdk::Key::Return || keyval == gtk4::gdk::Key::KP_Enter {
                if let Some(doc) = weak_doc.upgrade() {
                    let forward = !state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
                    doc.navigate_search(forward);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.search_entry.add_controller(key_controller);

        // Кнопки навигации по совпадениям.
        let weak_doc = Rc::downgrade(self);
        self.search_prev_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.navigate_search(false);
            }
        });
        let weak_doc = Rc::downgrade(self);
        self.search_next_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.navigate_search(true);
            }
        });

        // Показ/скрытие строки замены.
        let replace_row = self.replace_row.clone();
        let replace_entry = self.replace_entry.clone();
        let search_entry = self.search_entry.clone();
        let replace_btn = self.replace_btn.clone();
        let replace_all_btn = self.replace_all_btn.clone();
        self.replace_toggle.connect_toggled(move |button| {
            let visible = button.is_active();
            replace_row.set_visible(visible);
            replace_entry.set_visible(visible);
            replace_btn.set_visible(visible);
            replace_all_btn.set_visible(visible);
            if visible {
                if search_entry.text().is_empty() {
                    search_entry.grab_focus();
                } else {
                    replace_entry.grab_focus();
                }
            }
        });

        let weak_doc = Rc::downgrade(self);
        self.replace_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.replace_current();
            }
        });
        let weak_doc = Rc::downgrade(self);
        self.replace_all_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.replace_all();
            }
        });
        let weak_doc = Rc::downgrade(self);
        self.replace_entry.connect_activate(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.replace_current();
            }
        });

        // Esc в поле замены должен закрывать всю панель, как и в поле поиска.
        let weak_doc = Rc::downgrade(self);
        let replace_key_controller = gtk4::EventControllerKey::new();
        replace_key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        replace_key_controller.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                if let Some(doc) = weak_doc.upgrade() {
                    doc.search_bar.set_search_mode(false);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.replace_entry.add_controller(replace_key_controller);

        // Закрытие панели убирает подсветку и прячет строку замены;
        // повторное открытие с тем же запросом восстанавливает поиск.
        let weak_doc = Rc::downgrade(self);
        self.search_bar
            .connect_search_mode_enabled_notify(move |bar| {
                if let Some(doc) = weak_doc.upgrade() {
                    if bar.is_search_mode() {
                        doc.refresh_search_matches(true);
                    } else {
                        doc.clear_search_highlights();
                        doc.replace_toggle.set_active(false);
                        doc.replace_row.set_visible(false);
                    }
                }
            });
    }

    /// Открывает панель поиска сразу в режиме замены (Ctrl+H).
    ///
    /// Замена работает по тексту редактора, поэтому из предпросмотра
    /// сначала переключаемся в редактор.
    pub fn open_replace_mode(self: &Rc<Self>) {
        if self.is_preview_active.get() {
            self.switch_to_editor();
        }

        self.search_bar.set_search_mode(true);
        self.replace_toggle.set_active(true);
        self.replace_row.set_visible(true);

        // Если запрос еще не введен, сначала нужен поиск, а не поле замены.
        if self.search_entry.text().is_empty() {
            self.search_entry.grab_focus();
        } else {
            self.replace_entry.grab_focus();
        }
    }

    /// Заменяет текущее совпадение и переходит к следующему.
    fn replace_current(self: &Rc<Self>) {
        if self.is_preview_active.get() {
            return;
        }
        let Some((match_start, match_end)) = self
            .search_matches
            .borrow()
            .get(self.search_index.get())
            .copied()
        else {
            return;
        };

        let replacement = self.replace_entry.text().to_string();
        let replacement_len = replacement.chars().count() as i32;

        self.suppress_search_refresh.set(true);
        self.bulk_editing.set(true);
        self.buffer.begin_user_action();

        let mut start_iter = self.buffer.iter_at_offset(match_start);
        let mut end_iter = self.buffer.iter_at_offset(match_end);
        self.buffer.delete(&mut start_iter, &mut end_iter);

        if !replacement.is_empty() {
            let mut insert_iter = self.buffer.iter_at_offset(match_start);
            self.buffer.insert(&mut insert_iter, &replacement);
        }

        self.buffer.end_user_action();
        self.suppress_search_refresh.set(false);
        self.end_bulk_edit();

        self.refresh_search_matches(false);

        let after = match_start + replacement_len;
        let matches = self.search_matches.borrow();
        if !matches.is_empty() {
            let next_index = next_match_index_after(&matches, after);
            drop(matches);
            self.search_index.set(next_index);
            self.update_search_status();
            self.apply_current_search_match();
        }
    }

    /// Заменяет все совпадения одним действием Undo.
    fn replace_all(self: &Rc<Self>) {
        if self.is_preview_active.get() {
            return;
        }
        let query = self.search_entry.text().to_string();
        if query.is_empty() {
            return;
        }
        let replacement = self.replace_entry.text().to_string();

        self.suppress_search_refresh.set(true);
        self.bulk_editing.set(true);
        let count = replace_all_matches(&self.buffer, &query, &replacement);
        self.suppress_search_refresh.set(false);
        self.end_bulk_edit();

        if count > 0 {
            self.refresh_search_matches(true);
        }
    }

    /// Завершает серию правок и один раз обновляет тяжёлые представления.
    fn end_bulk_edit(&self) {
        self.bulk_editing.set(false);
        highlighter::highlight_markdown_buffer(&self.buffer);
        self.gutter.queue_draw();
        self.update_tab_title();
    }

    /// Пересчитывает совпадения по текущему запросу.
    ///
    /// `reset_current` используется при смене запроса: начинаем с первого
    /// совпадения. При редактировании текста текущий индекс сохраняется,
    /// насколько это возможно.
    fn refresh_search_matches(self: &Rc<Self>, reset_current: bool) {
        // Пока панель поиска закрыта, редактирование текста не должно
        // возвращать подсветку старого запроса.
        if !self.search_bar.is_search_mode() {
            self.clear_search_highlights();
            return;
        }

        if self.is_preview_active.get() {
            self.refresh_preview_search(reset_current);
            return;
        }

        let query = self.search_entry.text().to_string();
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();

        self.buffer.remove_tag_by_name("search-match", &start, &end);
        self.buffer.remove_tag_by_name("search-current", &start, &end);

        if query.is_empty() {
            self.search_matches.borrow_mut().clear();
            self.search_index.set(0);
            self.update_search_status();
            return;
        }

        let matches = find_matches(&self.buffer, &query);
        let total = matches.len();

        for &(match_start, match_end) in matches.iter().take(MAX_SEARCH_HIGHLIGHTS) {
            let start_iter = self.buffer.iter_at_offset(match_start);
            let end_iter = self.buffer.iter_at_offset(match_end);
            self.buffer
                .apply_tag_by_name("search-match", &start_iter, &end_iter);
        }

        let index = if reset_current || self.search_index.get() >= total {
            0
        } else {
            self.search_index.get()
        };

        *self.search_matches.borrow_mut() = matches;
        self.search_index.set(index);
        self.update_search_status();
        self.apply_current_search_match();
    }

    /// Переходит к следующему или предыдущему совпадению с заворотом.
    fn navigate_search(self: &Rc<Self>, forward: bool) {
        if self.is_preview_active.get() {
            self.navigate_preview_search(forward);
            return;
        }

        let total = self.search_matches.borrow().len();
        let Some(index) = wrapped_search_index(self.search_index.get(), total, forward) else {
            return;
        };

        self.search_index.set(index);
        self.update_search_status();
        self.apply_current_search_match();
    }

    /// Подсвечивает текущее совпадение и прокручивает к нему редактор.
    fn apply_current_search_match(&self) {
        let Some((match_start, match_end)) = self
            .search_matches
            .borrow()
            .get(self.search_index.get())
            .copied()
        else {
            return;
        };

        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer
            .remove_tag_by_name("search-current", &start, &end);

        let mut match_start_iter = self.buffer.iter_at_offset(match_start);
        let match_end_iter = self.buffer.iter_at_offset(match_end);
        self.buffer.apply_tag_by_name(
            "search-current",
            &match_start_iter,
            &match_end_iter,
        );

        self.buffer.place_cursor(&match_start_iter);
        self.scroll_editor_to_iter(&mut match_start_iter, 0.5);
    }

    /// Прокручивает редактор к итератору по вертикали, не сдвигая
    /// горизонтальную позицию.
    ///
    /// `GtkTextView::scroll_to_iter` с `xalign = 0.0` даже при включенном
    /// переносе строк пытается выровнять позицию по левому краю и может
    /// сдвинуть горизонтальный adjustment. Тогда справа появляется пустая
    /// полоса. После прокрутки возвращаем горизонтальную позицию в ноль.
    fn scroll_editor_to_iter(&self, iter: &mut gtk4::TextIter, yalign: f64) {
        self.text_view.scroll_to_iter(iter, 0.1, true, 0.0, yalign);
        self.reset_editor_horizontal_scroll();
    }

    /// Возвращает горизонтальную прокрутку редактора в начало.
    fn reset_editor_horizontal_scroll(&self) {
        let adjustment = self.editor_scroll.hadjustment();
        adjustment.set_value(0.0);

        // Повторно в idle: GTK может пересчитать adjustment после текущей
        // итерации главного цикла.
        glib::idle_add_local_once(move || adjustment.set_value(0.0));
    }

    /// Убирает подсветку поиска при закрытии панели, сохраняя запрос.
    fn clear_search_highlights(&self) {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer.remove_tag_by_name("search-match", &start, &end);
        self.buffer.remove_tag_by_name("search-current", &start, &end);
        self.search_matches.borrow_mut().clear();
        self.search_index.set(0);
        self.clear_preview_search();
        self.update_search_status();
    }

    /// Снимает preview-подсветку и сбрасывает ее счетчики.
    fn clear_preview_search(&self) {
        self.preview_search_generation
            .set(self.preview_search_generation.get() + 1);
        self.preview_search_count.set(0);
        self.preview_search_index.set(0);

        self.web_view.evaluate_javascript(
            "if (window.__asterSearch) { window.__asterSearch.clear(); }",
            None,
            None,
            Option::<&gio::Cancellable>::None,
            |_| {},
        );
    }

    /// Ищет видимый текст в HTML-предпросмотре через JavaScript.
    fn refresh_preview_search(self: &Rc<Self>, _reset_current: bool) {
        let query = self.search_entry.text().to_string();
        let generation = self.preview_search_generation.get() + 1;
        self.preview_search_generation.set(generation);

        if query.is_empty() {
            self.clear_preview_search();
            self.update_search_status();
            return;
        }

        let query_json = serde_json::to_string(&query)
            .unwrap_or_else(|_| "\"\"".to_string());
        let script = format!(
            "window.__asterSearch ? window.__asterSearch.find({query_json}) : JSON.stringify({{count: 0, index: -1}});"
        );

        let weak_doc = Rc::downgrade(self);
        self.web_view.evaluate_javascript(
            &script,
            None,
            None,
            Option::<&gio::Cancellable>::None,
            move |result| {
                let Some(doc) = weak_doc.upgrade() else {
                    return;
                };
                if doc.preview_search_generation.get() != generation
                    || !doc.is_preview_active.get()
                {
                    return;
                }
                if let Ok(value) = result {
                    doc.apply_preview_search_result(&value.to_str());
                }
            },
        );
    }

    /// Переходит к следующему/предыдущему совпадению в предпросмотре.
    fn navigate_preview_search(self: &Rc<Self>, forward: bool) {
        let generation = self.preview_search_generation.get() + 1;
        self.preview_search_generation.set(generation);

        let script = format!(
            "window.__asterSearch ? window.__asterSearch.next({}) : JSON.stringify({{count: 0, index: -1}});",
            if forward { "true" } else { "false" }
        );

        let weak_doc = Rc::downgrade(self);
        self.web_view.evaluate_javascript(
            &script,
            None,
            None,
            Option::<&gio::Cancellable>::None,
            move |result| {
                let Some(doc) = weak_doc.upgrade() else {
                    return;
                };
                if doc.preview_search_generation.get() != generation
                    || !doc.is_preview_active.get()
                {
                    return;
                }
                if let Ok(value) = result {
                    doc.apply_preview_search_result(&value.to_str());
                }
            },
        );
    }

    /// Применяет JSON-ответ JavaScript-поиска к счетчику и статусу.
    fn apply_preview_search_result(&self, raw: &str) {
        let Some((count, index)) = parse_preview_search_result(raw) else {
            return;
        };

        self.preview_search_count.set(count);
        self.preview_search_index.set(index);
        self.update_search_status();
    }

    /// Обновляет счетчик и доступность кнопок навигации.
    fn update_search_status(&self) {
        let lang = Language::from_code(&self.settings.borrow().language);
        let in_preview = self.is_preview_active.get();
        let (total, current) = if in_preview {
            (
                self.preview_search_count.get(),
                self.preview_search_index.get(),
            )
        } else {
            (self.search_matches.borrow().len(), self.search_index.get())
        };

        self.search_prev_btn.set_sensitive(total > 0);
        self.search_next_btn.set_sensitive(total > 0);
        self.replace_btn.set_sensitive(!in_preview && total > 0);
        self.replace_all_btn.set_sensitive(!in_preview && total > 0);

        if !self.search_bar.is_search_mode() || self.search_entry.text().is_empty() {
            self.search_status.set_text("");
        } else if total == 0 {
            self.search_status.set_text(lang.search_no_results());
        } else {
            self.search_status
                .set_text(&lang.search_counter(current + 1, total));
        }
    }
}

/// Куда направить клик по ссылке в предпросмотре.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LinkTarget {
    /// Внутренний якорь вида `#section`.
    Anchor(String),
    /// Внешний URI, который откроет системный обработчик.
    External(String),
    /// Небезопасная или неподдерживаемая ссылка.
    Blocked,
}

/// Классифицирует URI из HTML-предпросмотра.
///
/// Навигация по самому WebView запрещена: разрешены только внутренние
/// якоря и передача внешнего URI системному приложению.
///
/// `current_file` нужен потому, что при `base href="file:///dir/"` WebKit
/// превращает `#fragment` в `file:///dir/#fragment`. Такой URI является
/// якорем текущего документа, а не ссылкой на файловый менеджер.
fn classify_link_uri(uri: &str, current_file: Option<&Path>) -> LinkTarget {
    let uri = uri.trim();

    if let Some((base, fragment)) = uri.split_once('#') {
        if base.is_empty()
            || current_file
                .map(|file| is_current_document_base(base, file))
                .unwrap_or(false)
        {
            return LinkTarget::Anchor(fragment.to_string());
        }
    }

    let lower = uri.to_ascii_lowercase();
    let external = ["http://", "https://", "mailto:", "tel:", "file://", "ftp://"]
        .iter()
        .any(|prefix| lower.starts_with(prefix));

    if external {
        LinkTarget::External(uri.to_string())
    } else {
        LinkTarget::Blocked
    }
}

/// Проверяет, совпадает ли база URI с текущим файлом или его каталогом.
fn is_current_document_base(base: &str, current_file: &Path) -> bool {
    let file = current_file.to_string_lossy();
    let file_url = format!("file://{file}");

    if base == file_url {
        return true;
    }

    if let Some(parent) = current_file.parent() {
        let parent = parent.to_string_lossy();
        let dir_url = if parent.ends_with('/') {
            format!("file://{parent}")
        } else {
            format!("file://{parent}/")
        };

        if base == dir_url || base.trim_end_matches('/') == dir_url.trim_end_matches('/') {
            return true;
        }
    }

    false
}

/// Разбирает JSON-ответ JavaScript-поиска.
///
/// Возвращает `(количество, индекс)`. При нулевом количестве индекс всегда 0.
fn parse_preview_search_result(raw: &str) -> Option<(usize, usize)> {
    let result = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let count = result.get("count")?.as_u64()? as usize;
    let index = result
        .get("index")
        .and_then(|value| value.as_i64())
        .unwrap_or(-1);

    Some((count, if count == 0 { 0 } else { index.max(0) as usize }))
}

/// Индекс ближайшего совпадения, начинающегося не раньше `after`.
///
/// Если таких нет, возвращает 0: после замены поиск заворачивается на начало.
fn next_match_index_after(matches: &[(i32, i32)], after: i32) -> usize {
    matches
        .iter()
        .position(|(start, _)| *start >= after)
        .unwrap_or(0)
}

/// Заменяет все вхождения `query` на `replacement` и возвращает их количество.
///
/// Правки идут с конца, чтобы offset'ы еще не обработанных совпадений
/// оставались корректными. Все изменения оборачиваются в одно user action,
/// поэтому `Ctrl+Z` отменяет замену целиком.
fn replace_all_matches(buffer: &gtk4::TextBuffer, query: &str, replacement: &str) -> usize {
    let matches = find_matches(buffer, query);
    if matches.is_empty() {
        return 0;
    }

    let original = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    let mut result = String::with_capacity(original.len());
    let mut chars = original.char_indices();
    let mut char_index = 0usize;

    // Собираем новый текст в памяти: одна операция set_text вместо тысяч
    // delete/insert, каждое из которых поднимает changed и перестраивает
    // undo-стек GtkTextBuffer.
    for &(start, end) in &matches {
        let start = start.max(0) as usize;
        let end = end.max(0) as usize;

        while char_index < start {
            let Some((_, ch)) = chars.next() else {
                break;
            };
            result.push(ch);
            char_index += 1;
        }

        result.push_str(replacement);

        while char_index < end {
            if chars.next().is_none() {
                break;
            }
            char_index += 1;
        }
    }

    for (_, ch) in chars {
        result.push(ch);
    }

    // Одна группа Undo: удаляем старый текст и вставляем новый. Это
    // заметно быстрее тысяч отдельных delete/insert и, в отличие от
    // set_text, остается отменяемым действием.
    buffer.begin_user_action();
    let mut start = buffer.start_iter();
    let mut end = buffer.end_iter();
    buffer.delete(&mut start, &mut end);
    if !result.is_empty() {
        let mut insert_at = buffer.start_iter();
        buffer.insert(&mut insert_at, &result);
    }
    buffer.end_user_action();

    matches.len()
}

/// Находит все вхождения запроса и возвращает их диапазоны в offset'ах буфера.
fn find_matches(buffer: &gtk4::TextBuffer, query: &str) -> Vec<(i32, i32)> {
    if query.is_empty() {
        return Vec::new();
    }

    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    let lower_text = text.to_lowercase();
    let lower_query = query.to_lowercase();

    // to_lowercase() может изменить длину строки (например, для некоторых
    // редких Unicode-символов). В таком случае byte offsets нельзя
    // сопоставить с исходным текстом, поэтому используем медленный,
    // но универсальный поиск GTK.
    if lower_query.is_empty() || lower_text.len() != text.len() {
        return find_matches_gtk(buffer, query);
    }

    // Сопоставление byte offset → character offset нужно, потому что
    // GtkTextBuffer и GtkTextIter работают с character offset.
    let mut boundaries = Vec::with_capacity(text.len().min(1_000_000));
    for (byte_index, _) in text.char_indices() {
        boundaries.push(byte_index as u32);
    }
    boundaries.push(text.len() as u32);

    lower_text
        .match_indices(&lower_query)
        .map(|(start, _matched)| {
            let end = start + lower_query.len();
            let start_char = boundaries
                .binary_search(&(start as u32))
                .unwrap_or_else(|index| index);
            let end_char = boundaries
                .binary_search(&(end as u32))
                .unwrap_or_else(|index| index);
            (start_char as i32, end_char as i32)
        })
        .collect()
}

/// Универсальный fallback поиска через GtkTextBuffer.
///
/// Используется, только если lowercase-преобразование меняет длину строки.
fn find_matches_gtk(buffer: &gtk4::TextBuffer, query: &str) -> Vec<(i32, i32)> {
    let mut matches = Vec::new();
    let mut iter = buffer.start_iter();

    while let Some((match_start, match_end)) = iter.forward_search(
        query,
        gtk4::TextSearchFlags::CASE_INSENSITIVE,
        None,
    ) {
        matches.push((match_start.offset(), match_end.offset()));
        iter = match_end;
    }

    matches
}

/// Вычисляет индекс следующего/предыдущего совпадения с заворотом.
fn wrapped_search_index(current: usize, total: usize, forward: bool) -> Option<usize> {
    if total == 0 {
        return None;
    }

    if forward {
        Some((current + 1) % total)
    } else {
        Some((current + total - 1) % total)
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

    // WeakRef разрывает цикл TextView -> gutter -> draw func -> TextView.
    // Иначе каждая закрытая вкладка навсегда удерживала бы свой TextView.
    let text_view_weak = text_view.downgrade();
    let settings_for_draw = settings.clone();

    gutter.set_draw_func(move |area, cr, width, height| {
        let Some(text_view) = text_view_weak.upgrade() else {
            return;
        };

        let buffer = text_view.buffer();
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
            text_view.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, 0);
        let (_, bottom_buf_y) =
            text_view.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, height);

        let top_buf_y = top_buf_y.max(0);
        let (mut iter, _) = text_view.line_at_y(top_buf_y);

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
            let (y_buf, h_buf) = text_view.line_yrange(&iter);
            let (_, win_y) =
                text_view.buffer_to_window_coords(gtk4::TextWindowType::Left, 0, y_buf);

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
fn schedule_cursor_scroll(
    text_view: &gtk4::TextView,
    editor_scroll: &gtk4::ScrolledWindow,
) {
    let text_view_clone = text_view.clone();
    let editor_scroll_clone = editor_scroll.clone();
    glib::idle_add_local_once(move || {
        let buf = text_view_clone.buffer();
        let insert_mark = buf.get_insert();
        let mut iter = buf.iter_at_mark(&insert_mark);
        if iter.line() == 0 {
            text_view_clone.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.0);
        } else {
            text_view_clone.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.5);
        }

        // В режиме переноса строк горизонтальная прокрутка не должна
        // сдвигать текст: это оставляет пустую полосу справа.
        let adjustment = editor_scroll_clone.hadjustment();
        adjustment.set_value(0.0);
        glib::idle_add_local_once(move || adjustment.set_value(0.0));
    });
}

/// Настраивает Tab/Shift+Tab и подавляет системный звук на границах текста.
fn setup_editor_key_handling(text_view: &gtk4::TextView) {
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    // WeakRef разрывает цикл TextView -> controller -> handler -> TextView.
    let text_view_weak = text_view.downgrade();

    key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
        let Some(text_view) = text_view_weak.upgrade() else {
            return glib::Propagation::Proceed;
        };

        let is_control = state.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
        let is_alt = state.contains(gtk4::gdk::ModifierType::ALT_MASK);

        if is_control {
            return glib::Propagation::Proceed;
        }

        if is_alt {
            // Alt+D — дублировать текущую строку ниже. Остальные Alt-комбинации
            // не перехватываем, чтобы не ломать системные/меню-акселераторы.
            if keyval == gtk4::gdk::Key::d || keyval == gtk4::gdk::Key::D {
                duplicate_current_line(&text_view.buffer());
                return glib::Propagation::Stop;
            }
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
            let buffer = text_view.buffer();
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

        let buffer = text_view.buffer();

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

/// Дублирует строку под курсором и ставит курсор на копию.
///
/// Копия вставляется сразу после текущей строки. Если текущая строка
/// последняя и файл не заканчивается переводом строки, перед копией
/// добавляется `\n`. Курсор сохраняет колонку, поэтому повторный `Alt+D`
/// последовательно создает копии.
pub fn duplicate_current_line(buffer: &gtk4::TextBuffer) {
    let insert_mark = buffer.get_insert();
    let cursor = buffer.iter_at_mark(&insert_mark);
    let line_index = cursor.line();
    let column = cursor.line_offset();

    let line_count = buffer.line_count();
    let Some(line_start) = buffer.iter_at_line(line_index) else {
        return;
    };
    let line_end = match buffer.iter_at_line(line_index + 1) {
        Some(mut next_line_start) => {
            next_line_start.backward_char();
            next_line_start
        }
        None => buffer.end_iter(),
    };
    let line_text = buffer.text(&line_start, &line_end, false);
    let line_len = line_text.chars().count() as i32;

    buffer.begin_user_action();

    // Для не последней строки вставляем копию перед началом следующей.
    // Для последней строки без завершающего перевода строки добавляем его
    // перед копией.
    let duplicate_start = if line_index + 1 < line_count {
        let Some(mut insert_at) = buffer.iter_at_line(line_index + 1) else {
            buffer.end_user_action();
            return;
        };
        let start_offset = insert_at.offset();
        buffer.insert(&mut insert_at, &format!("{line_text}\n"));
        start_offset
    } else {
        let mut insert_at = buffer.end_iter();
        let end_offset = insert_at.offset();
        buffer.insert(&mut insert_at, &format!("\n{line_text}"));
        end_offset + 1
    };

    let mut new_cursor = buffer.iter_at_offset(duplicate_start + column.min(line_len));
    buffer.place_cursor(&mut new_cursor);
    buffer.end_user_action();
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

    #[gtk4::test]
    fn test_indent_and_unindent() {
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

    #[gtk4::test]
    fn test_find_matches_is_case_insensitive() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("Foo foo food\nbar\n");

        let matches = find_matches(&buffer, "foo");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].0, 0);
        assert_eq!(matches[1].0, 4);
        assert_eq!(matches[2].0, 8);

        assert!(find_matches(&buffer, "xyz").is_empty());
        assert!(find_matches(&buffer, "").is_empty());

        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("Привет привет ПРИВЕТ");
        let matches = find_matches(&buffer, "привет");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0], (0, 6));
        assert_eq!(matches[1], (7, 13));
    }

    #[test]
    fn test_wrapped_search_index() {
        assert_eq!(wrapped_search_index(0, 3, true), Some(1));
        assert_eq!(wrapped_search_index(2, 3, true), Some(0));
        assert_eq!(wrapped_search_index(0, 3, false), Some(2));
        assert_eq!(wrapped_search_index(2, 3, false), Some(1));
        assert_eq!(wrapped_search_index(0, 0, true), None);
    }

    #[test]
    fn test_classify_link_uri() {
        assert_eq!(
            classify_link_uri("#section-1", None),
            LinkTarget::Anchor("section-1".to_string())
        );
        assert_eq!(
            classify_link_uri("#", None),
            LinkTarget::Anchor(String::new())
        );
        assert_eq!(
            classify_link_uri("https://example.com", None),
            LinkTarget::External("https://example.com".to_string())
        );
        assert_eq!(
            classify_link_uri("MAILTO:test@example.com", None),
            LinkTarget::External("MAILTO:test@example.com".to_string())
        );
        assert_eq!(
            classify_link_uri("file:///tmp/other.md", None),
            LinkTarget::External("file:///tmp/other.md".to_string())
        );
        assert_eq!(
            classify_link_uri("javascript:alert(1)", None),
            LinkTarget::Blocked
        );
        assert_eq!(
            classify_link_uri("data:text/html,hi", None),
            LinkTarget::Blocked
        );
        assert_eq!(
            classify_link_uri("relative/file.md", None),
            LinkTarget::Blocked
        );

        // Регрессия: base href="file:///tmp/" превращает #anchor
        // в file:///tmp/#anchor. Это внутренний якорь, а не ссылка на /tmp.
        let current = std::path::Path::new("/tmp/aster-links.md");
        assert_eq!(
            classify_link_uri("file:///tmp/#target", Some(current)),
            LinkTarget::Anchor("target".to_string())
        );
        assert_eq!(
            classify_link_uri("file:///tmp/aster-links.md#target", Some(current)),
            LinkTarget::Anchor("target".to_string())
        );
        assert_eq!(
            classify_link_uri("https://example.com#target", Some(current)),
            LinkTarget::External("https://example.com#target".to_string())
        );
    }

    #[gtk4::test]
    fn test_gutter_does_not_keep_text_view_alive() {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let text_view = gtk4::TextView::new();
        let weak_text_view = text_view.downgrade();

        // Держим gutter отдельно: если draw-функция захватит TextView
        // сильной ссылкой, цикл не даст TextView освободиться.
        let _gutter = setup_line_numbers_gutter(&text_view, &settings);

        drop(text_view);
        assert!(
            weak_text_view.upgrade().is_none(),
            "gutter draw func keeps TextView alive"
        );
    }

    #[gtk4::test]
    fn test_key_controller_does_not_keep_text_view_alive() {
        let text_view = gtk4::TextView::new();
        let weak_text_view = text_view.downgrade();

        setup_editor_key_handling(&text_view);

        drop(text_view);
        assert!(
            weak_text_view.upgrade().is_none(),
            "key controller keeps TextView alive"
        );
    }

    #[gtk4::test]
    fn test_document_is_freed_after_drop() {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let doc = Document::new(settings, None, false).expect("create document");
        let weak_doc = Rc::downgrade(&doc);
        let weak_web_view = doc.web_view.downgrade();
        let weak_text_view = doc.text_view.downgrade();

        drop(doc);
        assert!(
            weak_doc.upgrade().is_none(),
            "Document is kept alive by internal signal handlers"
        );
        assert!(
            weak_web_view.upgrade().is_none(),
            "WebView is kept alive by its own load handler"
        );
        assert!(
            weak_text_view.upgrade().is_none(),
            "TextView is kept alive by editor signal handlers"
        );
    }

    #[gtk4::test]
    fn test_preview_search_script_finds_visible_text() {
        use std::time::{Duration, Instant};

        let manager = UserContentManager::new();
        manager.add_script(&UserScript::new(
            SCROLL_TRACKER_SCRIPT,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::End,
            &[],
            &[],
        ));

        let web_view = webkit6::WebView::builder()
            .user_content_manager(&manager)
            .build();

        let loaded = Rc::new(Cell::new(false));
        let loaded_for_signal = loaded.clone();
        web_view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                loaded_for_signal.set(true);
            }
        });

        web_view.load_html(
            "<!DOCTYPE html><html><body>             <p>Hello world</p><p>hello again</p>             <script>Hello hidden</script>             </body></html>",
            None,
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while !loaded.get() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(loaded.get(), "preview did not load");

        let evaluate = |script: &str| -> String {
            let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
            let result_for_callback = result.clone();
            web_view.evaluate_javascript(
                script,
                None,
                None,
                Option::<&gio::Cancellable>::None,
                move |value| {
                    if let Ok(value) = value {
                        *result_for_callback.borrow_mut() = Some(value.to_str().to_string());
                    }
                },
            );

            let deadline = Instant::now() + Duration::from_secs(5);
            while result.borrow().is_none() && Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(10));
            }

            let output = result.borrow().clone().expect("JS result");
            output
        };

        let first = evaluate("window.__asterSearch.find('hello')");
        assert_eq!(parse_preview_search_result(&first), Some((2, 0)));

        let second = evaluate("window.__asterSearch.next(true)");
        assert_eq!(parse_preview_search_result(&second), Some((2, 1)));

        let mark_count = evaluate(
            "document.querySelectorAll('mark.aster-search-match, mark.aster-search-current').length",
        );
        assert_eq!(mark_count, "2");
    }

    #[test]
    fn test_parse_preview_search_result() {
        assert_eq!(parse_preview_search_result(r#"{"count":3,"index":1}"#), Some((3, 1)));
        assert_eq!(parse_preview_search_result(r#"{"count":0,"index":-1}"#), Some((0, 0)));
        assert_eq!(parse_preview_search_result(r#"{"count":2,"index":-1}"#), Some((2, 0)));
        assert_eq!(parse_preview_search_result("not json"), None);
    }

    #[test]
    fn test_next_match_index_after_replacement() {
        let matches = [(0, 3), (10, 13), (20, 23)];
        assert_eq!(next_match_index_after(&matches, 0), 0);
        assert_eq!(next_match_index_after(&matches, 4), 1);
        assert_eq!(next_match_index_after(&matches, 11), 2);
        assert_eq!(next_match_index_after(&matches, 21), 0);
        assert_eq!(next_match_index_after(&[], 0), 0);
    }

    #[gtk4::test]
    fn test_replace_all_matches() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("Foo foo food\nbar\n");

        buffer.set_enable_undo(true);
        let original = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let count = replace_all_matches(&buffer, "foo", "baz");
        assert_eq!(count, 3);
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text.as_str(), "baz baz bazd\nbar\n");

        assert!(buffer.can_undo());
        buffer.undo();
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            original.as_str()
        );

        // Пустая строка замены удаляет совпадения, но не ломает текст.
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("a-a-a");
        assert_eq!(replace_all_matches(&buffer, "a", ""), 3);
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            "--"
        );

        assert_eq!(replace_all_matches(&buffer, "", "x"), 0);
    }

    #[gtk4::test]
    fn test_duplicate_current_line() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("first\nsecond\nthird\n");

        let cursor = buffer.iter_at_line_offset(1, 3).expect("line offset");
        buffer.place_cursor(&cursor);
        duplicate_current_line(&buffer);

        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text.as_str(), "first\nsecond\nsecond\nthird\n");

        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        assert_eq!(cursor.line(), 2);
        assert_eq!(cursor.line_offset(), 3);
    }

    #[gtk4::test]
    fn test_duplicate_last_line_without_trailing_newline() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("one\ntwo");

        let cursor = buffer.iter_at_line_offset(1, 1).expect("line offset");
        buffer.place_cursor(&cursor);
        duplicate_current_line(&buffer);

        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text.as_str(), "one\ntwo\ntwo");

        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        assert_eq!(cursor.line(), 2);
        assert_eq!(cursor.line_offset(), 1);
    }

    #[gtk4::test]
    fn test_cursor_position_restore() {
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
