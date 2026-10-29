//! Saved-connection explorer.
//!
//! The tree is `folder → connection name → host → database`. Schema objects
//! (schemas, tables, views, columns) are rendered under the database by the
//! sidebar once that host has a live session. Expanding a host is what opens
//! the session; collapsing it leaves the session open.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::model::{ConnectionFolder, ConnectionProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogKind {
    Folder,
    Connection,
    Endpoint,
    Database,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRow {
    pub id: String,
    pub depth: u16,
    pub label: String,
    pub detail: String,
    pub kind: CatalogKind,
    pub profile_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub expandable: bool,
    pub expanded: bool,
    pub loading: bool,
    pub error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogToggle {
    Folder,
    Connection,
    Endpoint { profile_id: Uuid, expanded: bool },
    Database { profile_id: Uuid, expanded: bool },
    None,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub folders: Vec<ConnectionFolder>,
    pub expanded_folders: HashSet<Uuid>,
    pub expanded_connections: HashSet<Uuid>,
    pub expanded_endpoints: HashSet<Uuid>,
    pub expanded_databases: HashSet<Uuid>,
    pub connect_errors: HashMap<Uuid, String>,
    pub selected: Option<String>,
    pub filter: String,
}

impl Catalog {
    /// Folders and connection names start open so every saved host is visible.
    /// Hosts stay collapsed until the user expands them, which is what connects.
    pub fn from_saved(folders: Vec<ConnectionFolder>, profiles: &[ConnectionProfile]) -> Self {
        let mut catalog = Self {
            folders,
            ..Self::default()
        };
        catalog.reveal_saved(profiles);
        catalog
    }

    pub fn reveal_saved(&mut self, profiles: &[ConnectionProfile]) {
        for folder in &self.folders {
            self.expanded_folders.insert(folder.id);
        }
        for profile in profiles {
            self.expanded_connections.insert(profile.id);
        }
    }

    pub fn rows(
        &self,
        profiles: &[ConnectionProfile],
        live: &HashSet<Uuid>,
        connecting: Option<Uuid>,
    ) -> Vec<CatalogRow> {
        let needle = self.filter.trim().to_lowercase();
        let mut rows = Vec::new();
        let mut placed = HashSet::new();
        let known_folders: HashSet<Uuid> = self.folders.iter().map(|folder| folder.id).collect();

        let mut folders: Vec<&ConnectionFolder> = self.folders.iter().collect();
        folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for folder in folders {
            let children = profiles_in_folder(profiles, &known_folders, Some(folder.id));
            let folder_matches = needle.is_empty() || folder.name.to_lowercase().contains(&needle);
            let child_matches = children
                .iter()
                .any(|profile| profile_matches(profile, &needle));
            if !needle.is_empty() && !folder_matches && !child_matches {
                continue;
            }
            let expanded = self.expanded_folders.contains(&folder.id) || !needle.is_empty();
            rows.push(CatalogRow {
                id: folder_id(folder.id),
                depth: 0,
                label: folder.name.clone(),
                detail: String::new(),
                kind: CatalogKind::Folder,
                profile_id: None,
                folder_id: Some(folder.id),
                expandable: true,
                expanded,
                loading: false,
                error: false,
            });
            if expanded {
                for profile in children {
                    placed.insert(profile.id);
                    if needle.is_empty() || folder_matches || profile_matches(profile, &needle) {
                        push_connection(
                            &mut rows,
                            self,
                            profile,
                            1,
                            live,
                            connecting,
                            !needle.is_empty(),
                        );
                    }
                }
            } else {
                for profile in children {
                    placed.insert(profile.id);
                }
            }
        }

        for profile in profiles_in_folder(profiles, &known_folders, None) {
            if placed.contains(&profile.id) {
                continue;
            }
            if !needle.is_empty() && !profile_matches(profile, &needle) {
                continue;
            }
            push_connection(
                &mut rows,
                self,
                profile,
                0,
                live,
                connecting,
                !needle.is_empty(),
            );
        }
        rows
    }

    pub fn toggle(&mut self, id: &str) -> CatalogToggle {
        if let Some(folder) = id.strip_prefix("folder:") {
            let Ok(folder_id) = Uuid::parse_str(folder) else {
                return CatalogToggle::None;
            };
            if !self.expanded_folders.remove(&folder_id) {
                self.expanded_folders.insert(folder_id);
            }
            return CatalogToggle::Folder;
        }
        if let Some(profile) = id.strip_prefix("conn:") {
            let Ok(profile_id) = Uuid::parse_str(profile) else {
                return CatalogToggle::None;
            };
            if !self.expanded_connections.remove(&profile_id) {
                self.expanded_connections.insert(profile_id);
            }
            return CatalogToggle::Connection;
        }
        if let Some(profile) = id.strip_prefix("host:") {
            let Ok(profile_id) = Uuid::parse_str(profile) else {
                return CatalogToggle::None;
            };
            let expanded = if self.expanded_endpoints.remove(&profile_id) {
                false
            } else {
                self.expanded_endpoints.insert(profile_id);
                true
            };
            return CatalogToggle::Endpoint {
                profile_id,
                expanded,
            };
        }
        if let Some(profile) = id.strip_prefix("db:") {
            let Ok(profile_id) = Uuid::parse_str(profile) else {
                return CatalogToggle::None;
            };
            let expanded = if self.expanded_databases.remove(&profile_id) {
                false
            } else {
                self.expanded_databases.insert(profile_id);
                true
            };
            return CatalogToggle::Database {
                profile_id,
                expanded,
            };
        }
        CatalogToggle::None
    }

    pub fn select(&mut self, id: impl Into<String>) {
        self.selected = Some(id.into());
    }
}

fn push_connection(
    rows: &mut Vec<CatalogRow>,
    catalog: &Catalog,
    profile: &ConnectionProfile,
    depth: u16,
    live: &HashSet<Uuid>,
    connecting: Option<Uuid>,
    force_open: bool,
) {
    let name_open = force_open || catalog.expanded_connections.contains(&profile.id);
    let host_open = force_open || catalog.expanded_endpoints.contains(&profile.id);
    let db_open = force_open || catalog.expanded_databases.contains(&profile.id);
    let is_live = live.contains(&profile.id);
    let loading = connecting == Some(profile.id);
    let error = catalog.connect_errors.contains_key(&profile.id);
    rows.push(CatalogRow {
        id: conn_id(profile.id),
        depth,
        label: profile.name.clone(),
        detail: profile.driver.short().to_string(),
        kind: CatalogKind::Connection,
        profile_id: Some(profile.id),
        folder_id: profile.folder_id,
        expandable: true,
        expanded: name_open,
        loading: false,
        error: false,
    });
    if !name_open {
        return;
    }
    rows.push(CatalogRow {
        id: host_id(profile.id),
        depth: depth + 1,
        label: profile.host_label(),
        detail: if loading {
            "connecting".to_string()
        } else if let Some(message) = catalog.connect_errors.get(&profile.id) {
            message.clone()
        } else if is_live {
            "connected".to_string()
        } else {
            String::new()
        },
        kind: CatalogKind::Endpoint,
        profile_id: Some(profile.id),
        folder_id: profile.folder_id,
        expandable: true,
        expanded: host_open,
        loading,
        error,
    });
    if host_open && is_live {
        rows.push(CatalogRow {
            id: db_id(profile.id),
            depth: depth + 2,
            label: profile.database_label(),
            detail: profile.driver.label().to_string(),
            kind: CatalogKind::Database,
            profile_id: Some(profile.id),
            folder_id: profile.folder_id,
            expandable: true,
            expanded: db_open,
            loading: false,
            error: false,
        });
    }
}

fn profiles_in_folder<'a>(
    profiles: &'a [ConnectionProfile],
    known_folders: &HashSet<Uuid>,
    folder_id: Option<Uuid>,
) -> Vec<&'a ConnectionProfile> {
    let mut matched: Vec<&ConnectionProfile> = profiles
        .iter()
        .filter(|profile| profile.folder_id.filter(|id| known_folders.contains(id)) == folder_id)
        .collect();
    matched.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    matched
}

