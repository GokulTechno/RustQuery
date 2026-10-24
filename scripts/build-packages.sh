#!/usr/bin/env bash
#
# Build Debian and RPM packages for RustQuery from a locally built release
# binary. No root privileges are required; packages are written to ./dist.
# Both formats install the desktop launcher, scalable icon, AppStream metadata,
# man page, README and license alongside the binary.
#
# Usage:
#   scripts/build-packages.sh                 # .deb and .rpm
#   scripts/build-packages.sh --deb
#   scripts/build-packages.sh --rpm --no-build
#   scripts/build-packages.sh --help
#
# Debian packages need dpkg-deb (and optionally dpkg-shlibdeps from dpkg-dev
# for precise shared-library dependencies). RPM packages need rpmbuild, which
# is provided by the "rpm-build" package on Fedora/RHEL and the "rpm" package
# on Debian/Ubuntu.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

readonly PKG_NAME="rustquery"

VERSION=""
RELEASE="1"
OUT_DIR="$ROOT_DIR/dist"
TARGET=""
DEB_ARCH=""
RPM_ARCH=""
DO_BUILD=1
MAINTAINER="${PACKAGE_MAINTAINER:-}"
if [[ -z "$MAINTAINER" ]]; then
    maintainer_name="${DEBFULLNAME:-$(git config --get user.name 2>/dev/null || true)}"
    maintainer_email="${DEBEMAIL:-$(git config --get user.email 2>/dev/null || true)}"
    if [[ -n "$maintainer_name" && -n "$maintainer_email" ]]; then
        MAINTAINER="$maintainer_name <$maintainer_email>"
    else
        MAINTAINER="RustQuery contributors <rustquery@example.com>"
    fi
fi
FORMATS=()
ARTIFACTS=()

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
Build Debian and RPM packages for RustQuery.

USAGE:
    scripts/build-packages.sh [OPTIONS]

By default both a .deb and an .rpm are built.

OPTIONS:
    --deb                 Build only the Debian package
    --rpm                 Build only the RPM package
    --all                 Build both formats (the default)
    --out-dir DIR         Write packages to DIR (default: ./dist)
    --version VERSION     Package version (default: version in Cargo.toml)
    --release N           Package release number (default: 1)
    --target TRIPLE       Build with `cargo build --release --target TRIPLE`
    --deb-arch ARCH       Override the Debian architecture (e.g. amd64)
    --rpm-arch ARCH       Override the RPM architecture (e.g. x86_64)
    --no-build            Use an existing target/.../release/rustquery
    --maintainer TEXT     Maintainer field (default: $PACKAGE_MAINTAINER,
                          $DEBFULLNAME/$DEBEMAIL, git identity, or a generic
                          RustQuery contributor address)
    -h, --help            Show this help

OUTPUT:
    dist/rustquery_<version>-<release>_<deb-arch>.deb
    dist/rustquery-<version>-<release>[.<dist>].<rpm-arch>.rpm

The packages contain the desktop binary, a launcher entry, a scalable icon,
AppStream metadata, a man page, the README and the MIT license. SQLite support
is embedded; PostgreSQL and MySQL clients need a reachable database server at
runtime, but no build-time database service is required.
EOF
}

