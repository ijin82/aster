//! Политика ссылок в HTML-предпросмотре.

use adw::prelude::*;
use gtk4::glib;
use gtk4::gio;
use std::path::Path;
use std::rc::Rc;
use webkit6::prelude::*;

use super::Document;

impl Document {
    /// Настраивает безопасное поведение ссылок в предпросмотре.
    ///
    /// Внутренние якоря прокручиваются внутри страницы, внешние URI
    /// открываются системным обработчиком, а сам WebView не уходит
    /// с текущего HTML-документа.
    pub(super) fn setup_link_policy(self: &Rc<Self>) {
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
}

/// Куда направить клик по ссылке в предпросмотре.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LinkTarget {
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
pub(super) fn classify_link_uri(uri: &str, current_file: Option<&Path>) -> LinkTarget {
    let uri = uri.trim();

    if let Some((base, fragment)) = uri.split_once('#') {
        if base.is_empty()
            || base.eq_ignore_ascii_case("about:blank")
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
pub(super) fn is_current_document_base(base: &str, current_file: &Path) -> bool {
    let file = current_file.to_string_lossy();
    let file_url = format!("file://{file}");
    let encoded_file_url = gio::File::for_path(current_file).uri();

    if base == file_url || base == encoded_file_url.as_str() {
        return true;
    }

    if let Some(parent) = current_file.parent() {
        let parent_str = parent.to_string_lossy();
        let dir_url = if parent_str.ends_with('/') {
            format!("file://{parent_str}")
        } else {
            format!("file://{parent_str}/")
        };
        let encoded_dir_url = gio::File::for_path(parent).uri();

        if base == dir_url
            || base.trim_end_matches('/') == dir_url.trim_end_matches('/')
            || base.trim_end_matches('/') == encoded_dir_url.as_str().trim_end_matches('/')
        {
            return true;
        }
    }

    false
}

