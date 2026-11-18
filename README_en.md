# Aster ✳️

![Aster icon](assets/aster.svg)

**English** · [Русский](README.md)

**Aster** is an educational, lightweight application built with **Rust**, **GTK4**, **Libadwaita**, and **WebKitGTK 6.0** for viewing and editing Markdown documents in the GNOME style.

[Another file](AGENTS.md)

The main goal of the project is to serve as a clear, practical guide to building Linux GUI applications in Rust while following the GNOME Human Interface Guidelines (HIG).

---

## 🎯 Key features

- **Text editor** in the style of GNOME Text Editor:
  - **Line numbers**: a left gutter with line numbers, smoothly synchronized with scrolling and dynamically adapting its width and color to the selected theme.
  - **Cursor position memory**: when a file is reopened, the cursor is automatically restored to the line and column where it was before closing; for new and previously unopened files, the cursor is placed at the beginning of the document `(0, 0)`, eliminating delays ("freezing") when opening large files.
  - **Independent scroll memory for each mode**: the editor and preview remember their own vertical scroll positions. Switching modes no longer performs fragile recalculation of HTML element coordinates — each mode simply returns to where the user left it. Positions are saved per file and survive application restarts.
  - **Completely silent operation**: GTK's `gtk-error-bell`, event sounds (`gtk-enable-event-sounds`), and input feedback sounds (`gtk-enable-input-feedback-sounds`) are disabled. In addition, media, Web Audio, media streams, and EME are disabled in WebKit, so the Markdown preview cannot play sound.
  - Live Markdown syntax highlighting via GTK tags (`src/highlighter.rs`): headings, bold/italic text, inline code, code blocks, lists, blockquotes, and links.
  - Full color scheme support in the editor (background, text color, caret color, and highlight token colors match the selected theme).
  - Configurable monospace font family, soft line wrapping, and padding.
  - **Quick line indentation (`Tab` / `Shift+Tab`)**: when text is selected, pressing `Tab` adds two spaces to the beginning of each selected line, while `Shift+Tab` removes up to two spaces from the beginning of the lines (preserving selection boundaries and supporting undo with `Ctrl+Z`).
  - **Duplicate line `Alt+D`**: a copy of the line under the cursor is inserted below, and the cursor moves to the copy in the same column.
  - **Move to the beginning and end of the document (`Ctrl+Home` / `Ctrl+End`)**: the cursor moves to the start or end of the current file, and the editor scrolls to the corresponding edge without horizontal shifting.
  - **Meditation mode `Ctrl+Shift+H`**: hides the HeaderBar with menus and buttons, leaving only the text. If a single tab is open, the editor fills the entire window; pressing the shortcut again restores the panel.
- **GitHub-style Markdown preview**:
  - Quick switching with `Ctrl+E` (like in Obsidian) or with the toggle in the window header.
  - **Per-mode scroll memory**: the editor and preview keep separate scroll positions, so switching does not "jump" to a random place in the document.
  - Rendering through WebKitGTK with the original `github-markdown-css` stylesheet.
  - Support for GFM (GitHub Flavored Markdown) extensions: tables, task lists with checkboxes (`- [x]`), footnotes, strikethrough, blockquotes, and code blocks.
  - Automatic color adaptation for GitHub light and dark themes.
  - Loading local images (SVG, PNG, JPG, WebP) via relative paths based on the open file, using `base_uri`.
  - **Safe preview**: Content Security Policy blocks scripts and network requests from Markdown; external links open in the system browser; internal anchors (including heading anchors) scroll within the page; and the WebView itself cannot navigate away from the current document. Access to arbitrary `file://` URLs is disabled.