while (($# > 0)); do
    case "$1" in
        --deb)
            FORMATS+=("deb")
            ;;
        --rpm)
            FORMATS+=("rpm")
            ;;
        --all)
            FORMATS=("deb" "rpm")
            ;;
        --out-dir)
            (($# >= 2)) || die "--out-dir requires a directory"
            OUT_DIR="$2"
            shift
            ;;
        --version)
            (($# >= 2)) || die "--version requires a version string"
            VERSION="$2"
            shift
            ;;
        --release)
            (($# >= 2)) || die "--release requires a number"
            RELEASE="$2"
            shift
            ;;
        --target)
            (($# >= 2)) || die "--target requires a Rust target triple"
            TARGET="$2"
            shift
            ;;
        --deb-arch)
            (($# >= 2)) || die "--deb-arch requires an architecture"
            DEB_ARCH="$2"
            shift
            ;;
        --rpm-arch)
            (($# >= 2)) || die "--rpm-arch requires an architecture"
            RPM_ARCH="$2"
            shift
            ;;
        --no-build)
            DO_BUILD=0
            ;;
        --maintainer)
            (($# >= 2)) || die "--maintainer requires a value"
            MAINTAINER="$2"
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        *)
            die "unknown option: $1 (try --help)"
            ;;
    esac
    shift
done
(($# == 0)) || die "unexpected argument: $1"

# ---------------------------------------------------------------------------
# Validate inputs and required tools before doing any work
# ---------------------------------------------------------------------------

if ((${#FORMATS[@]} == 0)); then
    FORMATS=("deb" "rpm")
else
    # `--deb --deb`, or `--all --rpm`, should still build each format once.
    declare -A seen_formats=()
    unique_formats=()
    for format in "${FORMATS[@]}"; do
        [[ -z "${seen_formats[$format]:-}" ]] || continue
        seen_formats[$format]=1
        unique_formats+=("$format")
    done
    FORMATS=("${unique_formats[@]}")
fi

if [[ -z "$VERSION" ]]; then
    VERSION="$(
        awk '
            /^\[package\]/ { in_package = 1; next }
            /^\[/          { in_package = 0 }
            in_package && /^version[[:space:]]*=/ {
                sub(/^[^=]*=[[:space:]]*"/, "")
                sub(/".*$/, "")
                print
                exit
            }
        ' Cargo.toml
    )"
    [[ -n "$VERSION" ]] || die "could not read the package version from Cargo.toml"
fi

[[ "$VERSION" =~ ^[0-9][0-9A-Za-z.+~]*$ ]] ||
    die "invalid version '$VERSION' (expected a digit followed by letters, digits, '.', '+' or '~')"
[[ "$RELEASE" =~ ^[0-9]+$ ]] || die "release must be a non-negative integer"
[[ -n "$MAINTAINER" ]] || die "maintainer cannot be empty"

if ((DO_BUILD)); then
    command -v cargo >/dev/null 2>&1 ||
        die "cargo was not found; install Rust or pass --no-build"
fi

for format in "${FORMATS[@]}"; do
    case "$format" in
        deb)
            command -v dpkg-deb >/dev/null 2>&1 ||
                die "dpkg-deb was not found; install the dpkg package or build only --rpm"
            command -v gzip >/dev/null 2>&1 ||
                die "gzip was not found; install gzip or build only --rpm"
            ;;
        rpm)
            command -v rpmbuild >/dev/null 2>&1 ||
                die "rpmbuild was not found; install rpm-build (Fedora/RHEL) or rpm (Debian/Ubuntu), or build only --deb"
            ;;
    esac
done

for asset in \
    README.md \
    LICENSE \
    packaging/rustquery.1 \
    packaging/rustquery.desktop \
    packaging/rustquery.metainfo.xml \
    packaging/icons/hicolor/scalable/apps/rustquery.svg \
    packaging/rpm/rustquery.spec; do
    [[ -f "$ROOT_DIR/$asset" ]] || die "missing packaging asset: $asset"
done

# Validate the desktop metadata when the distribution tools are available.
# They are not required to build a package, but catching a malformed launcher
# or AppStream file here is cheaper than discovering it after installation.
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$ROOT_DIR/packaging/rustquery.desktop" ||
        die "packaging/rustquery.desktop failed desktop-file-validate"
else
    warn "desktop-file-validate not found; skipping launcher validation"
fi
if command -v appstreamcli >/dev/null 2>&1; then
    if ! appstream_output="$(appstreamcli validate --no-net "$ROOT_DIR/packaging/rustquery.metainfo.xml" 2>&1)"; then
        if printf '%s\n' "$appstream_output" | grep -q '^E:'; then
            printf '%s\n' "$appstream_output" >&2
            die "packaging/rustquery.metainfo.xml failed AppStream validation"
        fi
        note "AppStream metadata passed with non-fatal warnings"
    fi
else
    warn "appstreamcli not found; skipping AppStream validation"
fi

# ---------------------------------------------------------------------------
# Architecture mapping
# ---------------------------------------------------------------------------

target_arch() {
    if [[ -n "$TARGET" ]]; then
        printf '%s' "${TARGET%%-*}"
    else
        uname -m
    fi
}

deb_arch_for() {
    case "$1" in
        amd64 | x86_64) printf 'amd64' ;;
        arm64 | aarch64) printf 'arm64' ;;
        armhf | armv7 | armv7l) printf 'armhf' ;;
        armel | armv6l) printf 'armel' ;;
        i386 | i486 | i586 | i686 | x86) printf 'i386' ;;
        ppc64le | powerpc64le) printf 'ppc64el' ;;
        riscv64 | riscv64gc) printf 'riscv64' ;;
        loong64 | loongarch64) printf 'loong64' ;;
        *) printf '%s' "$1" ;;
    esac
}

rpm_arch_for() {
    case "$1" in
        amd64) printf 'x86_64' ;;
        arm64) printf 'aarch64' ;;
        armhf | armv7 | armv7l) printf 'armv7hl' ;;
        armel | armv6l) printf 'armv6hl' ;;
        i386 | i486 | i586 | i686 | x86) printf 'i686' ;;
        ppc64el | powerpc64le) printf 'ppc64le' ;;
        riscv64 | riscv64gc) printf 'riscv64' ;;
        loong64) printf 'loongarch64' ;;
        *) printf '%s' "$1" ;;
    esac
}

