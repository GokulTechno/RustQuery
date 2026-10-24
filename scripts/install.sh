#!/usr/bin/env bash
#
# Install RustQuery from a local checkout.
#
# Builds the release binary and installs it with the desktop launcher, the
# scalable and PNG icons, AppStream metadata, the man page and documentation.
# The same script removes an installation again with --uninstall.
#
# Usage:
#   scripts/install.sh                  # current user, ~/.local
#   sudo scripts/install.sh --system    # system-wide, /usr/local
#   scripts/install.sh --prefix DIR
#   scripts/install.sh --uninstall
#   scripts/install.sh --help
#
# No root privileges are required for the default per-user installation.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

readonly PKG_NAME="rustquery"

PREFIX="${PREFIX:-}"
DESTDIR="${DESTDIR:-}"
DO_BUILD=1
UNINSTALL=0
PREFIX_SET=0
REMOVED=0

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

note() {
    printf '==> %s\n' "$*"
}

warn() {
    printf 'warning: %s\n' "$*" >&2
}

usage() {
    cat <<'EOF'
Install or uninstall RustQuery from this checkout.

USAGE:
    scripts/install.sh [OPTIONS]

By default the release binary is built and installed for the current user in
~/.local. When run as root, the default prefix is /usr/local.

OPTIONS:
    --prefix DIR          Install under DIR (default: ~/.local, or /usr/local
                          for root)
    --system              Shortcut for --prefix /usr/local
    --user                Shortcut for --prefix $HOME/.local
    --destdir DIR         Stage the installation under DIR (for packaging);
                          cache refresh is skipped
    --no-build            Reuse target/release/rustquery instead of building
    --uninstall           Remove the files installed with the same prefix
    -h, --help            Show this help

INSTALLED FILES:
    <prefix>/bin/rustquery
    <prefix>/share/applications/rustquery.desktop
    <prefix>/share/icons/hicolor/.../apps/rustquery.{svg,png}
    <prefix>/share/metainfo/io.github.gokultechno.RustQuery.metainfo.xml
    <prefix>/share/man/man1/rustquery.1.gz
    <prefix>/share/doc/rustquery/{README.md,LICENSE}

The launcher's Exec entry points at the installed binary, so the application
works even when <prefix>/bin is not on PATH. User configuration in
~/.config/rustquery is never removed by --uninstall.
EOF
}

while (($#)); do
    case "$1" in
        --prefix)
            (($# >= 2)) || die "--prefix requires a directory"
            PREFIX="$2"
            PREFIX_SET=1
            shift
            ;;
        --prefix=*)
            PREFIX="${1#*=}"
            PREFIX_SET=1
            ;;
        --system)
            PREFIX="/usr/local"
            PREFIX_SET=1
            ;;
        --user)
            [[ -n "${HOME:-}" ]] || die "HOME is not set; pass --prefix instead"
            PREFIX="$HOME/.local"
            PREFIX_SET=1
            ;;
        --destdir)
            (($# >= 2)) || die "--destdir requires a directory"
            DESTDIR="$2"
            shift
            ;;
        --destdir=*)
            DESTDIR="${1#*=}"
            ;;
        --no-build)
            DO_BUILD=0
            ;;
        --uninstall)
            UNINSTALL=1
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "unknown option '$1' (try --help)"
            ;;
    esac
    shift
done

if ((PREFIX_SET == 0)) && [[ -z "$PREFIX" ]]; then
    if ((EUID == 0)); then
        PREFIX="/usr/local"
    else
        [[ -n "${HOME:-}" ]] || die "HOME is not set; pass --prefix"
        PREFIX="$HOME/.local"
    fi
fi

