#!/usr/bin/env bash
# Aster release packaging: fresh Cargo release build, strip, archive, checksum.
#
#   ./package-release.sh            — use the version from VERSION
#   ./package-release.sh v0.3.0     — set this version in VERSION/Cargo.toml
#   VERSION=v0.3.0 ./package-release.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

APP_NAME="aster"
APP_TITLE="Aster"
DIST_DIR="${SCRIPT_DIR}/dist"
CARGO_BIN="${SCRIPT_DIR}/target/release/${APP_NAME}"

[ -f "Cargo.toml" ] || {
    echo "Error: Cargo.toml not found. Run this script from a source checkout." >&2
    exit 1
}

# --- version ---------------------------------------------------------------

# Explicit version (argument or environment) is the source of truth:
# the script itself updates VERSION and Cargo.toml, so a release can be
# prepared with a single command and no manual edits.
VERSION_SPECIFIED=false
if [ -n "${1:-}" ]; then
    VERSION="$1"
    VERSION_SPECIFIED=true
elif [ -n "${VERSION:-}" ]; then
    VERSION_SPECIFIED=true
elif [ -f "VERSION" ]; then
    VERSION="$(xargs < VERSION)"
else
    read -r -p "Enter release version (e.g. v0.3.0): " VERSION
    VERSION_SPECIFIED=true
fi

VERSION="$(echo "${VERSION}" | xargs)"
[ -n "${VERSION}" ] || { echo "Error: release version cannot be empty." >&2; exit 1; }

case "${VERSION}" in
    v*) ;;
    *) VERSION="v${VERSION}" ;;
esac

if [[ ! "${VERSION}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
    echo "Error: '${VERSION}' is not a valid release version." >&2
    echo "       Expected a semantic version such as v0.3.0 or v1.0.0-rc1." >&2
    exit 1
fi

BARE_VERSION="${VERSION#v}"

# Update the two version sources automatically when a version was passed.
if [ "${VERSION_SPECIFIED}" = true ]; then
    echo "==> Setting project version to ${VERSION}..."

    printf '%s\n' "${VERSION}" > VERSION
    sed -i "0,/^version = \"[^\"]*\"/s//version = \"${BARE_VERSION}\"/" Cargo.toml

    FILE_VERSION="$(xargs < VERSION)"
    CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
    if [ "${FILE_VERSION}" != "${VERSION}" ] || [ "${CARGO_VERSION}" != "${BARE_VERSION}" ]; then
        echo "Error: failed to synchronize VERSION and Cargo.toml." >&2
        echo "       VERSION=${FILE_VERSION}, Cargo.toml=${CARGO_VERSION}" >&2
        exit 1
    fi
else
    # No explicit version: only verify that the existing files agree.
    [ -f "VERSION" ] || { echo "Error: VERSION file is missing." >&2; exit 1; }

    FILE_VERSION="$(xargs < VERSION)"
    CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
    if [ "${FILE_VERSION}" != "${VERSION}" ] || [ "v${CARGO_VERSION}" != "${VERSION}" ]; then
        echo "Error: VERSION and Cargo.toml do not match." >&2
        echo "       VERSION=${FILE_VERSION}, Cargo.toml=v${CARGO_VERSION}" >&2
        echo "       Pass the release version explicitly, e.g. ./package-release.sh v0.3.0," >&2
        echo "       and the script will update both files automatically." >&2
        exit 1
    fi
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
if [ "${VERSION_SPECIFIED}" = true ]; then
    # Cargo.lock stores the version of the root package, so it must be
    # refreshed after changing Cargo.toml. A plain build updates only that
    # entry and keeps all dependency versions pinned by the existing lock.
    cargo build --release
else
    cargo build --release --locked
fi

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