if [[ -z "$DEB_ARCH" ]]; then
    if [[ -z "$TARGET" ]] && command -v dpkg >/dev/null 2>&1; then
        DEB_ARCH="$(dpkg --print-architecture)"
    else
        DEB_ARCH="$(deb_arch_for "$(target_arch)")"
    fi
fi
if [[ -z "$RPM_ARCH" ]]; then
    RPM_ARCH="$(rpm_arch_for "$(target_arch)")"
fi

[[ "$DEB_ARCH" =~ ^[a-z0-9][a-z0-9-]*$ ]] || die "invalid Debian architecture: $DEB_ARCH"
[[ "$RPM_ARCH" =~ ^[a-z0-9_]+$ ]] || die "invalid RPM architecture: $RPM_ARCH"

# ---------------------------------------------------------------------------
# Build the release binary
# ---------------------------------------------------------------------------

if [[ -n "$TARGET" ]]; then
    BINARY="target/$TARGET/release/$PKG_NAME"
else
    BINARY="target/release/$PKG_NAME"
fi

if ((DO_BUILD)); then
    cargo_args=(build --locked --release)
    if [[ -n "$TARGET" ]]; then
        cargo_args+=(--target "$TARGET")
    fi
    note "Building $PKG_NAME $VERSION (cargo ${cargo_args[*]})"
    cargo "${cargo_args[@]}"
else
    note "Reusing existing binary $BINARY"
fi

[[ -x "$BINARY" ]] || die "release binary not found or not executable: $BINARY"

# Cross-compiled binaries may not be runnable on this host, so a failed probe
# is only a warning. A successful probe catching a stale binary is useful.
reported_version=""
if version_output="$("$BINARY" --version 2>/dev/null)"; then
    reported_version="$(printf '%s\n' "$version_output" | awk '{print $2; exit}')"
fi
if [[ -n "$reported_version" && "$reported_version" != "$VERSION" ]]; then
    warn "binary reports version $reported_version but the package version is $VERSION"