PREFIX="${PREFIX%/}"
[[ -n "$PREFIX" ]] || PREFIX="/"
[[ "$PREFIX" == /* ]] || die "prefix must be an absolute path: $PREFIX"
[[ "$PREFIX" != "/" ]] ||
    die "refusing to install directly into /; use --prefix /usr/local"
[[ "$PREFIX" != *[[:space:]]* ]] ||
    die "prefix paths containing whitespace are not supported"

DESTDIR="${DESTDIR%/}"
INSTALL_ROOT="${DESTDIR}${PREFIX}"

refresh_caches() {
    [[ -n "$DESTDIR" ]] && return 0

    if command -v update-desktop-database >/dev/null 2>&1 &&
        [[ -d "$INSTALL_ROOT/share/applications" ]]; then
        update-desktop-database -q "$INSTALL_ROOT/share/applications" 2>/dev/null || true
    fi

    # A per-user hicolor directory usually has no index.theme; the icon theme
    # still finds the files, so only refresh a cache that already exists.
    if command -v gtk-update-icon-cache >/dev/null 2>&1 &&
        [[ -f "$INSTALL_ROOT/share/icons/hicolor/index.theme" ]]; then
        gtk-update-icon-cache -q -t -f "$INSTALL_ROOT/share/icons/hicolor" 2>/dev/null || true
    fi
}

remove_path() {
    local path="$1"
    if [[ -e "$path" || -L "$path" ]]; then
        rm -f -- "$path"
        note "Removed $path"
        REMOVED=$((REMOVED + 1))
    fi
}

uninstall() {
    note "Removing RustQuery from $INSTALL_ROOT"

    remove_path "$INSTALL_ROOT/bin/$PKG_NAME"
    remove_path "$INSTALL_ROOT/share/applications/$PKG_NAME.desktop"
    remove_path "$INSTALL_ROOT/share/metainfo/io.github.gokultechno.RustQuery.metainfo.xml"
    remove_path "$INSTALL_ROOT/share/man/man1/$PKG_NAME.1.gz"
    remove_path "$INSTALL_ROOT/share/man/man1/$PKG_NAME.1"
    remove_path "$INSTALL_ROOT/share/doc/$PKG_NAME/README.md"
    remove_path "$INSTALL_ROOT/share/doc/$PKG_NAME/LICENSE"

    if [[ -d "$ROOT_DIR/packaging/icons" ]]; then
        while IFS= read -r -d '' icon; do
            relative="${icon#"$ROOT_DIR/packaging/icons/"}"
            remove_path "$INSTALL_ROOT/share/icons/$relative"
        done < <(find "$ROOT_DIR/packaging/icons" -type f -print0 | sort -z)
    fi

    # Remove only directories that became empty; shared parents are left alone.
    rmdir "$INSTALL_ROOT/share/doc/$PKG_NAME" 2>/dev/null || true
    rmdir "$INSTALL_ROOT/share/icons/hicolor/scalable/apps" 2>/dev/null || true
    if [[ -d "$ROOT_DIR/packaging/icons/hicolor" ]]; then
        while IFS= read -r -d '' directory; do
            relative="${directory#"$ROOT_DIR/packaging/icons/"}"
            rmdir "$INSTALL_ROOT/share/icons/$relative" 2>/dev/null || true
        done < <(find "$ROOT_DIR/packaging/icons/hicolor" -mindepth 1 -type d -print0 | sort -zr)
    fi
    rmdir "$INSTALL_ROOT/share/man/man1" 2>/dev/null || true
    rmdir "$INSTALL_ROOT/share/applications" 2>/dev/null || true
    rmdir "$INSTALL_ROOT/share/metainfo" 2>/dev/null || true
    rmdir "$INSTALL_ROOT/bin" 2>/dev/null || true

    refresh_caches

    if ((REMOVED == 0)); then
        note "No installed files were found under $INSTALL_ROOT"
    else
        note "Removed $REMOVED file(s)"
    fi
    if [[ -z "$DESTDIR" ]]; then
        note "Configuration in ~/.config/rustquery was left untouched"
    fi
}

if ((UNINSTALL)); then
    uninstall
    exit 0
fi

if ((DO_BUILD)); then
    command -v cargo >/dev/null 2>&1 ||
        die "cargo was not found; install Rust or pass --no-build"
    note "Building the release binary"
    cargo build --release --locked
fi

BINARY="$ROOT_DIR/target/release/$PKG_NAME"
[[ -x "$BINARY" ]] ||
    die "release binary not found at $BINARY (run without --no-build)"

for asset in \
    README.md \
    LICENSE \
    packaging/rustquery.1 \
    packaging/rustquery.desktop \
    packaging/io.github.gokultechno.RustQuery.metainfo.xml \
    packaging/icons/hicolor/scalable/apps/rustquery.svg; do
    [[ -f "$ROOT_DIR/$asset" ]] || die "missing installation asset: $asset"
done
[[ -d "$ROOT_DIR/packaging/icons/hicolor" ]] ||
    die "missing icon directory: packaging/icons/hicolor"

command -v install >/dev/null 2>&1 || die "the 'install' utility was not found"
command -v gzip >/dev/null 2>&1 || die "gzip was not found"

if ! mkdir -p "$INSTALL_ROOT" 2>/dev/null; then
    die "cannot create $INSTALL_ROOT; run with sudo or choose another --prefix"
fi
[[ -w "$INSTALL_ROOT" ]] ||
    die "cannot write to $INSTALL_ROOT; run with sudo or choose another --prefix"

note "Installing RustQuery into $INSTALL_ROOT"

install -D -m 0755 -p "$BINARY" "$INSTALL_ROOT/bin/$PKG_NAME"
install -D -m 0644 -p "$ROOT_DIR/README.md" "$INSTALL_ROOT/share/doc/$PKG_NAME/README.md"
install -D -m 0644 -p "$ROOT_DIR/LICENSE" "$INSTALL_ROOT/share/doc/$PKG_NAME/LICENSE"
install -D -m 0644 -p "$ROOT_DIR/packaging/io.github.gokultechno.RustQuery.metainfo.xml" \
    "$INSTALL_ROOT/share/metainfo/io.github.gokultechno.RustQuery.metainfo.xml"

install -d "$INSTALL_ROOT/share/man/man1"
gzip -9n -c "$ROOT_DIR/packaging/rustquery.1" >"$INSTALL_ROOT/share/man/man1/$PKG_NAME.1.gz"
chmod 0644 "$INSTALL_ROOT/share/man/man1/$PKG_NAME.1.gz"

install -d "$INSTALL_ROOT/share/applications"
desktop_exec="$PREFIX/bin/$PKG_NAME"
# A literal percent sign must be doubled in a desktop Exec key.
desktop_exec="${desktop_exec//%/%%}"
desktop_exec="$(printf '%s' "$desktop_exec" | sed -e 's/[\\&#]/\\&/g')"
sed "s#^Exec=.*#Exec=$desktop_exec#" "$ROOT_DIR/packaging/rustquery.desktop" \
    >"$INSTALL_ROOT/share/applications/$PKG_NAME.desktop"
chmod 0644 "$INSTALL_ROOT/share/applications/$PKG_NAME.desktop"

while IFS= read -r -d '' icon; do
    relative="${icon#"$ROOT_DIR/packaging/icons/"}"
    install -D -m 0644 -p "$icon" "$INSTALL_ROOT/share/icons/$relative"
done < <(find "$ROOT_DIR/packaging/icons" -type f -print0 | sort -z)

refresh_caches

note "Installed RustQuery to $INSTALL_ROOT"
note "Start it from the application menu or with $PREFIX/bin/$PKG_NAME"

if [[ -z "$DESTDIR" ]]; then
    case ":$PATH:" in
        *":$PREFIX/bin:"*) ;;
        *) warn "$PREFIX/bin is not on PATH; use the full path or add it to PATH" ;;
    esac
    note "Uninstall with: $0 --uninstall --prefix $PREFIX"
fi