fn profile_matches(profile: &ConnectionProfile, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    profile.name.to_lowercase().contains(needle)
        || profile.host_label().to_lowercase().contains(needle)
        || profile.database_label().to_lowercase().contains(needle)
        || profile.driver.label().to_lowercase().contains(needle)
}

pub fn folder_id(id: Uuid) -> String {
    format!("folder:{id}")
}

pub fn conn_id(id: Uuid) -> String {
    format!("conn:{id}")
}

pub fn host_id(id: Uuid) -> String {
    format!("host:{id}")
}

pub fn db_id(id: Uuid) -> String {
    format!("db:{id}")
}

/// Profiles whose folder was deleted are shown at the top level.
pub fn normalize_folders(folders: &[ConnectionFolder], profiles: &mut [ConnectionProfile]) {
    let known: HashSet<Uuid> = folders.iter().map(|folder| folder.id).collect();
    for profile in profiles {
        if profile.folder_id.is_some_and(|id| !known.contains(&id)) {
            profile.folder_id = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Driver;

    fn profile(name: &str, host: &str, database: &str) -> ConnectionProfile {
        let mut profile = ConnectionProfile::new(Driver::Postgres);
        profile.name = name.to_string();
        profile.host = host.to_string();
        profile.database = database.to_string();
        profile
    }

    #[test]
    fn saved_connections_show_name_then_host_until_expanded() {
        let mut local = profile("Local", "127.0.0.1", "app");
        let remote = profile("Remote", "db.internal", "warehouse");
        let folder = ConnectionFolder::new("Work");
        local.folder_id = Some(folder.id);
        let profiles = vec![remote.clone(), local.clone()];
        let catalog = Catalog::from_saved(vec![folder.clone()], &profiles);
        let rows = catalog.rows(&profiles, &HashSet::new(), None);

        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Work",
                "Local",
                "127.0.0.1:5432",
                "Remote",
                "db.internal:5432"
            ]
        );
        assert!(rows
            .iter()
            .any(|row| row.kind == CatalogKind::Folder && row.expanded));
        assert!(rows
            .iter()
            .filter(|row| row.kind == CatalogKind::Endpoint)
            .all(|row| !row.expanded));
        assert!(rows.iter().all(|row| row.kind != CatalogKind::Database));
        let _ = remote;
    }

    #[test]
    fn expanding_a_live_host_reveals_its_database() {
        let local = profile("Local", "localhost", "app");
        let mut catalog = Catalog::from_saved(Vec::new(), std::slice::from_ref(&local));
        let toggle = catalog.toggle(&host_id(local.id));
        assert_eq!(
            toggle,
            CatalogToggle::Endpoint {
                profile_id: local.id,
                expanded: true,
            }
        );
        let mut live = HashSet::new();
        live.insert(local.id);
        let rows = catalog.rows(std::slice::from_ref(&local), &live, None);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["Local", "localhost:5432", "app"]);
        assert_eq!(rows[1].detail, "connected");
        assert_eq!(rows[2].kind, CatalogKind::Database);
        catalog.expanded_databases.insert(local.id);
        let rows = catalog.rows(&[local], &live, None);
        assert!(rows
            .iter()
            .any(|row| row.kind == CatalogKind::Database && row.expanded));
    }

    #[test]
    fn collapsing_the_host_hides_the_database_without_forgetting_it() {
        let local = profile("Local", "localhost", "app");
        let mut catalog = Catalog::from_saved(Vec::new(), std::slice::from_ref(&local));
        catalog.expanded_endpoints.insert(local.id);
        catalog.expanded_databases.insert(local.id);
        let toggle = catalog.toggle(&host_id(local.id));
        assert_eq!(
            toggle,
            CatalogToggle::Endpoint {
                profile_id: local.id,
                expanded: false,
            }
        );
        let mut live = HashSet::new();
        live.insert(local.id);
        let rows = catalog.rows(&[local], &live, None);
        assert!(rows.iter().all(|row| row.kind != CatalogKind::Database));
        assert!(catalog.expanded_databases.contains(
            &rows
                .iter()
                .find(|row| row.kind == CatalogKind::Connection)
                .and_then(|row| row.profile_id)
                .unwrap()
        ));
    }

    #[test]
    fn missing_folder_puts_the_connection_at_the_top() {
        let mut local = profile("Local", "localhost", "app");
        local.folder_id = Some(Uuid::new_v4());
        let mut profiles = vec![local];
        normalize_folders(&[], &mut profiles);
        assert!(profiles[0].folder_id.is_none());
        let catalog = Catalog::from_saved(Vec::new(), &profiles);
        let rows = catalog.rows(&profiles, &HashSet::new(), None);
        assert_eq!(rows[0].kind, CatalogKind::Connection);
        assert_eq!(rows[0].depth, 0);
    }

    #[test]
    fn filter_keeps_the_matching_connection_and_its_host() {
        let local = profile("Local", "localhost", "app");
        let remote = profile("Billing", "10.0.0.8", "invoices");
        let mut catalog = Catalog::from_saved(Vec::new(), &[local.clone(), remote.clone()]);
        catalog.filter = "10.0.0.8".to_string();
        let rows = catalog.rows(&[local, remote], &HashSet::new(), None);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["Billing", "10.0.0.8:5432"]);
    }
}
