# Installing RustQuery

RustQuery is a native desktop SQL workbench for PostgreSQL, MySQL and SQLite.
This guide covers installing a prebuilt Debian or RPM package, building and
installing from source, verifying the installation, and uninstalling it again.

## Requirements

- Linux with an X11 or Wayland session and OpenGL support. RustQuery also
  builds on macOS and Windows using their native window backends.
- PostgreSQL and/or MySQL servers if you use those drivers. SQLite support is
  embedded and needs no external service.
- Native open/save dialogs use `xdg-desktop-portal` on Linux. Install the
  portal and a backend such as `xdg-desktop-portal-gtk`; the in-app export
  dialog works without a portal.

For source builds you also need:

- Rust **1.89+** (edition 2021), preferably installed with
  [rustup](https://rustup.rs/). Source builds honour `rust-toolchain.toml` in
  the repository root.
- `pkg-config` and OpenSSL development headers.
- A C toolchain for native dependencies.

Debian/Ubuntu:

```bash
sudo apt update
sudo apt install build-essential pkg-config libssl-dev
```

Fedora/RHEL:

```bash
sudo dnf install gcc make pkgconf-pkg-config openssl-devel
```

Install Rust if it is not already available:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

## Install from a package

Build the packages first with `scripts/build-packages.sh`, or use packages
provided with a release.

### Debian / Ubuntu

```bash
sudo apt install ./dist/rustquery_*.deb
```

`apt` resolves the shared-library dependencies automatically. If you use
`dpkg` directly, finish with `sudo apt-get -f install`.

### Fedora / RHEL

```bash
sudo dnf install ./dist/rustquery-*.rpm
```

### Package contents

Both formats install:

- `/usr/bin/rustquery`
- `/usr/share/applications/rustquery.desktop`
- a scalable SVG icon and PNG icons from 16 to 512 px under
  `/usr/share/icons/hicolor/.../apps/`
- AppStream metadata in `/usr/share/metainfo/`
- the `rustquery(1)` man page
- the README and the MIT license

The Debian package depends on `hicolor-icon-theme` and recommends
`xdg-desktop-portal` plus a portal backend. The RPM package declares the same
icon-theme requirement and recommends the portal.

## Install from source

### 1. Clone the repository

```bash
git clone https://github.com/GokulTechno/RustQuery.git
cd RustQuery
```

With SSH instead:

```bash
git clone git@github.com:GokulTechno/RustQuery.git
cd RustQuery
```

### 2. Run the installer

```bash
./scripts/install.sh
```

The script builds the release binary with `cargo build --release --locked` and
installs everything for the current user under `~/.local`. The installed
launcher points at the absolute binary path, so it works even when
`~/.local/bin` is not on `PATH`.

System-wide installation:

```bash
sudo ./scripts/install.sh --system
```

Custom prefix:

```bash
./scripts/install.sh --prefix /opt/rustquery
```

### Installer options

| Option | Meaning |
| --- | --- |
| `--prefix DIR` | Install under `DIR` |
| `--system` | Shortcut for `--prefix /usr/local` |
| `--user` | Shortcut for `--prefix $HOME/.local` |
| `--destdir DIR` | Stage the installation under `DIR`; skips cache refresh |
| `--no-build` | Reuse `target/release/rustquery` instead of building |
| `--uninstall` | Remove files installed with the same prefix |
| `-h`, `--help` | Show the installer help |

The installer writes:

```text
<prefix>/bin/rustquery
<prefix>/share/applications/rustquery.desktop
<prefix>/share/icons/hicolor/.../apps/rustquery.{svg,png}
<prefix>/share/metainfo/io.github.gokultechno.RustQuery.metainfo.xml
<prefix>/share/man/man1/rustquery.1.gz
<prefix>/share/doc/rustquery/{README.md,LICENSE}
```

It refreshes the desktop database and the icon cache when the corresponding
tools and cache directories are available. Staged installs with `--destdir` do
not touch the host caches.

### Manual installation

If you prefer to install the files yourself, build first:

```bash
cargo build --release --locked
```

Then copy the binary, launcher, metadata, man page and icons:

```bash
install -D -m 755 target/release/rustquery ~/.local/bin/rustquery
install -D -m 644 packaging/rustquery.desktop ~/.local/share/applications/rustquery.desktop
install -D -m 644 packaging/io.github.gokultechno.RustQuery.metainfo.xml \
    ~/.local/share/metainfo/io.github.gokultechno.RustQuery.metainfo.xml
install -D -m 644 packaging/icons/hicolor/scalable/apps/rustquery.svg \
    ~/.local/share/icons/hicolor/scalable/apps/rustquery.svg
for icon in packaging/icons/hicolor/*/apps/rustquery.png; do
    size="$(basename "$(dirname "$(dirname "$icon")")")"
    install -D -m 644 "$icon" "$HOME/.local/share/icons/hicolor/$size/apps/rustquery.png"
done
install -d ~/.local/share/man/man1
gzip -9n -c packaging/rustquery.1 > ~/.local/share/man/man1/rustquery.1.gz
```

The packaged launcher uses `Exec=rustquery`, so add `~/.local/bin` to `PATH`
or edit the installed desktop file to use the absolute path.

## Verify the installation

```bash
rustquery --version
```

If `~/.local/bin` is not on `PATH`:

```bash
~/.local/bin/rustquery --version
```

Start RustQuery from the application menu, or run `rustquery` in a terminal.
The connection dashboard opens when no session is active.

## Uninstall

If you used the installer:

```bash
./scripts/install.sh --uninstall
```

For a system-wide installation:

```bash
sudo ./scripts/install.sh --uninstall --system
```

Pass the same `--prefix` you used for installation if it was custom. The
uninstaller removes the binary, launcher, icons, AppStream metadata, man page
and documentation. It does **not** remove your saved connections or settings.

If you installed a package:

```bash
sudo apt remove rustquery
```

or:

```bash
sudo dnf remove rustquery
```

Configuration remains in `~/.config/rustquery` (or the directory selected with
`RUSTQUERY_CONFIG_DIR` / `--config-dir`). Remove it manually only if you also
want to delete saved profiles, history and workspace state:

```bash
rm -rf ~/.config/rustquery
```

## Troubleshooting

- **`error: failed to run custom build command for openssl-sys`** — install
  `libssl-dev` and `pkg-config` on Debian/Ubuntu, or `openssl-devel` and
  `pkgconf-pkg-config` on Fedora/RHEL.
- **`rustquery: command not found`** — use the full path
  `~/.local/bin/rustquery`, or add `~/.local/bin` to `PATH`.
- **The launcher or icon is missing** — log out and back in so the desktop
  refreshes its application and icon caches. Running the installer again also
  refreshes the desktop database when `update-desktop-database` is available.
- **The window does not open or is blank** — check that `DISPLAY` or
  `WAYLAND_DISPLAY` points at a running session and that the GPU driver supports
  OpenGL. Try `LIBGL_ALWAYS_SOFTWARE=1 rustquery`. On a hybrid Wayland/X11
  setup, unset `WAYLAND_DISPLAY` to force X11.
- **Open/save file dialogs do not appear** — install and start
  `xdg-desktop-portal` and a backend such as `xdg-desktop-portal-gtk`. The
  export dialog is part of the application and does not need a portal.
- **Connection test times out** — verify the host, port and firewall, and make
  sure the server accepts TCP connections. For `verify-ca`/`verify-full` SSL
  modes the server certificate must be valid for the configured host.
- **SQLite says “unable to open database file”** — check that the directory
  exists and is writable. RustQuery creates missing files, but not missing
  parent directories.
- **SQLite says “database is locked”** — another process holds a write lock.
  RustQuery waits up to five seconds; close the other writer or retry.

## Configuration locations

State is stored as JSON in the first available location:

1. `$RUSTQUERY_CONFIG_DIR`
2. `$XDG_CONFIG_HOME/rustquery`
3. `~/.config/rustquery`
4. `./.rustquery` when `HOME` is unset

| File | Contents |
| --- | --- |
| `profiles.json` | Saved connection profiles |
| `settings.json` | Theme, row limit, timeout and layout preferences |
| `history.json` | Recent executed SQL (maximum 500 entries) |
| `workspace.json` | Open query tabs and the active tab |

Passwords are stored in plain text inside the permission-protected
`profiles.json`. Use dedicated, least-privilege database accounts.

## License

MIT. See [LICENSE](LICENSE).