elif [[ -z "$reported_version" ]]; then
    warn "could not run $BINARY to verify its version; continuing"
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rustquery-package.XXXXXX")"
cleanup() {
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Debian package
# ---------------------------------------------------------------------------

# Let dpkg-shlibdeps determine the exact shared-library dependencies of the
# binary when dpkg-dev is available. This produces versioned libc/libssl
# requirements for the current build distribution. The fallback covers systems
# with only dpkg-deb installed.
deb_depends() {
    local binary="$1"
    local deps=""
    local probe_dir="$WORK_DIR/shlibdeps"
    local ldd_output

    ldd_output="$(ldd "$binary" 2>&1 || true)"
    if [[ "$ldd_output" != *"not a dynamic executable"* ]]; then
        if command -v dpkg-shlibdeps >/dev/null 2>&1; then
            mkdir -p "$probe_dir/debian"
            cat >"$probe_dir/debian/control" <<EOF
Source: $PKG_NAME
Section: database
Priority: optional
Maintainer: $MAINTAINER
Standards-Version: 4.6.2

Package: $PKG_NAME
Architecture: any
Description: dependency probe
 Probe used while building the package.
EOF
            deps="$(
                cd "$probe_dir" &&
                    dpkg-shlibdeps -O "$ROOT_DIR/$binary" 2>/dev/null |
                    sed -n 's/^shlibs:Depends=//p'
            )" || deps=""
        fi
    fi

    if [[ -n "$deps" ]]; then
        printf '%s' "$deps"
    elif [[ "$ldd_output" == *"not a dynamic executable"* ]]; then
        # A static binary (for example a musl target) has no shared-library
        # dependencies to record.
        printf ''
    else
        # Conservative fallback for hosts without dpkg-dev. libssl3t64 provides
        # libssl3 on time_t-transition distributions.
        printf 'libc6, libssl3 | libssl3t64'
    fi
}

build_deb() {
    note "Building Debian package for $DEB_ARCH"
    local stage="$WORK_DIR/deb/${PKG_NAME}_${VERSION}-${RELEASE}_${DEB_ARCH}"
    local depends
    local installed_size
    local output="$OUT_DIR/${PKG_NAME}_${VERSION}-${RELEASE}_${DEB_ARCH}.deb"

    install -D -m 0755 -p "$BINARY" "$stage/usr/bin/$PKG_NAME"
    install -D -m 0644 -p "$ROOT_DIR/README.md" "$stage/usr/share/doc/$PKG_NAME/README.md"
    install -D -m 0644 -p "$ROOT_DIR/packaging/rustquery.1" "$stage/usr/share/man/man1/rustquery.1"
    gzip -9n "$stage/usr/share/man/man1/rustquery.1"
    install -D -m 0644 -p "$ROOT_DIR/packaging/rustquery.desktop" \
        "$stage/usr/share/applications/$PKG_NAME.desktop"
    install -D -m 0644 -p "$ROOT_DIR/packaging/rustquery.metainfo.xml" \
        "$stage/usr/share/metainfo/$PKG_NAME.metainfo.xml"
    install -D -m 0644 -p "$ROOT_DIR/packaging/icons/hicolor/scalable/apps/rustquery.svg" \
        "$stage/usr/share/icons/hicolor/scalable/apps/$PKG_NAME.svg"

    # Debian policy expects a machine-readable copyright file.
    {
        cat <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: $PKG_NAME

Files: *
Copyright: 2026 RustQuery contributors
License: MIT
EOF
        sed -e 's/^$/./' -e 's/^/ /' "$ROOT_DIR/LICENSE"
    } >"$stage/usr/share/doc/$PKG_NAME/copyright"
    chmod 0644 "$stage/usr/share/doc/$PKG_NAME/copyright"

    # A Debian changelog keeps the package lintian-clean and lets tools such as
    # apt-listchanges show what was installed.
    {
        cat <<EOF
$PKG_NAME ($VERSION-$RELEASE) unstable; urgency=medium

  * Package built by scripts/build-packages.sh from version $VERSION.

 -- $MAINTAINER  $(LC_ALL=C date -R)
EOF
    } >"$stage/usr/share/doc/$PKG_NAME/changelog.Debian"
    gzip -9n "$stage/usr/share/doc/$PKG_NAME/changelog.Debian"
    chmod 0644 "$stage/usr/share/doc/$PKG_NAME/changelog.Debian.gz"

    depends="$(deb_depends "$BINARY")"
    if [[ -n "$depends" ]]; then
        depends="$depends, hicolor-icon-theme"
    else
        depends="hicolor-icon-theme"
    fi
    installed_size="$(du -sk "$stage/usr" | cut -f1)"

    mkdir -p "$stage/DEBIAN"
    {
        cat <<EOF
Package: $PKG_NAME
Version: $VERSION-$RELEASE
Architecture: $DEB_ARCH
Maintainer: $MAINTAINER
Installed-Size: $installed_size
EOF
        if [[ -n "$depends" ]]; then
            printf 'Depends: %s\n' "$depends"
        fi
        cat <<EOF
Section: database
Priority: optional
Recommends: xdg-desktop-portal, xdg-desktop-portal-backend
Description: desktop SQL workbench for PostgreSQL, MySQL and SQLite
 RustQuery is a fast, native desktop SQL client with saved connection
 profiles, a lazy schema explorer, multiple SQL editor tabs, a virtualised
 result grid, filtering, sorting, CSV/JSON/SQL exports and transaction
 controls.
 .
 SQLite support is embedded. PostgreSQL and MySQL require a reachable
 database server at runtime.
EOF
    } >"$stage/DEBIAN/control"

    # Checksums let package tools verify files after installation.
    (
        cd "$stage"
        find . -path ./DEBIAN -prune -o -type f -print0 |
            sort -z |
            xargs -0 md5sum |
            sed 's|  \./|  |'
    ) >"$stage/DEBIAN/md5sums"
    chmod 0644 "$stage/DEBIAN/md5sums"

    dpkg-deb --build --root-owner-group "$stage" "$output" >/dev/null
    ARTIFACTS+=("$output")
}

