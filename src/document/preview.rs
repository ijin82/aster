//! HTML-предпросмотр документа на базе WebKitGTK.
//!
//! Модуль инкапсулирует:
//! - внедряемый `UserScript` для якорей заголовков, поиска по DOM и
//!   отслеживания вертикальной прокрутки (`SCROLL_TRACKER_SCRIPT`);
//! - безопасную настройку `WebKitWebView` (отключение медиа, звука,
//!   локального хранилища и произвольного доступа к `file://`);
//! - переключение режимов «Редактор / Предпросмотр» и ленивый рендеринг HTML.

use adw::prelude::*;
use gtk4::gio;
use std::cell::Cell;
use std::rc::Rc;
use webkit6::prelude::*;
use webkit6::{
    UserContentInjectedFrames, UserContentManager, UserScript, UserScriptInjectionTime,
};

use crate::renderer;

use super::Document;

/// Имя обработчика JavaScript-сообщений, через который WebKit сообщает
/// актуальную позицию прокрутки предпросмотра.
pub(super) const SCROLL_MESSAGE_HANDLER: &str = "asterPreviewScroll";

/// JavaScript-скрипт, внедряемый в каждую страницу предпросмотра.
///
/// Он слушает событие `scroll` и отправляет `window.scrollY` в нативный код.
/// Благодаря этому позиция предпросмотра запоминается непрерывно, без
/// синхронного опроса WebKit при переключении режимов.
pub(super) const SCROLL_TRACKER_SCRIPT: &str = r#"
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

/// Создаёт `WebKitWebView` и его `UserContentManager` с безопасными настройками.
pub(super) fn build_preview_web_view() -> (UserContentManager, webkit6::WebView) {
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

    (user_content_manager, web_view)
}

impl Document {
    /// Подключает обработчики WebKit: приём `scrollY` из JS и восстановление
    /// прокрутки/поиска после завершения загрузки HTML.
    pub(super) fn setup_preview_handlers(
        self: &Rc<Self>,
        user_content_manager: UserContentManager,
        preview_scroll_position: Rc<Cell<f64>>,
        pending_preview_scroll: Rc<Cell<Option<f64>>>,
    ) {
        // WebKit присылает scrollY через message handler. Сохраняем значение
        // в ячейку документа, чтобы при возврате в предпросмотр восстановить его.
        let preview_cell_for_message = preview_scroll_position.clone();
        user_content_manager.connect_script_message_received(
            Some(SCROLL_MESSAGE_HANDLER),
            move |_, value| {
                if value.is_number() {
                    preview_cell_for_message.set(value.to_double().max(0.0));
                }
            },
        );

        // После загрузки HTML восстанавливаем позицию прокрутки предпросмотра.
        // `pending_preview_scroll` защищает от гонки со скриптом отслеживания,
        // который может успеть сообщить 0 до программной прокрутки.
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
            p.parent().map(|parent_dir| {
                // `gio::File::uri()` корректно кодирует пробелы, кириллицу и
                // спецсимволы (`#`, `?`, `%`), чтобы WebKit не путал их с
                // якорем или строкой запроса.
                let uri = gio::File::for_path(parent_dir).uri().to_string();
                if uri.ends_with('/') {
                    uri
                } else {
                    format!("{uri}/")
                }
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
}
