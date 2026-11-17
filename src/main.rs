//! Главный модуль приложения `Aster`.
//!
//! Инициализирует приложение на базе `adw::Application`, обрабатывает сигналы
//! `activate` и `open`, а также восстанавливает прошлую сессию (открытые
//! вкладки) при обычном запуске без аргументов.

use adw::prelude::*;
use gtk4::glib::variant::ToVariant;
use std::fs::OpenOptions;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
mod document;
mod highlighter;
mod i18n;
mod recovery;
mod renderer;
mod settings;
mod shortcuts;
mod window;

/// Уникальный идентификатор приложения по стандарту Reverse-DNS (D-Bus ID).
const APP_ID: &str = "io.github.aster";

/// Нужно ли запускать GUI отдельным процессом.
///
/// Отсоединяемся только при запуске из настоящего терминала. Если stderr
/// перенаправлен в файл, пользователь явно хочет видеть логи — оставляем
/// процесс на переднем плане. `ASTER_FOREGROUND=1` отключает отсоединение.
fn should_detach() -> bool {
    if std::env::var_os("ASTER_DETACHED").is_some()
        || std::env::var_os("ASTER_FOREGROUND").is_some()
    {
        return false;
    }

    std::io::stderr().is_terminal()
}

/// Запускает копию Aster в фоне и перенаправляет ее вывод в лог.
///
/// Возвращает `true`, если родительский процесс может завершиться. При
/// ошибке `spawn` возвращает `false`, и приложение продолжает работу в
/// текущем процессе — лучше показать предупреждение в консоли, чем не
/// запустить редактор вообще.
fn detach_from_terminal() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };

    let log_file = dirs::cache_dir().and_then(|dir| {
        let dir = dir.join("aster");
        std::fs::create_dir_all(&dir).ok()?;
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(dir.join("aster.log"))
            .ok()
    });

    let mut command = Command::new(exe);
    command
        .args(std::env::args_os().skip(1))
        .env("ASTER_DETACHED", "1")
        .stdin(Stdio::null());

    // Новая сессия отвязывает GUI от управляющего терминала: закрытие
    // терминала или выход из `script` не должен убивать редактор.
    // SAFETY: `pre_exec` выполняется между fork и exec. `setsid` —
    // async-signal-safe системный вызов и не обращается к памяти процесса.
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    match log_file {
        Some(file) => {
            let stderr_file = match file.try_clone() {
                Ok(file) => file,
                Err(_) => return false,
            };
            command.stdout(Stdio::from(file)).stderr(Stdio::from(stderr_file));
        }
        None => {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }

    command.spawn().is_ok()
}

fn main() {
    // `--version` и `--help` обрабатываем до инициализации GTK, чтобы они
    // работали в терминале и в сценариях упаковки без графической сессии.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("aster v{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Aster — просмотрщик и редактор Markdown\n\n\
             Использование: aster [ФАЙЛ ...]\n\n\
             Параметры:\n  -h, --help     показать эту справку\n  -V, --version  показать версию\n\n\
             Переменные окружения:\n  ASTER_FOREGROUND=1  не отсоединяться от терминала"
        );
        return;
    }

    // Как `code` из VS Code: при запуске из терминала возвращаем приглашение
    // сразу, а GUI продолжаем в отдельном процессе. Иначе GTK-предупреждения
    // и прочий вывод остаются в консоли пользователя и мешают работе.
    if should_detach() && detach_from_terminal() {
        return;
    }

    // HANDLES_OPEN позволяет GApplication самому разобрать пути из командной
    // строки и передать их в сигнал `open`, не дублируя их ручным разбором env::args.
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    // Обычный запуск без файлов: окно Aster восстанавливает вкладки прошлой сессии.
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
