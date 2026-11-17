#!/usr/bin/env bash
# Aster installer: build, binary, SVG/PNG icons, desktop entry and uninstall.
#
#   ./install.sh            — interactive menu: install or uninstall
#   ./install.sh install    — install (builds when no binary is present)
#   ./install.sh rebuild    — clean rebuild and install
#   ./install.sh lite       — install the already built binary
#   ./install.sh shortcut   — create a desktop shortcut
#   ./install.sh uninstall  — remove the app, icons, menu entry and settings
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

APP_NAME="aster"
APP_TITLE="Aster"
APP_ID="io.github.aster"
APP_COMMENT_EN="View and edit Markdown documents in the GNOME style"
APP_COMMENT_RU="Просмотр и редактирование Markdown-документов в стиле GNOME"

DATA_HOME="${XDG_DATA_HOME:-${HOME}/.local/share}"
BIN_DIR="${XDG_BIN_HOME:-${HOME}/.local/bin}"
DESKTOP_DIR="${DATA_HOME}/applications"
DESKTOP_FILE="${DESKTOP_DIR}/${APP_NAME}.desktop"
ICON_THEME_DIR="${DATA_HOME}/icons/hicolor"
ICON_SCALABLE_DIR="${ICON_THEME_DIR}/scalable/apps"
ICON_SIZES=(16 24 32 48 64 128 256 512)
CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/${APP_NAME}"
ICON_SOURCE="${SCRIPT_DIR}/assets/${APP_NAME}.svg"
TARGET_BIN="${SCRIPT_DIR}/target/release/${APP_NAME}"

# A release archive ships the binary next to install.sh, while a source
# checkout keeps it in target/release.
binary_path() {
    if [ -x "${SCRIPT_DIR}/${APP_NAME}" ]; then
        echo "${SCRIPT_DIR}/${APP_NAME}"
    else
        echo "${TARGET_BIN}"
    fi
}

info() {
    echo "==> $*"
}

warn() {
    echo "!!! $*" >&2
}

die() {
    echo "Error: $*" >&2
    exit 1
}

check_deps() {
    command -v cargo >/dev/null 2>&1 \
        || die "cargo not found. Install Rust: curl https://sh.rustup.rs -sSf | sh"
    command -v pkg-config >/dev/null 2>&1 \
        || die "pkg-config not found. Install it: sudo apt install pkg-config"

    pkg-config --exists gtk4 2>/dev/null \
        || die "GTK4 development files not found. Install: sudo apt install libgtk-4-dev"
    pkg-config --atleast-version=4.10 gtk4 \
        || die "GTK >= 4.10 is required, found $(pkg-config --modversion gtk4)"

    pkg-config --exists libadwaita-1 2>/dev/null \
        || die "libadwaita development files not found. Install: sudo apt install libadwaita-1-dev"
    pkg-config --atleast-version=1.4 libadwaita-1 \
        || die "libadwaita >= 1.4 is required, found $(pkg-config --modversion libadwaita-1)"

    pkg-config --exists webkitgtk-6.0 2>/dev/null \
        || die "WebKitGTK 6.0 development files not found. Install: sudo apt install libwebkitgtk-6.0-dev"

    echo "    cargo:      $(cargo --version)"
    echo "    gtk4:       $(pkg-config --modversion gtk4)"
    echo "    libadwaita: $(pkg-config --modversion libadwaita-1)"
    echo "    webkitgtk:  $(pkg-config --modversion webkitgtk-6.0)"
}

build_app() {
    info "Building ${APP_TITLE} (release)..."
    cargo build --release --locked
}

# --- icons ------------------------------------------------------------------

# PNG icons are needed by docks, alt-tab switchers and older panels.
# Both the short name and the application ID are installed so that
# GtkApplication can find the icon by its D-Bus ID as well.
generate_png_icons() {
    local size target_dir

    if [ ! -f "${ICON_SOURCE}" ]; then
        warn "Icon not found: ${ICON_SOURCE}"
        return 0
    fi

    if command -v rsvg-convert >/dev/null 2>&1; then
        for size in "${ICON_SIZES[@]}"; do
            target_dir="${ICON_THEME_DIR}/${size}x${size}/apps"
            mkdir -p "${target_dir}"
            rsvg-convert --width "${size}" --height "${size}" \
                --output "${target_dir}/${APP_NAME}.png" "${ICON_SOURCE}"
            install -m 644 "${target_dir}/${APP_NAME}.png" "${target_dir}/${APP_ID}.png"
        done
        return 0
    fi

    if command -v python3 >/dev/null 2>&1 \
        && python3 -c "import gi; gi.require_version('Rsvg', '2.0'); from gi.repository import Rsvg; import cairo" >/dev/null 2>&1; then
        python3 - "${ICON_SOURCE}" "${ICON_THEME_DIR}" "${APP_NAME}" "${APP_ID}" <<'PYEOF'
import os
import sys

source, base, name, app_id = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]

