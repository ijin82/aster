#!/usr/bin/env bash
# Aster release packaging: fresh Cargo release build, strip, archive, checksum.
#
#   ./package-release.sh            — use the version from VERSION
#   ./package-release.sh v0.2.0     — explicit version (must match VERSION)
#   VERSION=v0.2.0 ./package-release.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

APP_NAME="aster"
APP_TITLE="Aster"
DIST_DIR="${SCRIPT_DIR}/dist"
CARGO_BIN="${SCRIPT_DIR}/target/release/${APP_NAME}"

# --- version ---------------------------------------------------------------

VERSION="${1:-${VERSION:-}}"
if [ -z "${VERSION}" ] && [ -f "VERSION" ]; then
    VERSION="$(xargs < VERSION)"
fi
if [ -z "${VERSION}" ]; then
    read -r -p "Enter release version (e.g. v0.2.0): " VERSION
fi

VERSION="$(echo "${VERSION}" | xargs)"
[ -n "${VERSION}" ] || { echo "Error: release version cannot be empty." >&2; exit 1; }

case "${VERSION}" in
    v*) ;;
    *) VERSION="v${VERSION}" ;;
esac

[ -f "VERSION" ] || { echo "Error: VERSION file is missing." >&2; exit 1; }

FILE_VERSION="$(xargs < VERSION)"
if [ "${FILE_VERSION}" != "${VERSION}" ]; then
    echo "Error: VERSION contains '${FILE_VERSION}', but '${VERSION}' was requested." >&2
    echo "       Update the VERSION file first — it is the single source of truth." >&2
    exit 1
fi

# Cargo.toml must agree with VERSION, otherwise the binary reports a different
# version and the release would be misleading.
CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
if [ "v${CARGO_VERSION}" != "${VERSION}" ]; then
    echo "Error: Cargo.toml version is '${CARGO_VERSION}', but VERSION is '${VERSION}'." >&2
    echo "       Update Cargo.toml so both files match." >&2
    exit 1
fi

ARCH="$(uname -m)"
RELEASE_TAG="${APP_NAME}-${VERSION}-linux-${ARCH}"
STAGE_DIR="${DIST_DIR}/${RELEASE_TAG}"
ARCHIVE_NAME="${RELEASE_TAG}.tar.gz"
ARCHIVE_PATH="${DIST_DIR}/${ARCHIVE_NAME}"

echo "=================================================="
echo " Preparing release package: ${RELEASE_TAG}"
echo "=================================================="

# --- 1. build --------------------------------------------------------------

echo "==> Building a fresh release binary..."
cargo build --release --locked

[ -f "${CARGO_BIN}" ] || { echo "Error: build failed, '${CARGO_BIN}' not found." >&2; exit 1; }

BUILT_VERSION="$("${CARGO_BIN}" --version 2>/dev/null || true)"
EXPECTED_VERSION="${APP_NAME} ${VERSION}"
if [ "${BUILT_VERSION}" != "${EXPECTED_VERSION}" ]; then
    echo "Error: the binary reports '${BUILT_VERSION}', expected '${EXPECTED_VERSION}'." >&2
    exit 1
fi

# --- 2. staging ------------------------------------------------------------

echo "==> Preparing the staging directory..."
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}/assets"

copy_if_exists() {
    if [ -e "$1" ]; then
        cp -rp "$1" "${STAGE_DIR}/$(basename "$1")"
        echo "    + $(basename "$1")"
    fi
}

echo "==> Copying release files..."
cp -p "${CARGO_BIN}" "${STAGE_DIR}/${APP_NAME}"

# Strip the binary to keep the archive small.
if command -v strip >/dev/null 2>&1; then
    echo "==> Stripping debug symbols..."
    strip --strip-unneeded "${STAGE_DIR}/${APP_NAME}" 2>/dev/null || true
fi

cp -p "install.sh" "${STAGE_DIR}/install.sh"
chmod 755 "${STAGE_DIR}/install.sh"
echo "    + install.sh"

cp -p "assets/${APP_NAME}.svg" "${STAGE_DIR}/assets/${APP_NAME}.svg"
echo "    + assets/${APP_NAME}.svg"

copy_if_exists "README.md"
copy_if_exists "LICENSE"
copy_if_exists "VERSION"

# --- 3. archive ------------------------------------------------------------

echo "==> Creating ${ARCHIVE_NAME}..."
mkdir -p "${DIST_DIR}"
tar -czf "${ARCHIVE_PATH}" -C "${DIST_DIR}" "${RELEASE_TAG}"

if command -v sha256sum >/dev/null 2>&1; then
    (cd "${DIST_DIR}" && sha256sum "${ARCHIVE_NAME}" > "${ARCHIVE_NAME}.sha256")
fi

rm -rf "${STAGE_DIR}"

# --- 4. summary ------------------------------------------------------------

echo ""
echo "=================================================="
echo "✓ Release package completed."
echo "=================================================="
echo "  Archive:  ${ARCHIVE_PATH}"
echo "  Size:     $(du -h "${ARCHIVE_PATH}" | cut -f1)"
if [ -f "${ARCHIVE_PATH}.sha256" ]; then
    echo "  SHA256:   $(cut -d' ' -f1 < "${ARCHIVE_PATH}.sha256")"
fi
echo ""
echo "Archive contents:"
tar -ztvf "${ARCHIVE_PATH}"
echo ""
echo "To test the package:"
echo "  tar -xzf ${ARCHIVE_PATH}"
echo "  cd ${RELEASE_TAG}"
echo "  ./install.sh install     # binary, icons, menu entry"
echo "  ./install.sh shortcut    # optional desktop shortcut"
echo "  ./install.sh uninstall   # removes everything, including settings"
echo "=================================================="