# ---------------------------------------------------------------------------
# RPM package
# ---------------------------------------------------------------------------

build_rpm() {
    note "Building RPM package for $RPM_ARCH"
    local top="$WORK_DIR/rpmbuild"
    local rpm
    local found=0
    local -a args

    mkdir -p "$top/BUILD" "$top/BUILDROOT" "$top/RPMS" "$top/SOURCES" "$top/SPECS" "$top/SRPMS" "$top/rpmdb"
    install -m 0755 -p "$BINARY" "$top/SOURCES/rustquery"
    install -m 0644 -p "$ROOT_DIR/README.md" "$top/SOURCES/README.md"
    install -m 0644 -p "$ROOT_DIR/LICENSE" "$top/SOURCES/LICENSE"
    install -m 0644 -p "$ROOT_DIR/packaging/rustquery.1" "$top/SOURCES/rustquery.1"
    install -m 0644 -p "$ROOT_DIR/packaging/rustquery.desktop" "$top/SOURCES/rustquery.desktop"
    install -m 0644 -p "$ROOT_DIR/packaging/rustquery.metainfo.xml" "$top/SOURCES/rustquery.metainfo.xml"
    install -m 0644 -p "$ROOT_DIR/packaging/icons/hicolor/scalable/apps/rustquery.svg" "$top/SOURCES/rustquery.svg"

    args=(
        -bb
        --define "_topdir $top"
        --define "_dbpath $top/rpmdb"
        --define "rq_version $VERSION"
        --define "rq_release $RELEASE"
        --define "rq_date $(LC_ALL=C date '+%a %b %d %Y')"
        --define "rq_maintainer $MAINTAINER"
    )
    if [[ -n "$RPM_ARCH" ]]; then
        args+=(--target "$RPM_ARCH")
    fi

    rpmbuild "${args[@]}" "$ROOT_DIR/packaging/rpm/rustquery.spec"

    while IFS= read -r -d '' rpm; do
        found=1
        cp -p "$rpm" "$OUT_DIR/"
        ARTIFACTS+=("$OUT_DIR/$(basename "$rpm")")
    done < <(find "$top/RPMS" -type f -name '*.rpm' -print0)

    ((found)) || die "rpmbuild completed without producing an RPM"
}

for format in "${FORMATS[@]}"; do
    case "$format" in
        deb) build_deb ;;
        rpm) build_rpm ;;
    esac
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

note "Packages written to $OUT_DIR"
for artifact in "${ARTIFACTS[@]}"; do
    printf '    %s (%s)\n' "$artifact" "$(du -h "$artifact" | cut -f1)"
done