- **Files and tabs**:
  - **Document tabs** based on `adw::TabView`: opening multiple Markdown files at once, dragging tabs, a context menu, closing with `Ctrl+W`, and creating a new tab with `Ctrl+T`. Cyclic tab switching uses `Ctrl+Tab` / `Ctrl+Shift+Tab`; from the last tab it goes to the first and vice versa. `Ctrl+Shift+T` reopens the most recently closed file in the current session (a stack of up to 50 tabs, similar to Chrome). The tab bar is automatically hidden when only one document is open.
  - **Session restore**: when launched normally without arguments, the application opens all tabs that were open in the previous session, including cursor positions, scroll positions, and the active tab (similar to GNOME Text Editor). Unsaved untitled tabs are not persisted because their contents are not associated with a file on disk.
  - **Opening one or more files** (`Ctrl+O`, the `aster file.md` argument, a file manager, or D-Bus). The open dialog starts in the directory of the active document or the last opened file. Reopening an already open file switches to the existing tab instead of creating a duplicate.
  - Saving (`Ctrl+S`) and "Save As..." (`Ctrl+Shift+S`).
  - **Unsaved changes indication** following GNOME HIG:
    - A subtle `●` dot before the file name in the window title (`● document.md — Aster`).
    - The "Save" button is highlighted with the accent `.suggested-action` style (blue button) while there are unsaved changes.
    - Dynamic button tooltip: *"Save (Ctrl+S) — Unsaved changes"* / *"Сохранить (Ctrl+S) — Есть несохраненные изменения"*.
  - Data loss protection: a confirmation dialog (`adw::MessageDialog`) when closing a tab or window with unsaved changes. When closing the window, a "Save All" button is available.
  - Close a tab with `Ctrl+W`; close the window and exit the application with `Ctrl+Q` or `Ctrl+F4`.
  - **Window state and size memory**: saves the maximized state and exact dimensions (width/height).
- **Reliability**:
  - **Reaction to external file changes**: a `gio::FileMonitor` watches every open file. If the file changes externally and the document has no unsaved edits, Aster automatically reloads it and shows a notification. If there are unsaved edits, a toast "File changed on disk" appears with a "Reload" button — editor contents are never silently lost.
  - **Recovery of untitled tabs after a crash**: the contents of unsaved untitled tabs are saved to `~/.local/share/aster/recovery.json` with a one-second debounce after the last edit. After an abnormal exit, drafts are restored on the next launch as modified untitled tabs. On a normal application exit, the recovery file is deleted.
- **Search and replace (`Ctrl+F`, `Ctrl+R`)**:
  - Search bar with highlighting of all occurrences; the current match is highlighted with higher contrast.
  - An `N of M` counter, previous/next buttons, `Enter` / `Shift+Enter`, and cyclic navigation.
  - **Preview search**: `Ctrl+F` searches the visible text of the rendered HTML, highlights all occurrences, shows the current one, and scrolls to it.
  - **Substring replacement**: `Ctrl+R` opens replace mode in the editor. A "Replace with..." field and "Replace" / "Replace All" buttons. Regular replace moves to the next match; "Replace All" is undone with a single `Ctrl+Z` and is performed as one buffer operation, without freezing the editor on large files.
  - With a very large number of matches (more than 200), the first 200 are highlighted for responsiveness; the counter and navigation work over the full list, and the current match is always highlighted.
  - Closing the panel removes the highlighting but keeps the query; reopening the panel restores the results.
