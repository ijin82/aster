//! Главный модуль приложения `rmd-view`.
//!
//! Инициализирует приложение на базе `adw::Application` (библиотека Libadwaita),
//! обрабатывает сигналы активации (`activate`) и открытия файлов (`open`),
//! а также разбирает переданные аргументы командной строки.

use adw::prelude::*;
use std::env;
use std::path::PathBuf;

mod renderer;
mod settings;
mod window;

/// Уникальный идентификатор приложения по стандарту Reverse-DNS (D-Bus ID).
const APP_ID: &str = "io.github.rmd_view";

fn main() {
    // Создаем экземпляр приложения Adwaita с поддержкой открытия файлов (HANDLES_OPEN)
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    // Сигнал 'activate': вызывается при обычном запуске приложения без аргументов
    app.connect_activate(|app| {
        let win = window::build_window(app, None);
        win.present();
    });

    // Сигнал 'open': вызывается при открытии файлов через контекстное меню GNOME или D-Bus
    app.connect_open(|app, files, _hint| {
        let path = files.first().and_then(|f| f.path());
        let win = window::build_window(app, path);
        win.present();
    });

    // Обработка прямого аргумента пути из консоли: `rmd-view path/to/file.md`
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && !args[1].starts_with('-') {
        let path = PathBuf::from(&args[1]);
        if path.exists() {
            app.connect_activate(move |app| {
                let win = window::build_window(app, Some(path.clone()));
                win.present();
            });
        }
    }

    // Запускаем основной цикл событий GTK/GLib (event loop)
    app.run();
}
