//! Главный модуль приложения `rmd-view`.
//!
//! Инициализирует приложение на базе `adw::Application`, обрабатывает сигналы
//! `activate` и `open`, а также восстанавливает прошлую сессию (открытые
//! вкладки) при обычном запуске без аргументов.

use adw::prelude::*;
use gtk4::glib::variant::ToVariant;
use std::path::PathBuf;
mod document;
mod highlighter;
mod i18n;
mod renderer;
mod settings;
mod window;

/// Уникальный идентификатор приложения по стандарту Reverse-DNS (D-Bus ID).
const APP_ID: &str = "io.github.rmd_view";

fn main() {
    // HANDLES_OPEN позволяет GApplication самому разобрать пути из командной
    // строки и передать их в сигнал `open`, не дублируя их ручным разбором env::args.
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    // Обычный запуск без файлов: окно восстанавливает вкладки прошлой сессии.
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let win = window::build_window(app, Vec::new(), true);
        win.present();
    });

    // Запуск с файлами (из терминала, файлового менеджера или по D-Bus).
    app.connect_open(|app, files, _hint| {
        let paths: Vec<PathBuf> = files.iter().filter_map(|file| file.path()).collect();
        if paths.is_empty() {
            return;
        }

        if let Some(window) = app.active_window() {
            // В уже запущенном приложении каждый файл открывается в новой вкладке.
            for path in paths {
                let _ = window.activate_action(
                    "win.open_document",
                    Some(&path.to_string_lossy().to_string().to_variant()),
                );
            }
            window.present();
        } else {
            let win = window::build_window(app, paths, false);
            win.present();
        }
    });

    // Запускаем основной цикл событий GTK/GLib (event loop).
    app.run();
}
