//! State for the collapsible schema explorer.
//!
//! Schemas/tables are loaded on connect; table metadata (columns, indexes,
//! foreign keys) is fetched lazily when a table is expanded. The tree flattens
//! itself into lightweight render rows, which also makes keyboard navigation
//! and mouse hit-testing straightforward.

use crate::model::{ColumnInfo, ForeignKeyInfo, IndexInfo, ObjectKind, SchemaEntry, TableDetails};

const SEP: char = '\u{1f}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKind {
    Tables,
    Views,
    Columns,
    Indexes,
    ForeignKeys,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Root,
    Schema,
    Folder(FolderKind),
    Table {
        schema: String,
        name: String,
        object: ObjectKind,
    },
    Column {
        info: ColumnInfo,
    },
    Index {
        table: String,
        info: IndexInfo,
    },
    ForeignKey {
        table: String,
        info: ForeignKeyInfo,
    },
}

#[derive(Debug, Clone)]
pub struct SchemaNode {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    pub expanded: bool,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub children: Vec<SchemaNode>,
}

impl SchemaNode {
    fn new(id: String, label: String, kind: NodeKind) -> Self {
        Self {
            id,
            label,
            kind,
            expanded: false,
            loading: false,
            loaded: false,
            error: None,
            children: Vec::new(),
        }
    }

    fn expandable(&self) -> bool {
        !matches!(
            self.kind,
            NodeKind::Column { .. } | NodeKind::Index { .. } | NodeKind::ForeignKey { .. }
        )
    }

