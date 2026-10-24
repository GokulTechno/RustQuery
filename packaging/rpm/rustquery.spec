%{!?rq_version:%global rq_version 0.1.0}
%{!?rq_release:%global rq_release 1}
%{!?rq_date:%global rq_date Thu Jan 01 2026}
%{!?rq_maintainer:%global rq_maintainer RustQuery contributors <rustquery@localhost>}

# RustQuery packages a prebuilt release binary: no Rust toolchain or network
# access is required inside rpmbuild. The build-packages.sh script populates
# the rpmbuild SOURCES directory with the binary, documentation and desktop
# integration files.
%global debug_package %{nil}
%global _build_id_links none

Name:           rustquery
Version:        %{rq_version}
Release:        %{rq_release}%{?dist}
Summary:        Desktop SQL workbench for PostgreSQL, MySQL and SQLite

License:        MIT
Packager:       %{rq_maintainer}
Requires:       hicolor-icon-theme
Recommends:     xdg-desktop-portal

Source0:        rustquery
Source1:        README.md
Source2:        LICENSE
Source3:        rustquery.1
Source4:        rustquery.desktop
Source5:        io.github.gokultechno.RustQuery.metainfo.xml
Source6:        rustquery-icons.tar.gz

%description
RustQuery is a fast, native desktop SQL client for PostgreSQL, MySQL and
SQLite. It provides saved connection profiles, a lazy schema explorer, multiple
SQL editor tabs, a virtualised result grid, sorting, filtering, CSV/JSON/SQL
exports and transaction controls.

SQLite support is embedded. PostgreSQL and MySQL require a reachable database
server at runtime.

%prep

%build

%install
install -D -m 0755 -p %{SOURCE0} %{buildroot}%{_bindir}/rustquery
install -D -m 0644 -p %{SOURCE1} %{buildroot}%{_datadir}/doc/%{name}/README.md
install -D -m 0644 -p %{SOURCE2} %{buildroot}%{_datadir}/licenses/%{name}/LICENSE
install -D -m 0644 -p %{SOURCE3} %{buildroot}%{_mandir}/man1/rustquery.1
install -D -m 0644 -p %{SOURCE4} %{buildroot}%{_datadir}/applications/%{name}.desktop
install -D -m 0644 -p %{SOURCE5} %{buildroot}%{_datadir}/metainfo/io.github.gokultechno.RustQuery.metainfo.xml
install -d %{buildroot}%{_datadir}/icons
tar -xzf %{SOURCE6} -C %{buildroot}%{_datadir}/icons

%files
%{_bindir}/rustquery
%{_datadir}/applications/%{name}.desktop
%{_datadir}/icons/hicolor/scalable/apps/%{name}.svg
%{_datadir}/icons/hicolor/*/apps/%{name}.png
%{_datadir}/metainfo/io.github.gokultechno.RustQuery.metainfo.xml
%{_mandir}/man1/rustquery.1*
%doc %{_datadir}/doc/%{name}/README.md
%license %{_datadir}/licenses/%{name}/LICENSE

%changelog
* %{rq_date} %{rq_maintainer} - %{rq_version}-%{rq_release}
- Package built by scripts/build-packages.sh.
