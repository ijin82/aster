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
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
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

mod diagnostics;
mod editor;
mod gutter;
mod links;
mod search;
#[cfg(test)]
mod tests;
mod whitespace;

use diagnostics::Diagnostic;
use editor::{
    apply_restored_cursor_position, schedule_cursor_scroll, setup_editor_key_handling,
};
use gutter::setup_line_numbers_gutter;
use whitespace::setup_whitespace_layer;

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

/// Причина уведомления об изменении файла на диске.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    /// Файл изменился, несохранённых правок не было — документ перезагружен.
    Reloaded,
    /// Файл изменился, но в документе есть несохранённые правки.
    Conflict,
}

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
    /// Последний загруженный HTML предпросмотра и base_uri.
    ///
    /// Если содержимое и настройки не менялись, при переключении режимов
    /// не нужно заново вызывать `load_html`: WebKit сохраняет DOM и позицию
    /// прокрутки скрытого WebView. Без этого новый документ успевал
    /// отрисоваться сверху до восстановления сохраненной прокрутки.
    last_preview_render: RefCell<Option<(String, Option<String>)>>,

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
    /// Найденные нарушения стиля Markdown.
    ///
    /// Список общий с gutter: колонка номеров строк рисует по нему цветные
    /// маркеры, в том числе для пустых строк, которые нельзя подчеркнуть.
    diagnostics: Rc<RefCell<Vec<Diagnostic>>>,
    /// Таймер отложенной проверки стиля после последней правки.
    diagnostics_timeout: Cell<Option<glib::SourceId>>,
    /// Слой редактора, поверх которого показывается собственный tooltip.
    lint_tooltip_overlay: gtk4::Overlay,
    /// Карточка собственного tooltip линтера.
    lint_tooltip: gtk4::Box,
    /// Текст собственного tooltip линтера.
    lint_tooltip_label: gtk4::Label,
    /// Текстовый буфер редактора.
    pub buffer: gtk4::TextBuffer,
    /// Виджет редактора.
    pub text_view: gtk4::TextView,
    /// Прокручиваемая область редактора.
    pub editor_scroll: gtk4::ScrolledWindow,
    /// Прозрачный слой для отрисовки пробелов и табов.
    pub whitespace_area: gtk4::DrawingArea,
    /// Показывать ли пробелы и табы.
    pub show_whitespace: Rc<Cell<bool>>,
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

    /// Монитор изменений файла на диске. Хранится здесь, чтобы не дать
    /// GObject-сигналу монитора быть уничтоженным вместе с локальной копией.
    file_monitor: RefCell<Option<gio::FileMonitor>>,
    /// Обработчик, который окно устанавливает для показа toast-уведомлений.
    file_change_handler: RefCell<Option<Rc<dyn Fn(FileChange)>>>,
    /// Хеш последней внешней версии файла, о которой уже показали toast.
    ///
    /// Позволяет не дублировать уведомление на серию событий `Changed` /
    /// `ChangesDoneHint` от одного сохранения, но показать новый toast,
    /// если файл изменили ещё раз, а пользователь старый toast не закрыл.
    last_external_file_hash: RefCell<Option<u64>>,
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

        // Теги диагностики живут в той же таблице, что и подсветка
        // синтаксиса, но применяются отдельно: линтер не конфликтует
        // с `md-*` и может быть выключен без пересоздания буфера.
        diagnostics::register_tags(&tag_table);

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

        // Список диагностик создаётся до gutter: колонка номеров строк
        // получает его копию и рисует маркеры найденных проблем.
        let diagnostics = Rc::new(RefCell::new(Vec::new()));

        let gutter = setup_line_numbers_gutter(&text_view, &settings, &diagnostics);
        let editor_scroll = gtk4::ScrolledWindow::builder()
            .child(&text_view)
            .hexpand(true)
            .vexpand(true)
            .build();

        // Прозрачный слой поверх редактора для точек пробелов и линий табов.
        let whitespace_area = gtk4::DrawingArea::new();
        whitespace_area.set_hexpand(true);
        whitespace_area.set_vexpand(true);
        whitespace_area.set_halign(gtk4::Align::Fill);
        whitespace_area.set_valign(gtk4::Align::Fill);
        whitespace_area.set_can_target(false);
        let show_whitespace = Rc::new(Cell::new(settings.borrow().show_whitespace));
        setup_whitespace_layer(&whitespace_area, &text_view, show_whitespace.clone());

        let editor_overlay = gtk4::Overlay::new();
        editor_overlay.set_child(Some(&editor_scroll));
        editor_overlay.add_overlay(&whitespace_area);

        // Собственный tooltip вместо GtkTooltip. Штатный механизм GTK
        // перестаёт показывать подсказку после нескольких срабатываний
        // на одном и том же виджете, поэтому рисуем карточку сами.
        // Находится поверх текста, не перехватывает мышь и не является
        // ни popover, ни диалогом.
        let lint_tooltip_label = gtk4::Label::new(None);
        lint_tooltip_label.set_xalign(0.0);
        lint_tooltip_label.set_wrap(true);
        lint_tooltip_label.set_max_width_chars(56);
        lint_tooltip_label.set_margin_top(8);
        lint_tooltip_label.set_margin_bottom(8);
        lint_tooltip_label.set_margin_start(10);
        lint_tooltip_label.set_margin_end(10);

        let lint_tooltip = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        lint_tooltip.add_css_class("lint-tooltip");
        lint_tooltip.set_can_target(false);
        lint_tooltip.set_halign(gtk4::Align::Start);
        lint_tooltip.set_valign(gtk4::Align::Start);
        lint_tooltip.set_visible(false);
        lint_tooltip.append(&lint_tooltip_label);
        editor_overlay.add_overlay(&lint_tooltip);

        // Перерисовываем номера строк и слой пробелов при любой прокрутке.
        let gutter_for_scroll = gutter.clone();
        let whitespace_for_vscroll = whitespace_area.clone();
        editor_scroll.vadjustment().connect_value_changed(move |_| {
            gutter_for_scroll.queue_draw();
            whitespace_for_vscroll.queue_draw();
        });
        let whitespace_for_hscroll = whitespace_area.clone();
        editor_scroll.hadjustment().connect_value_changed(move |_| {
            whitespace_for_hscroll.queue_draw();
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
        view_stack.add_titled(&editor_overlay, Some("editor"), lang.tab_editor());

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
            last_preview_render: RefCell::new(None),
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
            diagnostics,
            diagnostics_timeout: Cell::new(None),
            lint_tooltip_overlay: editor_overlay.clone(),
            lint_tooltip,
            lint_tooltip_label,
            buffer,
            text_view,
            editor_scroll,
            whitespace_area,
            show_whitespace,
            gutter,
            tag_table,
            web_view,
            preview_scroll,
            had_saved_view_state: Cell::new(false),
            file_monitor: RefCell::new(None),
            file_change_handler: RefCell::new(None),
            last_external_file_hash: RefCell::new(None),
        });

        doc.setup_handlers(
            user_content_manager,
            preview_scroll_position,
            pending_preview_scroll,
        );
        doc.apply_editor_options();

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
                doc.whitespace_area.queue_draw();
                // При правке старая подсказка может относиться к уже
                // исчезнувшей диагностике.
                doc.hide_lint_tooltip();
                highlighter::highlight_markdown_buffer(buf);

                // Проверку стиля не запускаем на каждое нажатие: она
                // выполнится один раз после короткой паузы в наборе.
                doc.schedule_diagnostics();

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
        setup_editor_key_handling(
            &self.text_view,
            &self.editor_scroll,
            self.settings.clone(),
        );

        // 6. Поиск по буферу текущего документа.
        self.setup_search();

        // 7. Политика ссылок и запрет навигации самого WebView.
        self.setup_link_policy();

        // 8. Подсказки и Ctrl+клик по нарушениям стиля.
        self.setup_lint_links();
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
        // Хеш текущего содержимого на диске: первое событие монитора
        // после открытия не должно выглядеть как внешнее изменение.
        *self.last_external_file_hash.borrow_mut() = Some(hash_text(&content));
        // Файл загружен целиком — проверяем стиль сразу, не дожидаясь
        // дебаунса: пользователь должен увидеть проблемы при открытии.
        self.refresh_diagnostics();
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
            schedule_cursor_scroll(
            &self.text_view,
            &self.editor_scroll,
            self.settings.borrow().soft_wrap,
        );
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
        let reloaded = self.refresh_preview();

        // Если HTML не изменился, WebView сохранил старые mark-элементы,
        // но `switch_to_editor` их очищает. В этом случае подсветку поиска
        // нужно наложить заново вручную, без перезагрузки страницы.
        if !reloaded
            && self.search_bar.is_search_mode()
            && !self.search_entry.text().is_empty()
        {
            self.refresh_preview_search(true);
        }

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
    ///
    /// Возвращает `true`, если HTML действительно загружен в WebView, и
    /// `false`, если отрисованная страница не изменилась и была переиспользована.
    pub fn refresh_preview(&self) -> bool {
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

        // Сравниваем и html, и base_uri: при «Сохранить как...» путь мог
        // измениться, а относительные картинки должны загружаться из новой
        // папки даже при том же Markdown.
        if let Some((cached_html, cached_base_uri)) = self.last_preview_render.borrow().as_ref()
        {
            if cached_html == &html && cached_base_uri == &base_uri {
                return false;
            }
        }

        // Запоминаем желаемую позицию до загрузки: загрузка начнется
        // асинхронно, и скрипт отслеживания не должен успеть обнулить ее.
        self.pending_preview_scroll
            .set(Some(self.preview_scroll_position.get()));
        self.web_view.load_html(&html, base_uri.as_deref());
        *self.last_preview_render.borrow_mut() = Some((html, base_uri));
        true
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
    pub fn save_existing(self: &Rc<Self>) -> Result<(), String> {
        let path = self
            .current_file
            .borrow()
            .clone()
            .ok_or_else(|| "document has no file path".to_string())?;
        self.save_to(&path)
    }

    /// Сохраняет документ по указанному пути и делает его текущим файлом.
    pub fn save_to(self: &Rc<Self>, path: &Path) -> Result<(), String> {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        let text = self.buffer.text(&start, &end, false);

        std::fs::write(path, text.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))?;

        *self.current_file.borrow_mut() = Some(path.to_path_buf());
        self.is_modified.set(false);
        // Запоминаем хеш собственной записи: монитор увидит событие от
        // нашего сохранения и не должен показывать его как внешнее.
        *self.last_external_file_hash.borrow_mut() = Some(hash_text(&text));
        self.update_tab_title();
        self.restart_file_monitor();
        Ok(())
    }

    /// Подписывается на изменения текущего файла на диске.
    ///
    /// Окно передает обработчик, чтобы показать toast и, при конфликте,
    /// предложить кнопку «Перезагрузить».
    pub fn watch_file(self: &Rc<Self>, handler: Rc<dyn Fn(FileChange)>) {
        *self.file_change_handler.borrow_mut() = Some(handler);
        self.restart_file_monitor();
    }

    /// Пересоздает `gio::FileMonitor` для текущего пути.
    ///
    /// Вызывается после «Сохранить как», когда у документа появляется
    /// новый файл, а также при установке обработчика окна.
    fn restart_file_monitor(self: &Rc<Self>) {
        *self.file_monitor.borrow_mut() = None;

        let Some(path) = self.current_file.borrow().clone() else {
            return;
        };
        let Some(handler) = self.file_change_handler.borrow().clone() else {
            return;
        };

        let file = gio::File::for_path(&path);
        let Ok(monitor) = file.monitor_file(
            gio::FileMonitorFlags::WATCH_MOVES,
            gio::Cancellable::NONE,
        ) else {
            return;
        };

        let weak_doc = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, event| {
            // Удаление и размонтирование не пытаемся читать: файла может
            // уже не быть. Атомарные сохранения приходят как Changed/Created
            // или Moved/Renamed.
            if matches!(
                event,
                gio::FileMonitorEvent::Deleted
                    | gio::FileMonitorEvent::MovedOut
                    | gio::FileMonitorEvent::PreUnmount
                    | gio::FileMonitorEvent::Unmounted
                    | gio::FileMonitorEvent::AttributeChanged
            ) {
                return;
            }

            let Some(doc) = weak_doc.upgrade() else {
                return;
            };
            let Some(path) = doc.current_file.borrow().clone() else {
                return;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                return;
            };

            let current = doc
                .buffer
                .text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false);
            if content == current {
                // Это событие от нашего же сохранения — ничего не делаем.
                return;
            }

            // Хешируем именно внешнюю версию. Серия событий от одного
            // сохранения даст один и тот же хеш и не создаст несколько toast.
            let content_hash = hash_text(&content);
            if *doc.last_external_file_hash.borrow() == Some(content_hash) {
                return;
            }
            *doc.last_external_file_hash.borrow_mut() = Some(content_hash);

            if doc.is_modified.get() {
                handler(FileChange::Conflict);
                return;
            }

            match doc.reload_from_disk() {
                Ok(()) => {
                    handler(FileChange::Reloaded);
                }
                Err(_) => {
                    handler(FileChange::Conflict);
                }
            }
        });

        *self.file_monitor.borrow_mut() = Some(monitor);
    }

    /// Перечитывает файл с диска, сохраняя курсор и режим просмотра.
    ///
    /// Если содержимое уже совпадает с буфером, документ не перезагружается.
    pub fn reload_from_disk(&self) -> Result<(), String> {
        let path = self
            .current_file
            .borrow()
            .clone()
            .ok_or_else(|| "document has no file path".to_string())?;
        let content = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;

        let current = self
            .buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), false);
        if current == content {
            self.is_modified.set(false);
            *self.last_external_file_hash.borrow_mut() = Some(hash_text(&content));
            self.update_tab_title();
            return Ok(());
        }

        let insert_mark = self.buffer.get_insert();
        let cursor = self.buffer.iter_at_mark(&insert_mark);
        let cursor_line = cursor.line().max(0);
        let cursor_column = cursor.line_offset().max(0);

        self.bulk_editing.set(true);
        self.buffer.set_text(&content);
        self.bulk_editing.set(false);
        highlighter::highlight_markdown_buffer(&self.buffer);
        // Внешнее изменение уже применилось к буферу — сразу обновляем
        // диагностику, чтобы подчёркивания не отставали от текста.
        self.refresh_diagnostics();

        let line_count = self.buffer.line_count();
        let target_line = cursor_line.min(line_count.saturating_sub(1)).max(0);
        let mut iter = self
            .buffer
            .iter_at_line(target_line)
            .unwrap_or_else(|| self.buffer.start_iter());
        let valid_column = cursor_column.min(iter.chars_in_line()).max(0);
        iter.set_line_offset(valid_column);
        self.buffer.place_cursor(&iter);

        self.is_modified.set(false);
        *self.last_external_file_hash.borrow_mut() = Some(hash_text(&content));
        self.update_tab_title();
        if self.is_preview_active.get() {
            self.refresh_preview();
        }
        schedule_cursor_scroll(
            &self.text_view,
            &self.editor_scroll,
            self.settings.borrow().soft_wrap,
        );
        Ok(())
    }

    /// Восстанавливает содержимое безымянного черновика после сбоя.
    pub fn restore_draft(&self, content: &str, line: usize, column: usize) {
        self.bulk_editing.set(true);
        self.buffer.set_text(content);
        self.bulk_editing.set(false);
        highlighter::highlight_markdown_buffer(&self.buffer);
        // Черновик восстановлен целиком: проверяем его сразу.
        self.refresh_diagnostics();

        let line_count = self.buffer.line_count();
        let target_line = (line as i32).min(line_count.saturating_sub(1)).max(0);
        let mut iter = self
            .buffer
            .iter_at_line(target_line)
            .unwrap_or_else(|| self.buffer.start_iter());
        let valid_column = (column as i32).min(iter.chars_in_line()).max(0);
        iter.set_line_offset(valid_column);
        self.buffer.place_cursor(&iter);

        self.is_modified.set(true);
        *self.last_external_file_hash.borrow_mut() = None;
        self.update_tab_title();
        schedule_cursor_scroll(
            &self.text_view,
            &self.editor_scroll,
            self.settings.borrow().soft_wrap,
        );
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

    /// Применяет настройки редактора к текущему документу.
    ///
    /// Вызывается при создании вкладки и при изменении соответствующих
    /// переключателей в Preferences.
    pub fn apply_editor_options(&self) {
        let (soft_wrap, tab_width, show_whitespace) = {
            let settings = self.settings.borrow();
            (
                settings.soft_wrap,
                settings.tab_width,
                settings.show_whitespace,
            )
        };

        self.text_view.set_wrap_mode(if soft_wrap {
            gtk4::WrapMode::Word
        } else {
            gtk4::WrapMode::None
        });
        self.show_whitespace.set(show_whitespace);
        self.whitespace_area.queue_draw();
        // Смена переноса меняет высоту логических строк, поэтому номера
        // нужно перерисовать даже без прокрутки.
        self.gutter.queue_draw();
        self.update_tab_stops(tab_width);

        // При включении переноса возвращаем горизонтальную прокрутку в 0,
        // иначе после режима без переноса справа остается пустая полоса.
        if soft_wrap {
            self.editor_scroll.hadjustment().set_value(0.0);
        }
    }

    /// Настраивает ширину табуляции в пикселях через Pango TabArray.
    fn update_tab_stops(&self, tab_width: u32) {
        let cells = tab_width.clamp(1, 8) as i32;
        let layout = self.text_view.create_pango_layout(Some("M"));
        let (cell_width, _) = layout.pixel_size();
        let step = (cell_width.max(1) * cells).max(1);

        let mut tabs = gtk4::pango::TabArray::new(64, true);
        for index in 0..64 {
            tabs.set_tab(index, gtk4::pango::TabAlign::Left, index * step);
        }
        self.text_view.set_tabs(&tabs);
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
        self.whitespace_area.queue_draw();
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

}

/// Вычисляет быстрый хеш содержимого файла.
///
/// Хеш нужен не для криптографии, а чтобы отличить повторное событие
/// `GFileMonitor` от реального изменения. Он также подавляет реакцию на
/// собственную запись: после сохранения документ запоминает хеш того, что
/// сам записал, и не показывает тост «файл изменён на диске».
fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
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