import gi
gi.require_version("Rsvg", "2.0")
from gi.repository import Rsvg
import cairo

handle = Rsvg.Handle.new_from_file(source)

for size in (16, 24, 32, 48, 64, 128, 256, 512):
    target_dir = os.path.join(base, "%dx%d" % (size, size), "apps")
    os.makedirs(target_dir, exist_ok=True)

    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, size, size)
    cr = cairo.Context(surface)
    viewport = Rsvg.Rectangle()
    viewport.x = 0
    viewport.y = 0
    viewport.width = size
    viewport.height = size
    handle.render_document(cr, viewport)

    for icon_name in (name, app_id):
        surface.write_to_png(os.path.join(target_dir, icon_name + ".png"))
PYEOF
        return 0
    fi

    warn "rsvg-convert and PyGObject/Rsvg are both missing: only the SVG icon is installed."
    warn "Install one of them for crisp raster icons: sudo apt install librsvg2-bin"
}

refresh_icon_cache() {
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "${ICON_THEME_DIR}" 2>/dev/null || true
    fi
}

install_icons() {
    [ -f "${ICON_SOURCE}" ] || die "Icon not found: ${ICON_SOURCE}"

    info "Installing icons to ${ICON_SCALABLE_DIR}..."
    mkdir -p "${ICON_SCALABLE_DIR}"
    install -m 644 "${ICON_SOURCE}" "${ICON_SCALABLE_DIR}/${APP_NAME}.svg"
    install -m 644 "${ICON_SOURCE}" "${ICON_SCALABLE_DIR}/${APP_ID}.svg"

    generate_png_icons
    refresh_icon_cache
}

remove_icons() {
    local size target_dir

    rm -f "${ICON_SCALABLE_DIR}/${APP_NAME}.svg"
    rm -f "${ICON_SCALABLE_DIR}/${APP_ID}.svg"

    for size in "${ICON_SIZES[@]}"; do
        target_dir="${ICON_THEME_DIR}/${size}x${size}/apps"
        rm -f "${target_dir}/${APP_NAME}.png"
        rm -f "${target_dir}/${APP_ID}.png"
    done

    refresh_icon_cache
    echo "  Removed icons from ${ICON_THEME_DIR}"
}

# --- desktop entry -----------------------------------------------------------

write_desktop_entry() {
    info "Registering the application in ${DESKTOP_DIR}..."
    mkdir -p "${DESKTOP_DIR}"

    cat <<EOF > "${DESKTOP_FILE}"
[Desktop Entry]
Type=Application
Version=1.0
Name=${APP_TITLE}
GenericName=Markdown Editor
GenericName[ru]=Редактор Markdown
Comment=${APP_COMMENT_EN}
Comment[ru]=${APP_COMMENT_RU}
Exec=${BIN_DIR}/${APP_NAME} %F
TryExec=${BIN_DIR}/${APP_NAME}
Icon=${APP_NAME}
Terminal=false
Categories=Office;TextEditor;Utility;
MimeType=text/markdown;text/x-markdown;
Keywords=markdown;editor;viewer;notes;text;aster;
StartupNotify=true
StartupWMClass=${APP_ID}
EOF

    chmod 644 "${DESKTOP_FILE}"
    update_desktop_database
}

remove_desktop_entry() {
    rm -f "${DESKTOP_FILE}"
    update_desktop_database
    echo "  Removed ${DESKTOP_FILE}"
}

update_desktop_database() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${DESKTOP_DIR}" 2>/dev/null || true
    fi
}

# --- desktop shortcut --------------------------------------------------------

desktop_dir() {
    local dir
    dir="$(xdg-user-dir DESKTOP 2>/dev/null || true)"
    if [ -z "${dir}" ] || [ ! -d "${dir}" ]; then
        dir="${HOME}/Desktop"
    fi
    echo "${dir}"
}

create_shortcut() {
    local dir shortcut

    [ -f "${DESKTOP_FILE}" ] \
        || die "the application is not installed yet: run ./install.sh install"
    [ -x "${BIN_DIR}/${APP_NAME}" ] \
        || die "the application is not installed yet: run ./install.sh install"

    dir="$(desktop_dir)"
    mkdir -p "${dir}"
    shortcut="${dir}/${APP_NAME}.desktop"

    info "Creating the desktop shortcut..."
    cp -f "${DESKTOP_FILE}" "${shortcut}"
    chmod +x "${shortcut}"

    # GNOME Nautilus requires the trusted metadata flag for launchers.
    if command -v gio >/dev/null 2>&1; then
        gio set "${shortcut}" metadata::trusted true 2>/dev/null || true
    fi

    echo "  Shortcut: ${shortcut}"
}

remove_desktop_shortcut() {
    local shortcut
    shortcut="$(desktop_dir)/${APP_NAME}.desktop"

    if [ -f "${shortcut}" ]; then
        rm -f "${shortcut}"
        echo "  Removed ${shortcut}"
    fi
}

# --- install / uninstall -----------------------------------------------------