- **Preferences (`Ctrl+,`)**:
  - Opened with the global `Ctrl+,` shortcut; application shortcuts work in any keyboard layout, including Russian, because they are determined by hardware keycode rather than the current keyval.
  - **Interface language selection**: switching between **Russian** and **English** with instant UI updates and persistence in settings.
  - Theme selection: **System**, **GitHub Light**, **GitHub Dark**, **Dracula**, **Kimbie Dark**, **Solarized Dark**, **Solarized Light** (applied to both the preview and the code editor).
  - Preview side padding adjustment (padding in %, 5% on each side by default).
  - **Tab width** (1–8, default 2) — how many monospace cells an existing `\t` character occupies when displayed.
  - **Indent size** (1–8, default 2) — how many spaces `Tab` and `Shift+Tab` insert. Only spaces are inserted, never tab characters.
  - **Soft wrap** (enabled by default) — long lines wrap to the window width.
  - **Show spaces and tabs** (enabled by default) — spaces are marked with dots, tabs with horizontal lines.
  - **Selection from all monospace fonts on the system** (dynamically discovered via Pango Context: `Cascadia Code`, `Fira Code`, `JetBrains Mono`, `Adwaita Mono`, `DejaVu Sans Mono`, `Liberation Mono`, etc.).
  - Editor font size (applied on the fly to existing and newly entered text via the CSS rule `textview.editor-view text`).
  - Preview font size.
  - **Line number visibility toggle** in edit mode (`adw::SwitchRow`).
  - Settings are stored as JSON following the XDG standard (`~/.config/aster/settings.json`).

---

## 🛠️ Development environment setup

### 1. System dependencies

Compilation requires the Rust compiler (version 1.75+) and the development headers for GTK4, Libadwaita, and WebKitGTK 6.0.

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

### 2. Recommended VS Code / VSCodium extensions

1. **`rust-lang.rust-analyzer`** — code completion, type checking, GTK API hints, and on-the-fly compilation.
2. **`vadimcn.vscode-lldb`** — debugger (CodeLLDB) for running with breakpoints via `F5`.
3. **`tamasfe.even-better-toml`** — highlighting and completion for `Cargo.toml`.
4. **`usernamehw.errorlens`** — displays compiler errors directly in the editor lines.

---

## 🚀 Building and running the binary

### 1. Debug build
For quick compilation and checks during development:
```bash
cargo build
```
The resulting binary will appear in `target/debug/aster`.

### 2. Optimized release build
For maximum performance, a responsive interface, and the smallest binary size:
```bash
cargo build --release
```
The finished executable will be located at:
```bash
target/release/aster
```

> **Tip for reducing binary size (strip):**  
> To remove debug symbols and make the binary more compact:
> ```bash
> strip target/release/aster
> ```

### 3. Installation via `install.sh`

The script builds the release binary, installs it to `~/.local/bin`, installs icons to
`~/.local/share/icons/hicolor`, adds a desktop entry to `~/.local/share/applications`,
and can completely remove the application:

```bash
# Interactive menu (install / rebuild / uninstall)
./install.sh

# Install (runs cargo build --release if the binary is missing)
./install.sh install

# Clean rebuild and install
./install.sh rebuild

# Install an already built binary without rebuilding
./install.sh lite

# Desktop shortcut
./install.sh shortcut

# Full removal: binary, icons, menu entry, settings, and session
./install.sh uninstall
```

To install icons in PNG formats, the script uses `rsvg-convert` or Python bindings for PyGObject/Rsvg. If neither is available, only the SVG icon is installed.

### 4. Building the release archive

```bash
# Prepare a v0.7.0 release with one command
./package-release.sh v0.7.0

# Without an argument, the current version from VERSION is used
./package-release.sh
```

If a version is passed as an argument (or through the `VERSION` environment variable), the script updates `VERSION`, `Cargo.toml`, and `Cargo.lock` itself — there is no need to edit the files manually. It then checks the output of `aster --version`, builds the release, strips debug symbols, packages the binary, `install.sh`, icon, README, English README, and license into `dist/aster-vX.Y.Z-linux-<arch>.tar.gz`, and creates a `.sha256` file.

After preparing a release, do not forget to commit the updated `VERSION`, `Cargo.toml`, and `Cargo.lock`.

### 5. Running the application

```bash
# Run via cargo
cargo run -- test.md

# Run the built binary directly
./target/release/aster test.md

# Check the version without launching the GUI
./target/release/aster --version
```

When launched from a terminal, Aster behaves like VS Code's `code` command:
the shell prompt returns immediately while the GUI keeps running in a separate
process. GTK output is written to `~/.cache/aster/aster.log`. To keep the
process in the foreground and see logs in the terminal:

