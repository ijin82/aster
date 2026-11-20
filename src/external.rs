//! Открытие внешних ссылок системным обработчиком.
//!
//! Ссылки на справку по правилам линтера всегда фиксированные и ведут на
//! сайт markdownlint. Здесь нет загрузки содержимого: мы лишь передаём URI
//! системе, поэтому сеть и безопасность остаются вне приложения.

use gtk4::gio;

/// Открывает URI в браузере или другом обработчике по умолчанию.
///
/// Запуск асинхронный: интерфейс не блокируется, пока система ищет и
/// запускает браузер. Ошибка лишь пишется в лог — показывать модальный
/// диалог из-за недоступного браузера было бы навязчиво.
pub(crate) fn open_uri(uri: &str) {
    let uri = uri.to_string();
    let uri_for_log = uri.clone();

    gio::AppInfo::launch_default_for_uri_async(
        &uri,
        Option::<&gio::AppLaunchContext>::None,
        Option::<&gio::Cancellable>::None,
        move |result| {
            if let Err(error) = result {
                gtk4::glib::g_warning!("aster", "не удалось открыть {uri_for_log}: {error}");
            }
        },
    );
}