    fn matches(&self, needle: &str) -> bool {
        if self.label.to_lowercase().contains(needle) {
            return true;
        }
        match &self.kind {
            NodeKind::Column { info } => {
                info.data_type.to_lowercase().contains(needle)
                    || info.key.to_lowercase().contains(needle)
            }
            NodeKind::Index { info, .. } => {
                info.columns.join(" ").to_lowercase().contains(needle)
                    || info.name.to_lowercase().contains(needle)
            }
            NodeKind::ForeignKey { info, .. } => format!(
                "{} {}.{} {}",
                info.name,
                info.ref_schema,
                info.ref_table,
                info.ref_columns.join(" ")
            )
            .to_lowercase()
            .contains(needle),
            NodeKind::Table { .. } | NodeKind::Folder(_) | NodeKind::Schema | NodeKind::Root => {
                false
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Root,
    Schema,
    Folder,
    Table,
    View,
    Column,
    Index,
    ForeignKey,
}

/// A flattened, UI-ready tree row.
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub id: String,
    pub depth: u16,
    pub label: String,
    pub detail: String,
    pub marker: String,
    pub kind: RowKind,
    pub expandable: bool,
    pub expanded: bool,
    pub loading: bool,
    pub error: bool,
}

#[derive(Debug, Clone)]
pub struct TableLoadRequest {
    pub node_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub enum TreeAction {
    Preview {
        schema: String,
        table: String,
        object: ObjectKind,
    },
    InsertColumn {
        name: String,
    },
    ShowIndex {
        table: String,
        info: IndexInfo,
    },
    ShowForeignKey {
        table: String,
        info: ForeignKeyInfo,
    },
    Toggled,
    None,
}

#[derive(Debug, Default)]
pub struct SchemaTree {
    pub roots: Vec<SchemaNode>,
    pub rows: Vec<TreeRow>,
    pub selected_id: Option<String>,
    pub scroll: u16,
    pub filter: String,
}

impl SchemaTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.roots.clear();
        self.rows.clear();
        self.selected_id = None;
        self.scroll = 0;
    }

    /// Build roots from initial metadata. A single schema is expanded
    /// automatically (along with its table folder) for faster navigation.
    pub fn load_schemas(&mut self, schemas: &[SchemaEntry], root_label: &str, root_detail: &str) {
        let mut root = SchemaNode::new("root".to_string(), root_label.to_string(), NodeKind::Root);
        root.expanded = true;

        let single_schema = schemas.len() == 1;
        for schema in schemas {
            let schema_id = format!("schema{SEP}{}", schema.name);
            let mut schema_node =
                SchemaNode::new(schema_id.clone(), schema.name.clone(), NodeKind::Schema);
            schema_node.expanded = single_schema;

            let mut tables = Vec::new();
            let mut views = Vec::new();
            for object in &schema.objects {
                let id = format!("table{SEP}{}{SEP}{}", schema.name, object.name);
                let mut table_node = SchemaNode::new(
                    id,
                    object.name.clone(),
                    NodeKind::Table {
                        schema: schema.name.clone(),
                        name: object.name.clone(),
                        object: object.kind,
                    },
                );
                match object.kind {
                    ObjectKind::Table => tables.push(table_node),
                    ObjectKind::View => {
                        table_node.expanded = false;
                        views.push(table_node);
                    }
                }
            }

            let mut tables_folder = SchemaNode::new(
                format!("folder{SEP}{}{SEP}tables", schema.name),
                "Tables".to_string(),
                NodeKind::Folder(FolderKind::Tables),
            );
            tables_folder.loaded = true;
            tables_folder.expanded = single_schema;
            tables_folder.children = tables;

            let mut views_folder = SchemaNode::new(
                format!("folder{SEP}{}{SEP}views", schema.name),
                "Views".to_string(),
                NodeKind::Folder(FolderKind::Views),
            );
            views_folder.loaded = true;
            // Expanding both folders for a single schema gives an immediately
            // useful overview while still keeping multi-schema trees compact.
            views_folder.expanded = single_schema;
            views_folder.children = views;

            if !tables_folder.children.is_empty() {
                schema_node.children.push(tables_folder);
            }
            if !views_folder.children.is_empty() {
                schema_node.children.push(views_folder);
            }
            root.children.push(schema_node);
        }

        self.roots = vec![root];
        self.selected_id = None;
        self.scroll = 0;
        self.rebuild_rows(root_detail);
        // Prefer a table as the initial selection so Enter immediately previews.
        if let Some(first_table) = self
            .rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::Table | RowKind::View))
        {
            self.selected_id = Some(first_table.id.clone());
        } else {
            self.selected_id = self.rows.first().map(|r| r.id.clone());
        }
        self.rebuild_rows(root_detail);
    }

    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.rebuild_rows("");
    }

    pub fn is_filtered(&self) -> bool {
        !self.filter.trim().is_empty()
    }

    fn rebuild_rows(&mut self, _root_detail: &str) {
        let needle = self.filter.trim().to_lowercase();
        let mut rows = Vec::new();
        if needle.is_empty() {
            for root in &self.roots {
                flatten_normal(root, 0, &mut rows);
            }
        } else {
            for root in &self.roots {
                flatten_filtered(root, 0, &needle, &mut rows);
            }
        }
        self.rows = rows;
        if self.selected_id.is_none()
            || !self
                .rows
                .iter()
                .any(|r| Some(&r.id) == self.selected_id.as_ref())
        {
            self.selected_id = self.rows.first().map(|r| r.id.clone());
        }
        let max_scroll = self.rows.len().saturating_sub(1) as u16;
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn selected_index(&self) -> Option<usize> {
        let selected = self.selected_id.as_ref()?;
        self.rows.iter().position(|r| &r.id == selected)
    }

    pub fn select_row(&mut self, index: usize) {
        if let Some(row) = self.rows.get(index) {
            self.selected_id = Some(row.id.clone());
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let current = self.selected_index().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.rows.len().saturating_sub(1) as isize) as usize;
        self.select_row(next);
    }

    pub fn page_selection(&mut self, down: bool, page: usize) {
        let step = page.max(1) as isize;
        self.move_selection(if down { step } else { -step });
    }

    pub fn select_first(&mut self) {
        if !self.rows.is_empty() {
            self.select_row(0);
        }
    }

    pub fn select_last(&mut self) {
        if !self.rows.is_empty() {
            self.select_row(self.rows.len() - 1);
        }
    }

    pub fn ensure_selected_visible(&mut self, height: u16) {
        let Some(idx) = self.selected_index() else {
            return;
        };
        let idx = idx as u16;
        let height = height.max(1);
        if idx < self.scroll {
            self.scroll = idx;
        } else if idx >= self.scroll.saturating_add(height) {
            self.scroll = idx + 1 - height;
        }
        let max_scroll = self.rows.len().saturating_sub(1) as u16;
        self.scroll = self.scroll.min(max_scroll);
    }

    /// Enter behaviour. Tables preview their data; folders toggle; columns
    /// insert their name; indexes/FKs open a details panel.
    pub fn activate_selected(&mut self) -> TreeAction {
        let Some(node) = self.selected_node().cloned() else {
            return TreeAction::None;
        };
        match &node.kind {
            NodeKind::Table {
                schema,
                name,
                object,
            } => TreeAction::Preview {
                schema: schema.clone(),
                table: name.clone(),
                object: *object,
            },
            NodeKind::Column { info } => TreeAction::InsertColumn {
                name: info.name.clone(),
            },
            NodeKind::Index { table, info } => TreeAction::ShowIndex {
                table: table.clone(),
                info: info.clone(),
            },
            NodeKind::ForeignKey { table, info } => TreeAction::ShowForeignKey {
                table: table.clone(),
                info: info.clone(),
            },
            NodeKind::Root | NodeKind::Schema | NodeKind::Folder(_) => {
                self.toggle_expansion(&node.id);
                TreeAction::Toggled
            }
        }
    }

    /// Right arrow / Space: expand a node, fetching table metadata if needed.
    pub fn expand_selected(&mut self) -> Option<TableLoadRequest> {
        let node = self.selected_node()?.clone();
        match &node.kind {
            NodeKind::Root | NodeKind::Schema | NodeKind::Folder(_) => {
                if let Some(node) = find_node_mut(&mut self.roots, &node.id) {
                    node.expanded = true;
                }
                self.rebuild_rows("");
                None
            }
            NodeKind::Table { schema, name, .. } => {
                if node.loaded {
                    if let Some(node) = find_node_mut(&mut self.roots, &node.id) {
                        node.expanded = true;
                    }
                    self.rebuild_rows("");
                    return None;
                }
                if node.loading {
                    return None;
                }
                if let Some(n) = find_node_mut(&mut self.roots, &node.id) {
                    n.expanded = true;
                    n.loading = true;
                    n.error = None;
                }
                self.rebuild_rows("");
                Some(TableLoadRequest {
                    node_id: node.id,
                    schema: schema.clone(),
                    table: name.clone(),
                })
            }
            NodeKind::Column { .. } | NodeKind::Index { .. } | NodeKind::ForeignKey { .. } => None,
        }
    }

    /// Left arrow: collapse, or move to the parent when already collapsed.
    pub fn collapse_selected(&mut self) {
        let Some(node) = self.selected_node().cloned() else {
            return;
        };
        if node.expandable() && node.expanded {
            if let Some(n) = find_node_mut(&mut self.roots, &node.id) {
                n.expanded = false;
            }
            self.rebuild_rows("");
        }
    }

    pub fn toggle_expansion(&mut self, id: &str) {
        let Some(node) = self.selected_node().cloned() else {
            return;
        };
        if node.id != id {
            return;
        }
        if !node.expandable() {
            return;
        }
        if node.expanded {
            if let Some(n) = find_node_mut(&mut self.roots, id) {
                n.expanded = false;
            }
            self.rebuild_rows("");
        } else {
            // Use the same lazy-loading path as the right arrow.
            let _ = self.expand_selected();
        }
    }

    pub fn selected_node(&self) -> Option<&SchemaNode> {
        let id = self.selected_id.as_ref()?;
        find_node(&self.roots, id)
    }

    /// Called when lazy table metadata arrives.
    pub fn complete_table_load(&mut self, node_id: &str, details: Result<TableDetails, String>) {
        let Some(node) = find_node_mut(&mut self.roots, node_id) else {
            return;
        };
        node.loading = false;
        match details {
            Ok(details) => {
                node.loaded = true;
                node.error = None;
                node.children = build_table_children(node_id, &node.label, details);
                let expanded = node.expanded;
                for child in &mut node.children {
                    if matches!(child.kind, NodeKind::Folder(FolderKind::Columns)) && expanded {
                        child.expanded = true;
                    }
                }
            }
            Err(error) => {
                node.loaded = false;
                node.error = Some(error);
            }
        }
        self.rebuild_rows("");
    }

    pub fn retry_clear_error(&mut self) {
        if let Some(node) = self.selected_node().cloned() {
            if let Some(n) = find_node_mut(&mut self.roots, &node.id) {
                n.error = None;
            }
            self.rebuild_rows("");
        }
    }

    /// Rebuild rows after filters/detail changes; useful for the UI after
    /// direct mutable operations.
    pub fn refresh_rows(&mut self) {
        self.rebuild_rows("");
    }
}

