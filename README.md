# rmd-view 🦀

**rmd-view** — это обучающее легковесное приложение на **Rust**, **GTK4**, **Libadwaita** и **WebKitGTK 6.0** для просмотра и редактирования Markdown-документов в стиле GNOME.

Главная цель проекта — служить понятным практическим пособием по созданию графических Linux-приложений на Rust с соблюдением стандартов GNOME Human Interface Guidelines (HIG).

---

## 🎯 Основные возможности

- **Редактор текста** в стиле GNOME Text Editor: моноширинный шрифт, мягкий перенос строк, отступы.
- **Предпросмотр Markdown в стиле GitHub**:
  - Быстрое переключение по `Ctrl+Tab` или переключателем в заголовке окна.
  - Рендеринг через WebKitGTK с оригинальной таблицей стилей `github-markdown-css`.
  - Поддержка расширений GFM (GitHub Flavored Markdown): таблицы, списки задач с чекбоксами (`- [x]`), сноски, зачеркивание, цитаты и блоки кода.
  - Автоматическая адаптация цветов под светлую и темную тему GitHub.
  - Загрузка локальных изображений (SVG, PNG, JPG, WebP) по относительным путям относительно открытого файла с передачей `base_uri`.
- **Работа с файлами**:
  - Открытие одиночного файла (`Ctrl+O` или аргументом `rmd-view file.md`).
  - Сохранение (`Ctrl+S`) и «Сохранить как...» (`Ctrl+Shift+S`).
  - Индикатор несохраненных правок (`*` в заголовке).
  - Защита от потери данных: диалог подтверждения (`adw::MessageDialog`) при попытке закрыть окно или открыть другой файл с несохраненными правками.
  - Закрытие окна и выход из приложения по `Ctrl+Q` или `Ctrl+F4`.
- **Поиск по файлу (`Ctrl+F`)**:
  - Панель поиска с подсветкой всех найденных вхождений и навигацией.
- **Настройки (`Preferences`)**:
  - Выбор темы оформления (Системная / Светлая / Тёмная).
  - Выбор гарнитуры шрифта редактора (font-family: `Monospace`, `JetBrains Mono`, `Fira Code`, `Source Code Pro` и др.).
  - Размер шрифта в редакторе (применяется на лету с приоритетом USER).
  - Размер шрифта в окне предпросмотра.
  - Сохранение настроек в формате JSON по стандарту XDG (`~/.config/rmd-view/settings.json`).

---

## 🛠️ Настройка окружения для разработки

### 1. Системные зависимости

Для компиляции требуются компилятор Rust (версии 1.75+) и заголовочные файлы библиотек GTK4, Libadwaita и WebKitGTK 6.0.

#### ALT Linux:
```bash
su -
apt-get update
apt-get install -y \
    libgtk4-devel \
    libadwaita-devel \
    libwebkitgtk6.0-devel
```

#### Fedora / RHEL:
```bash
sudo dnf install -y \
    gtk4-devel \
    libadwaita-devel \
    webkitgtk6.0-devel
```

#### Ubuntu / Debian (24.04+):
```bash
sudo apt update
sudo apt install -y \
    libgtk-4-dev \
    libadwaita-1-dev \
    libwebkitgtk-6.0-dev
```

#### Arch Linux:
```bash
sudo pacman -S --needed \
    gtk4 \
    libadwaita \
    webkitgtk-6.0
```

---

### 2. Рекомендуемые расширения для VS Code / VSCodium

1. **`rust-lang.rust-analyzer`** — автодополнение кода, проверка типов, подсказки по API GTK и компиляция на лету.
2. **`vadimcn.vscode-lldb`** — отладчик (CodeLLDB) для запуска с точками останова по клавише `F5`.
3. **`tamasfe.even-better-toml`** — подсветка и автодополнение для `Cargo.toml`.
4. **`usernamehw.errorlens`** — отображение ошибок компилятора прямо в строках редактора.

---

## 🚀 Сборка бинарного файла и запуск

### 1. Отладочная сборка (Debug)
Для быстрой компиляции и проверки во время разработки:
```bash
cargo build
```
Итоговый бинарник появится в `target/debug/rmd-view`.

### 2. Релизная оптимизированная сборка (Release)
Для получения максимальной скорости работы, отзывчивого интерфейса и минимального размера бинарника:
```bash
cargo build --release
```
Готовый исполняемый файл будет лежать в:
```bash
target/release/rmd-view
```

> **Совет по уменьшению размера бинарника (strip):**  
> Чтобы убрать отладочные символы и сделать бинарник компактнее:
> ```bash
> strip target/release/rmd-view
> ```

### 3. Установка в систему (опционально)
Чтобы запускать `rmd-view` из любой папки в терминале или ассоциировать с `.md` файлами:
```bash
# Установка в пользовательскую директорию (~/.cargo/bin)
cargo install --path .

# Либо копирование в системную директорию (от root/sudo)
sudo cp target/release/rmd-view /usr/local/bin/
```

### 4. Запуск приложения
```bash
# Запуск через cargo
cargo run -- test.md

# Прямой запуск собранного бинарника
./target/release/rmd-view test.md
```

---

## 📚 Архитектура проекта и чему здесь можно научиться

Исходный код снабжен детальными комментариями на русском языке:

- [`src/main.rs`](src/main.rs) — инициализация `adw::Application`, D-Bus ID, обработка сигналов `activate` и `open` (открытие файлов через аргументы командной строки или контекстное меню ОС).
- [`src/window.rs`](src/window.rs) — построение интерфейса:
  - Использование `adw::HeaderBar`, `adw::ViewStack`, `gtk4::SearchBar`.
  - Управление состоянием через `Rc<RefCell<AppState>>` без гонок данных.
  - Клонирование хэндлов для захвата в замыкания сигналов GTK (`connect_clicked`, `connect_changed`).
  - Регистрация действий `gio::SimpleAction` и горячих клавиш приложения.
  - Подсветка текста через `GtkTextTag` и поиск `GtkTextIter::forward_search`.
  - Диалог настроек `adw::PreferencesWindow` и динамическое применение CSS через `gtk4::CssProvider`.
- [`src/renderer.rs`](src/renderer.rs) — парсинг Markdown с флагами GFM через `pulldown-cmark` и внедрение `github-markdown-css` во встроенный HTML-шаблон.
- [`src/settings.rs`](src/settings.rs) — сохранение структуры настроек в JSON через `serde` и библиотека `dirs` для работы с XDG Base Directory.