```bash
ASTER_FOREGROUND=1 aster test.md
```

---

## 🗺️ Roadmap

The current backlog of ideas and improvements is maintained in [ROADMAP.md](ROADMAP.md): reliability, architecture, search, performance, and UX.

---

## 📚 Project architecture and what you can learn here

The source code is thoroughly commented in Russian:

- [`src/main.rs`](src/main.rs) — initialization of `adw::Application`, the D-Bus ID, and handling of the `activate` and `open` signals (opening files via command-line arguments or the OS context menu).
- [`src/window.rs`](src/window.rs) — window composition, `adw::HeaderBar`, `adw::TabView`, `adw::TabBar`, and action registration:
  - Cloning handles for capture in GTK signal closures (`connect_clicked`, `connect_changed`).
  - Registering `gio::SimpleAction` actions and application keyboard shortcuts.
  - Wiring together independent window subsystems.
- [`src/window/`](src/window/) — extracted window parts:
  - `style.rs` — themes, editor CSS, fonts, sound disabling;
  - `menus.rs` — main menu and tab menu;
  - `tabs.rs` — active tab and `Ctrl+Tab` wrapping;
  - `dialogs.rs` — saving, unsaved-changes dialogs, and "About";
  - `preferences.rs` — `adw::PreferencesWindow`;
  - `session.rs` — session and window geometry persistence;
  - `actions.rs` — registration of `win.*` actions and their dependencies.
- [`src/document/mod.rs`](src/document/mod.rs) — model and state of a single tab:
  - Per-file `GtkTextBuffer`, `GtkTextView`, `WebKitWebView`, and `adw::ViewStack`.
  - Loading/saving, editor/preview modes, tab title, themes, and session state.
- [`src/document/search.rs`](src/document/search.rs) — search, replace, and visible-HTML search.
- [`src/document/editor.rs`](src/document/editor.rs) — `Tab`/`Shift+Tab`, line duplication, keyboard boundaries, and cursor restoration.
- [`src/document/gutter.rs`](src/document/gutter.rs) — line numbers.
- [`src/document/whitespace.rs`](src/document/whitespace.rs) — whitespace and tab markers drawn over the editor.
- [`src/document/links.rs`](src/document/links.rs) — preview link policy.
- [`src/document/tests.rs`](src/document/tests.rs) — document tests.
- [`src/highlighter.rs`](src/highlighter.rs) — built-in Markdown syntax highlighting for `GtkTextView` by creating and applying `GtkTextTag` (headings, bold/italic, code, links, blockquotes, lists) with palettes for each theme.
- [`src/i18n.rs`](src/i18n.rs) — typed internationalization and UI translations (Russian and English).
- [`src/renderer.rs`](src/renderer.rs) — Markdown parsing with GFM flags via `pulldown-cmark` and injection of `github-markdown-css` into the embedded HTML template.
- [`src/shortcuts.rs`](src/shortcuts.rs) — layout-independent mapping of hardware keycodes to Latin shortcuts (Wayland/X11 fallback).
- [`src/recovery.rs`](src/recovery.rs) — crash-time persistence and restoration of untitled tab drafts.
- [`src/settings.rs`](src/settings.rs) — persistence of settings structure, window geometry, scroll positions, and the tab list as JSON via `serde` and the `dirs` crate for XDG Base Directory support.
- [`install.sh`](install.sh) — application installation and removal: build, `~/.local/bin`, SVG/PNG icons, desktop entry, and desktop shortcut.
- [`package-release.sh`](package-release.sh) — version check, release build, `strip`, `tar.gz`, and SHA-256 checksum.
- [`Makefile`](Makefile) — short commands `make build`, `make test`, `make install`, `make package`.
- [`assets/aster.svg`](assets/aster.svg) — the original Aster vector icon: a Markdown document sheet with an amber asterisk. `install.sh` installs the SVG and generated PNG sizes (16–512 px).