fn build_table_children(
    node_id: &str,
    table_label: &str,
    details: TableDetails,
) -> Vec<SchemaNode> {
    let mut columns = SchemaNode::new(
        format!("{node_id}{SEP}columns"),
        "Columns".to_string(),
        NodeKind::Folder(FolderKind::Columns),
    );
    columns.loaded = true;
    columns.children = details
        .columns
        .into_iter()
        .map(|info| {
            let id = format!("{node_id}{SEP}col{SEP}{}", info.name);
            SchemaNode::new(id, info.name.clone(), NodeKind::Column { info })
        })
        .collect();

    let mut indexes = SchemaNode::new(
        format!("{node_id}{SEP}indexes"),
        "Indexes".to_string(),
        NodeKind::Folder(FolderKind::Indexes),
    );
    indexes.loaded = true;
    indexes.children = details
        .indexes
        .into_iter()
        .map(|info| {
            let id = format!("{node_id}{SEP}idx{SEP}{}", info.name);
            SchemaNode::new(
                id,
                info.name.clone(),
                NodeKind::Index {
                    table: table_label.to_string(),
                    info,
                },
            )
        })
        .collect();

    let mut fks = SchemaNode::new(
        format!("{node_id}{SEP}fks"),
        "Foreign Keys".to_string(),
        NodeKind::Folder(FolderKind::ForeignKeys),
    );
    fks.loaded = true;
    fks.children = details
        .foreign_keys
        .into_iter()
        .map(|info| {
            let id = format!("{node_id}{SEP}fk{SEP}{}", info.name);
            SchemaNode::new(
                id,
                info.name.clone(),
                NodeKind::ForeignKey {
                    table: table_label.to_string(),
                    info,
                },
            )
        })
        .collect();

    vec![columns, indexes, fks]
}

