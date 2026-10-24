# RustQuery

<img src="packaging/icons/hicolor/128x128/apps/rustquery.png" width="96" alt="RustQuery icon">

RustQuery is a fast, native desktop SQL workbench for **PostgreSQL**, **MySQL**
and **SQLite**, written in Rust with [eframe], [egui], [tokio] and [sqlx].

It is built for a focused desktop workflow: connect, browse the schema, write
several SQL tabs, execute a statement or a multi-statement script, inspect and
filter the result grid, then export the data. Menus, keyboard shortcuts, native
file dialogs and a resizable layout keep everything in one window.

The source code lives at <https://github.com/GokulTechno/rustquery>.

## Features

- **Connection manager** — save, edit, delete and test PostgreSQL/MySQL
  profiles (name, host, port, database, user, password, SSL mode) and SQLite
  profiles (name + database file, including `:memory:`).
- **Connection dashboard** — profile cards with driver, endpoint and status,
  quick connect, connection testing and a shortcut to the SQL workspace.
- **Schema explorer** — lazy tree of schemas/databases, tables, views, columns,
  indexes and foreign keys. Click or double-click a table to preview
  `SELECT * … LIMIT 100`; filter the tree and refresh it on demand.
- **SQL editor** — multiple tabs, line numbers, SQL highlighting, accurate
  selections, undo/redo, indentation, comment toggling, word motions and
  schema/type-aware completion.
- **Script execution** — executes a selection or the whole buffer.
  Multi-statement scripts run through sqlx's simple protocol, with a
  configurable row limit and timeout.
- **Transactions** — `F8`/`F9`/`F10` run `BEGIN`/`COMMIT`/`ROLLBACK` on a
  dedicated single-connection pool, so every statement in the transaction uses
  the same physical database session. SQLite uses one shared connection for all
  work, which keeps transactions and `:memory:` databases consistent. An open
  transaction is shown in the status bar.
- **Virtualised result grid** — scroll through large result sets without
  rendering every row, sort columns by clicking the header or pressing `s`,
  filter all columns client-side, copy cells/rows/visible data, and open full
  cell values.
- **Exports** — CSV, JSON and SQL `INSERT` statements, with visible/all rows and
  optional headers (`Ctrl+E`).
- **Four themes** — Catppuccin Mocha, Catppuccin Latte, Nord and Dracula
  (`F12`).
- **Console** — timestamped, levelled log with database error details
  (including PostgreSQL line/column hints), an errors-only filter and follow
  mode.
- **Persistent workspace** — tabs, settings, profiles and query history are
  restored on the next launch. Workspace edits autosave every five seconds and
  on a clean quit.
- **Desktop integration** — the packages and the installer add a launcher
  entry, a scalable icon plus PNG icons from 16 to 512 px, and AppStream
  metadata, so RustQuery appears in application menus, docks and software
  centres.

## Requirements

- Rust **1.82+** (edition 2021).
- A graphical session with OpenGL support. RustQuery runs on X11 and Wayland
  on Linux; macOS and Windows use their native backends.
- A reachable PostgreSQL and/or MySQL server for client/server databases.
  SQLite support is embedded and needs no external service.
- OpenSSL development libraries are needed by `sqlx`'s `native-tls` feature on
  Linux. On Debian/Ubuntu:

  ```bash
  sudo apt install pkg-config libssl-dev
  ```

  On macOS, the system libraries are sufficient.
- Native open/save dialogs use `xdg-desktop-portal` on Linux. Install the
  portal and a backend for your desktop (for example
  `xdg-desktop-portal-gtk`). The in-app export dialog works without a portal.

## Install

### Debian / Ubuntu