copy_files() {
    local bin
    bin="$(binary_path)"

    [ -x "${bin}" ] || die "binary not found: ${bin}"

    info "Installing the binary to ${BIN_DIR}..."
    mkdir -p "${BIN_DIR}"
    install -m 755 "${bin}" "${BIN_DIR}/${APP_NAME}"

    install_icons
    write_desktop_entry

    echo ""
    echo "✓ ${APP_TITLE} installed."
    echo "  Binary:     ${BIN_DIR}/${APP_NAME}"
    echo "  Menu entry: ${DESKTOP_FILE}"
    echo "  Icon:       ${ICON_SCALABLE_DIR}/${APP_NAME}.svg"
    echo "  Version:    $("${BIN_DIR}/${APP_NAME}" --version 2>/dev/null || echo unknown)"
    echo "  Settings:   ${CONFIG_DIR}/settings.json"
    echo ""

    case ":${PATH}:" in
        *":${BIN_DIR}:"*) ;;
        *)
            warn "${BIN_DIR} is not in PATH. Add it to your shell profile:"
            echo '    export PATH="$HOME/.local/bin:$PATH"'
            ;;
    esac
}

install_app() {
    local bin
    bin="$(binary_path)"

    if [ -x "${bin}" ]; then
        info "Using the prebuilt ${APP_NAME} binary (use \"./install.sh rebuild\" to rebuild)."
        copy_files
        return 0
    fi

    check_deps
    build_app
    copy_files
}

rebuild_app() {
    [ -f "${SCRIPT_DIR}/Cargo.toml" ] \
        || die "rebuild is only available in a source checkout (Cargo.toml not found)"

    check_deps
    info "Cleaning the Aster package artifacts..."
    cargo clean -p "${APP_NAME}"
    build_app
    copy_files
}

lite_install() {
    local bin
    bin="$(binary_path)"
    [ -x "${bin}" ] || die "no built binary found: ${bin}"
    copy_files
}

uninstall_app() {
    info "Uninstalling ${APP_TITLE}..."

    if [ -f "${BIN_DIR}/${APP_NAME}" ]; then
        rm -f "${BIN_DIR}/${APP_NAME}"
        echo "  Removed ${BIN_DIR}/${APP_NAME}"
    fi

    remove_desktop_entry
    remove_icons
    remove_desktop_shortcut

    # Settings, window geometry and the restored session live here.
    if [ -d "${CONFIG_DIR}" ]; then
        rm -rf "${CONFIG_DIR}"
        echo "  Removed settings and session: ${CONFIG_DIR}"
    fi

    echo ""
    echo "✓ ${APP_TITLE} removed completely."
}

usage() {
    cat <<EOF
Usage: $0 [command]

Without a command an interactive menu offers install or uninstall.

Commands:
  install     Install the application (builds it when no binary is present)
  rebuild     Clean, rebuild and install from the sources
  lite        Install the already built binary without rebuilding
  shortcut    Create a shortcut on the desktop
  uninstall   Remove the application, icons, menu entry, shortcut and settings

Examples:
  ./install.sh
  ./install.sh install
  ./install.sh uninstall
EOF
}

confirm_uninstall() {
    local answer

    echo ""
    echo "  This removes the binary, the menu entry, all icons, the desktop"
    echo "  shortcut and the settings/session in ${CONFIG_DIR}."
    printf "  Continue? [y/N]: "

    if ! read -r answer; then
        echo ""
        return 0
    fi

    case "${answer}" in
        y|Y|yes|YES)
            uninstall_app
            ;;
        *)
            echo "  Cancelled."
            ;;
    esac
}

interactive_menu() {
    local choice

    # Pipes and CI keep the non-interactive behaviour: just install.
    if [ ! -t 0 ]; then
        install_app
        return 0
    fi

    cat <<'EOF'

  Aster installer

    1) Install      build and install the application
    2) Rebuild      clean rebuild and install
    3) Uninstall    remove the app, icons, menu entry and settings
    q) Quit

EOF

    while true; do
        printf "  Choose [1/2/3/q]: "

        if ! read -r choice; then
            echo ""
            return 0
        fi

        case "${choice}" in
            1|i|install)
                install_app
                return 0
                ;;
            2|r|rebuild)
                rebuild_app
                return 0
                ;;
            3|u|uninstall)
                confirm_uninstall
                return 0
                ;;
            q|Q|quit|exit)
                echo "  Nothing to do."
                return 0
                ;;
            "")
                ;;
            *)
                echo "  Please choose 1, 2, 3 or q."
                ;;
        esac
    done
}

case "${1:-menu}" in
    menu)
        interactive_menu
        ;;
    install)
        install_app
        ;;
    rebuild)
        rebuild_app
        ;;
    lite)
        lite_install
        ;;
    shortcut)
        create_shortcut
        ;;
    uninstall)
        uninstall_app
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        echo "Unknown command: $1" >&2
        echo "" >&2
        usage >&2
        exit 1
        ;;
esac