// ---------- Flattening -----------------------------------------------------

fn row_for(node: &SchemaNode, depth: u16, expanded_override: Option<bool>) -> TreeRow {
    let (kind, detail, marker) = match &node.kind {
        NodeKind::Root => (RowKind::Root, String::new(), String::new()),
        NodeKind::Schema => {
            let count: usize = node
                .children
                .iter()
                .filter(|c| {
                    matches!(
                        c.kind,
                        NodeKind::Folder(FolderKind::Tables | FolderKind::Views)
                    )
                })
                .map(|c| c.children.len())
                .sum();
            (
                RowKind::Schema,
                format!("{count} object{}", if count == 1 { "" } else { "s" }),
                String::new(),
            )
        }
        NodeKind::Folder(folder) => {
            let label = match folder {
                FolderKind::Tables => "tables",
                FolderKind::Views => "views",
                FolderKind::Columns => "columns",
                FolderKind::Indexes => "indexes",
                FolderKind::ForeignKeys => "foreign keys",
            };
            let _ = label;
            (
                RowKind::Folder,
                format!("{}", node.children.len()),
                String::new(),
            )
        }
        NodeKind::Table { object, .. } => (
            if object.is_view() {
                RowKind::View
            } else {
                RowKind::Table
            },
            if object.is_view() {
                "view".to_string()
            } else {
                String::new()
            },
            String::new(),
        ),
        NodeKind::Column { info } => (
            RowKind::Column,
            format!(
                "{}{}",
                info.data_type,
                if info.nullable { "" } else { " · not null" }
            ),
            info.key.clone(),
        ),
        NodeKind::Index { info, .. } => (
            RowKind::Index,
            format!("{} · {}", info.method, info.columns.join(", ")),
            if info.primary {
                "PK".to_string()
            } else if info.unique {
                "UQ".to_string()
            } else {
                String::new()
            },
        ),
        NodeKind::ForeignKey { info, .. } => (
            RowKind::ForeignKey,
            format!(
                "-> {}.{}({})",
                info.ref_schema,
                info.ref_table,
                info.ref_columns.join(", ")
            ),
            if info.on_delete == "NO ACTION" {
                "FK".to_string()
            } else {
                format!("FK {}", info.on_delete)
            },
        ),
    };

    TreeRow {
        id: node.id.clone(),
        depth,
        label: node.label.clone(),
        detail,
        marker,
        kind,
        expandable: node.expandable(),
        expanded: expanded_override.unwrap_or(node.expanded),
        loading: node.loading,
        error: node.error.is_some(),
    }
}