After building a package with `scripts/build-packages.sh` (see
[Packaging](#packaging)), install it with:

```bash
sudo apt install ./dist/rustquery_0.1.0-1_amd64.deb
```

### Fedora / RHEL

```bash
sudo dnf install ./dist/rustquery-0.1.0-1.x86_64.rpm
```

### From source

`scripts/install.sh` builds the release binary and installs it together with
the launcher, icons, AppStream metadata, man page and documentation:

```bash
./scripts/install.sh                 # current user, ~/.local
sudo ./scripts/install.sh --system   # system-wide, /usr/local
./scripts/install.sh --prefix /opt/rustquery
```

The launcher points at the installed binary, so the application works even when
`<prefix>/bin` is not on `PATH`. Pass `--no-build` to reuse an existing
`target/release/rustquery`, `--destdir DIR` to stage an installation, or
`--uninstall` to remove an installation made with the same prefix. User
configuration in `~/.config/rustquery` is not touched by `--uninstall`.

After installation, RustQuery appears in the application menu. The binary is
installed as `<prefix>/bin/rustquery`.

## Build and run

```bash
cargo build --release
./target/release/rustquery
```

Or during development:

```bash
cargo run
```

### CLI

```
rustquery [OPTIONS]

  --config-dir <PATH>   Use a custom configuration directory
  -h, --help            Print help
  -V, --version         Print version
```

The `--config-dir` option is useful for portable installations and isolated
testing. Environment variables also affect the default location; see below.

## Quick start

1. Launch RustQuery. The connection dashboard opens when no session is active.
2. Choose **New connection** (`Ctrl+Shift+N`), select a driver and fill in the
   fields. Use **Test connection** to verify the settings, then save the
   profile.
3. Select the profile and choose **Connect** (or double-click the card).
4. Type SQL in the editor and press `F5` (or `Ctrl+Enter`). A selection runs on
   its own; otherwise the whole buffer runs as a script.
5. Sort by clicking a column header, press `Ctrl+F` to filter, and press
   `Ctrl+E` to export the current result set.
6. Press `F1` at any time for the full keyboard reference.

For a local PostgreSQL server the defaults are `localhost:5432`, user
`postgres`; for MySQL they are `localhost:3306`, user `root`. Set the database
name and password for your server.

For SQLite, choose **SQLite** as the driver and enter a file path (for example
an absolute path, or a path relative to the working directory). Missing files
are created automatically; use `:memory:` for a temporary in-memory database.
SQLite profiles do not ask for host, user, password or SSL settings.
`sqlite:` connection URLs are also accepted and follow SQLite URI semantics —
add `?mode=rwc` if a missing file should be created.

## Keyboard reference

### Global

| Keys | Action |
| --- | --- |
| `F1` | Help (keyboard reference) |
| `F2` | Connection dashboard / workspace toggle |
| `F3` / `F4` / `F6` / `F7` | Focus explorer / editor / results / console |
| `F5` / `Ctrl+Enter` | Execute selection or buffer |
| `F8` / `F9` / `F10` | `BEGIN` / `COMMIT` / `ROLLBACK` |
| `F11` / `Ctrl+,` | Settings |
| `F12` | Cycle theme |
| `Ctrl+H` | Query history |
| `Ctrl+D` | Disconnect (with confirmation for open work) |
| `Ctrl+Q` | Quit (confirmation when work is running) |
| `Esc` | Close dialog / clear filter / leave completion |

### Files and connections

| Keys | Action |
| --- | --- |
| `Ctrl+Shift+N` | New connection profile |
| `Ctrl+O` / `Ctrl+S` | Open / save a SQL file |
| `Ctrl+E` | Export the current result set |
| `Ctrl+F` | Filter all result columns |
| `Enter` | Connect to the selected profile |
| Double-click | Connect to a profile, preview a table, open a cell |

### Editor

| Keys | Action |
| --- | --- |
| `F5` / `Ctrl+Enter` | Execute selection or whole buffer |
| `Ctrl+C` | Cancel the running query when active |
| `Ctrl+T` / `Ctrl+W` | New / close tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+Space` | Schema + keyword completion |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
| `Ctrl+/` | Toggle `--` line comments |
| `Tab` / `Shift+Tab` | Indent / outdent |
| `Ctrl+Left` / `Ctrl+Right` | Move by word |
| `Ctrl+Backspace` | Delete the previous word |
| `Ctrl+A` | Select all |

### Results grid

| Keys | Action |
| --- | --- |
| Arrow keys | Move the selected cell |
| `Home` / `End` | First / last column |
| `Ctrl+Home` / `Ctrl+End` | First / last row |
| `PgUp` / `PgDn` | Page through rows |
| `s` / `Shift+S` | Cycle sort / clear sorting |
| `Ctrl+F` | Filter all columns |
| `Enter` | Open the full cell value |
| `c` / `y` / `Ctrl+Y` | Copy cell / row / visible results |
| `Ctrl+E` | Export CSV, JSON or SQL INSERT |
| `Esc` | Return focus to the editor |

### Mouse

- Click tabs, toolbar buttons, tree rows and grid cells.
- Click a column header to cycle its sort direction.
- Double-click a profile to connect, a table to preview it, or a cell to open
  the full value.
- Middle-click a tab to close it.
- Drag the split handles to resize the explorer, editor, results and console.
- Right-click a schema tree row for **Copy name** and **Preview data**.

## Configuration and data

State is stored as JSON in the first available location:

1. `$RUSTQUERY_CONFIG_DIR`
2. `$XDG_CONFIG_HOME/rustquery`
3. `~/.config/rustquery`
4. `./.rustquery` (fallback when `HOME` is unset)

| File | Contents |
| --- | --- |
| `profiles.json` | Saved connection profiles |
| `settings.json` | Theme, row limit, timeout, editor split, sidebar width, console |
| `history.json` | Recent executed SQL (max 500) |
| `workspace.json` | Open query tabs and active tab (autosaved every 5 s) |

Writes are atomic (temporary file + rename) and the profiles file is created
with Unix mode `0600` where supported. Corrupt JSON files are renamed with a
timestamp so they can be recovered; RustQuery starts with defaults and reports
the problem in the console.

### Security note

Passwords are stored in `profiles.json` as **plain text** inside a
permission-protected file. This is intentional for a local desktop client, but
it is not a secrets manager: anyone who can read your user's config directory
can read the passwords. Use dedicated, least-privilege database accounts and
consider your OS disk encryption and backup exposure. The internal clipboard
fallback also keeps copied data in process memory only.

## Architecture

```
scripts/
├── build-packages.sh    Debian and RPM packaging
└── install.sh           per-user/system installer and uninstaller
src/
├── main.rs              eframe entry point, CLI parsing, Tokio runtime, window icon
├── lib.rs               public module surface (testable application library)
├── app.rs               central App state, worker messages, query/tab/transaction logic
├── config.rs            profiles/settings/history/workspace persistence
├── db/
│   ├── mod.rs           DbPool/Session abstraction and pool helpers
│   ├── connect.rs       PostgreSQL/MySQL/SQLite pools, SSL modes, test connections
│   ├── execute.rs       multi-statement raw execution, row limits, transactions
│   ├── introspect.rs    schemas/databases, tables, views, columns, indexes, FKs
│   ├── value.rs         dynamic SQLx row decoding and text formatting
│   └── error.rs         structured DbError with SQLSTATE/line/column hints
├── editor/
│   ├── mod.rs           buffer, selections, undo/redo, motions, completion prefix
│   └── highlight.rs     dependency-free SQL tokenizer and syntax cache
├── grid.rs              virtual scrolling, filtering, sorting, column widths
├── schema_tree.rs       lazy schema tree and flattened render rows
├── export.rs            CSV/JSON/SQL INSERT serialisation
├── clipboard.rs         arboard integration + internal fallback
├── model.rs             shared domain types
└── desktop/
    ├── mod.rs           eframe shell, menus, shortcuts, status, toasts, file dialogs
    ├── theme.rs         Catppuccin/Nord/Dracula palettes and egui style
    ├── editor.rs        custom SQL editor backed by the shared editor buffer
    ├── workspace.rs     tabs, toolbar, explorer, editor and console layout
    ├── results.rs       virtualised result grid, inline filter and headers
    ├── dashboard.rs     profile cards, connection testing and session details
    └── modals.rs        help, forms, confirmations, filters, history, settings
tests/
├── sqlite.rs            end-to-end SQLite execution/introspection tests
└── desktop.rs           headless tests for profile, export, grid and theme helpers
```

### Why this design?

- **Single UI thread.** Rendering, input handling and app mutations happen on
  one thread. Background database work reports back over an unbounded channel
  as `WorkerMessage`s. This keeps state transitions atomic from the UI's point
  of view and avoids locking.
- **Immediate-mode desktop UI.** `eframe`/`egui` draws the whole interface each
  frame from the shared `App` state. The result grid only lays out visible
  rows, so large result sets stay responsive.
- **Request ids everywhere.** Each query gets a monotonic id. Results for
  closed tabs, replaced sessions or cancelled queries are ignored rather than
  corrupting the current tab.
- **Dedicated transaction pool.** A normal SQLx `Transaction` would borrow a
  connection across async tasks. Instead, RustQuery maintains a one-connection
  pool per session for `BEGIN`/`COMMIT`/`ROLLBACK` and routes every statement
  while `tx_active` through it. Queries remain plain owned futures. SQLite
  shares one single-connection pool for both paths, because SQLite transactions
  are tied to the physical connection and `:memory:` databases must not be
  duplicated.
- **Lazy introspection.** Connecting loads schemas and object names only.
  Columns, indexes and foreign keys are fetched when a table is expanded.
- **Custom editor and tokenizer.** This avoids external widget plugin coupling
  and gives character-accurate selection, grouped undo, correct wide-character
  rendering and fast incremental highlighting.

## Tests

Run the offline test suite:

```bash
cargo test
```

The 56 unit tests cover editor editing/undo/highlighting, SQL export escaping,
grid sorting/filtering, schema tree behaviour, config validation, SQL error
line/column mapping and more. Six headless desktop tests cover profile forms,
export paths, grid filtering/sorting, legacy settings and every theme palette.
Two end-to-end SQLite tests exercise real connections, scripts, transactions,
dynamic values and introspection without an external service. **No external
database is required.**

To check formatting and lints:

```bash
cargo fmt -- --check
cargo clippy --all-targets
```

## Packaging

`scripts/build-packages.sh` builds Debian and RPM packages from the release
binary:

```bash
./scripts/build-packages.sh              # .deb and .rpm
./scripts/build-packages.sh --deb        # Debian only
./scripts/build-packages.sh --rpm        # RPM only
./scripts/build-packages.sh --no-build   # reuse target/release/rustquery
```

Packages are written to `dist/` as
`rustquery_<version>-<release>_<arch>.deb` and
`rustquery-<version>-<release>[.<dist>].<arch>.rpm`, where the optional `.dist`
suffix comes from the RPM platform. They contain:

- `/usr/bin/rustquery`
- the `rustquery(1)` man page
- `rustquery.desktop` in the applications menu
- a scalable icon and PNG icons under `hicolor/.../apps`
- AppStream metadata in `/usr/share/metainfo`
- the README and the MIT license

`scripts/install.sh` installs the same files into `~/.local`, `/usr/local` or a
custom prefix without building a package. It also refreshes the desktop and
icon caches when the tools are available.

The Debian package depends on `hicolor-icon-theme` and recommends
`xdg-desktop-portal` plus a portal backend for native file dialogs. The RPM
spec declares the same icon-theme requirement and recommends the portal.

Packaging tools:

- Debian: `dpkg-deb`. Install `dpkg-dev` as well for automatic, versioned
  shared-library dependencies; without it the script falls back to `libc6` and
  `libssl3`. `desktop-file-validate` and `appstreamcli` are used when present
  to check the launcher and metadata.
- RPM: `rpmbuild`, provided by `rpm-build` on Fedora/RHEL and by `rpm` on
  Debian/Ubuntu.

The script packages the binary compiled on the current host. Build inside a
container or chroot matching an older target distribution if the package must
run there. Use `./scripts/build-packages.sh --help` for version, release,
target-triple, architecture, maintainer and output-directory overrides.

## Troubleshooting

- **`error: failed to run custom build command for openssl-sys`** — install the
  OpenSSL development package (`libssl-dev` / `openssl-devel`) and `pkg-config`.
- **The window does not open or is blank** — make sure `DISPLAY` or
  `WAYLAND_DISPLAY` points at a running session and that the GPU driver
  supports OpenGL. As a fallback, try
  `LIBGL_ALWAYS_SOFTWARE=1 rustquery`. On a hybrid Wayland/X11 setup, unset
  `WAYLAND_DISPLAY` to force X11.
- **Open/save file dialogs do not appear** — install and start
  `xdg-desktop-portal` and a backend such as `xdg-desktop-portal-gtk`. The
  export dialog is part of the application and does not need a portal.
- **Connection test times out** — verify host/port/firewall and that the server
  allows TCP connections. SQLx uses `native-tls`; for `verify-ca`/`verify-full`
  the server certificate must be valid for the configured host.
- **SQLite says “unable to open database file”** — check that the directory
  exists and is writable. RustQuery creates missing files, but not missing
  parent directories.
- **SQLite says “database is locked”** — another process holds a write lock.
  RustQuery waits up to five seconds; close the other writer or retry.
- **No system clipboard** — in minimal or remote sessions, RustQuery keeps
  copied text in an internal clipboard and still supports pasting it back.
- **A file says it is corrupt** — RustQuery backs it up as
  `<name>.corrupt-<timestamp>.json` and starts with defaults; inspect or restore
  the backup manually.

## License

MIT. See `Cargo.toml` for package metadata.

[eframe]: https://github.com/emilk/egui/tree/master/crates/eframe
[egui]: https://github.com/emilk/egui
[tokio]: https://tokio.rs
[sqlx]: https://github.com/launchbadge/sqlx