fn flatten_normal(node: &SchemaNode, depth: u16, out: &mut Vec<TreeRow>) {
    out.push(row_for(node, depth, None));
    if node.expanded {
        for child in &node.children {
            flatten_normal(child, depth + 1, out);
        }
    }
}

fn flatten_filtered(node: &SchemaNode, depth: u16, needle: &str, out: &mut Vec<TreeRow>) -> bool {
    let direct = node.matches(needle);
    let mut children = Vec::new();
    for child in &node.children {
        flatten_filtered(child, depth + 1, needle, &mut children);
    }
    if direct || !children.is_empty() {
        let mut row = row_for(node, depth, None);
        // Search results are shown expanded regardless of saved state.
        row.expanded = !children.is_empty() || row.expanded;
        out.push(row);
        out.extend(children);
        true
    } else {
        false
    }
}

// ---------- Lookup helpers -------------------------------------------------

fn find_node<'a>(nodes: &'a [SchemaNode], id: &str) -> Option<&'a SchemaNode> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_node(&node.children, id) {
            return Some(found);
        }
    }
    None
}

fn find_node_mut<'a>(nodes: &'a mut [SchemaNode], id: &str) -> Option<&'a mut SchemaNode> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_node_mut(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TableEntry;

    fn sample_schemas() -> Vec<SchemaEntry> {
        vec![SchemaEntry {
            name: "public".into(),
            objects: vec![
                TableEntry {
                    schema: "public".into(),
                    name: "users".into(),
                    kind: ObjectKind::Table,
                },
                TableEntry {
                    schema: "public".into(),
                    name: "active_users".into(),
                    kind: ObjectKind::View,
                },
            ],
        }]
    }

    #[test]
    fn loads_and_expands_single_schema() {
        let mut tree = SchemaTree::new();
        tree.load_schemas(&sample_schemas(), "app", "PostgreSQL");
        assert!(tree.rows.iter().any(|r| r.label == "users"));
        assert!(tree.rows.iter().any(|r| r.kind == RowKind::View));
        assert!(tree.selected_id.is_some());
    }

    #[test]
    fn filtering_finds_ancestors() {
        let mut tree = SchemaTree::new();
        tree.load_schemas(&sample_schemas(), "app", "PostgreSQL");
        tree.set_filter("active".into());
        let labels: Vec<&str> = tree.rows.iter().map(|r| r.label.as_str()).collect();
        assert!(labels.contains(&"active_users"));
        assert!(!labels.contains(&"users"));
        assert!(labels.contains(&"public"));
    }

    #[test]
    fn table_expansion_requests_load() {
        let mut tree = SchemaTree::new();
        tree.load_schemas(&sample_schemas(), "app", "PostgreSQL");
        // Select the users table.
        let idx = tree.rows.iter().position(|r| r.label == "users").unwrap();
        tree.select_row(idx);
        let req = tree.expand_selected().expect("load request");
        assert_eq!(req.table, "users");
        assert!(tree.selected_node().unwrap().loading);

        tree.complete_table_load(
            &req.node_id,
            Ok(TableDetails {
                columns: vec![crate::model::ColumnInfo {
                    name: "id".into(),
                    data_type: "integer".into(),
                    nullable: false,
                    default: None,
                    key: "PK".into(),
                    ordinal: 1,
                }],
                ..TableDetails::default()
            }),
        );
        assert!(tree.rows.iter().any(|r| r.label == "id"));
    }

    #[test]
    fn activate_table_previews() {
        let mut tree = SchemaTree::new();
        tree.load_schemas(&sample_schemas(), "app", "PostgreSQL");
        let idx = tree.rows.iter().position(|r| r.label == "users").unwrap();
        tree.select_row(idx);
        match tree.activate_selected() {
            TreeAction::Preview { table, .. } => assert_eq!(table, "users"),
            other => panic!("unexpected action: {other:?}"),
        }
    }
}
