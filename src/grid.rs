//! Interactive result-grid state: virtual scrolling, selection, client-side
//! filtering/sorting and column sizing.
//!
//! Only visible rows are ever rendered by the UI; the full result can contain
//! thousands of rows without affecting frame time.

use std::cmp::Ordering;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::model::QueryResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub fn arrow(self) -> &'static str {
        match self {
            SortDirection::Asc => "▲",
            SortDirection::Desc => "▼",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub column: usize,
    pub direction: SortDirection,
}

#[derive(Debug, Clone)]
pub struct ResultGrid {
    /// Selected row as an index into `order` (the filtered/sorted view).
    pub sel_row: usize,
    pub sel_col: usize,
    pub top_row: usize,
    pub h_col: usize,
    pub widths: Vec<u16>,
    pub sort: Option<SortSpec>,
    pub filter: String,
    pub order: Vec<usize>,
    pub view_rows: u16,
    pub view_width: u16,
}

impl ResultGrid {
    pub fn new(result: &QueryResult) -> Self {
        let widths = result
            .columns
            .iter()
            .enumerate()
            .map(|(idx, col)| default_width(col.name.as_str(), idx, result))
            .collect();
        let order: Vec<usize> = (0..result.rows.len()).collect();
        Self {
            sel_row: 0,
            sel_col: 0,
            top_row: 0,
            h_col: 0,
            widths,
            sort: None,
            filter: String::new(),
            order,
            view_rows: 10,
            view_width: 80,
        }
    }

    pub fn visible_count(&self) -> usize {
        self.order.len()
    }

    pub fn total_count(&self) -> usize {
        // `order` is rebuilt from the result, so this is the visible count.
        self.order.len()
    }

    pub fn is_filtered(&self) -> bool {
        !self.filter.trim().is_empty()
    }

    pub fn selected_raw_row(&self) -> Option<usize> {
        self.order.get(self.sel_row).copied()
    }

    pub fn row_at(&self, display_idx: usize) -> Option<usize> {
        self.order.get(display_idx).copied()
    }

    /// Recompute the visible order after filtering/sorting. The currently
    /// selected *raw* row is preserved when possible.
    pub fn rebuild(&mut self, result: &QueryResult, preserve_raw: Option<usize>) {
        if self.widths.len() != result.columns.len() {
            self.widths = result
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| default_width(col.name.as_str(), idx, result))
                .collect();
        }
        if let Some(sort) = self.sort {
            if sort.column >= result.columns.len() {
                self.sort = None;
            }
        }

        let needle = self.filter.trim().to_lowercase();
        let mut order: Vec<usize> = if needle.is_empty() {
            (0..result.rows.len()).collect()
        } else {
            (0..result.rows.len())
                .filter(|&row_idx| {
                    result.rows[row_idx].iter().any(|cell| {
                        cell.as_ref()
                            .map(|v| v.to_lowercase().contains(&needle))
                            .unwrap_or(false)
                    })
                })
                .collect()
        };

        if let Some(sort) = self.sort {
            order.sort_by(|&a, &b| {
                let av = result.rows[a].get(sort.column).and_then(|v| v.as_deref());
                let bv = result.rows[b].get(sort.column).and_then(|v| v.as_deref());
                match (av, bv) {
                    // NULLs stay at the bottom in both directions.
                    (None, None) => Ordering::Equal,
                    (None, Some(_)) => Ordering::Greater,
                    (Some(_), None) => Ordering::Less,
                    (Some(av), Some(bv)) => {
                        let cmp = compare_values(av, bv);
                        match sort.direction {
                            SortDirection::Asc => cmp,
                            SortDirection::Desc => cmp.reverse(),
                        }
                    }
                }
            });
        }

        self.order = order;
        self.sel_row = preserve_raw
            .and_then(|raw| self.order.iter().position(|&r| r == raw))
            .unwrap_or(0)
            .min(self.order.len().saturating_sub(1));
        self.sel_col = self.sel_col.min(result.columns.len().saturating_sub(1));
        self.top_row = 0;
        self.h_col = 0;
    }

    /// Set the filter and rebuild, preserving the selected raw row.
    pub fn set_filter(&mut self, result: &QueryResult, filter: String) {
        let raw = self.selected_raw_row();
        self.filter = filter;
        self.rebuild(result, raw);
    }

    pub fn clear_filter(&mut self, result: &QueryResult) {
        self.set_filter(result, String::new());
    }

    /// Cycle column sorting: unsorted -> ascending -> descending -> unsorted.
    pub fn cycle_sort(&mut self, result: &QueryResult) {
        let raw = self.selected_raw_row();
        self.sort = match self.sort {
            Some(SortSpec {
                column,
                direction: SortDirection::Asc,
            }) if column == self.sel_col => Some(SortSpec {
                column,
                direction: SortDirection::Desc,
            }),
            Some(SortSpec { column, .. }) if column == self.sel_col => None,
            _ => Some(SortSpec {
                column: self.sel_col,
                direction: SortDirection::Asc,
            }),
        };
        self.rebuild(result, raw);
    }

    pub fn move_cell(&mut self, d_row: isize, d_col: isize, result: &QueryResult) {
        let max_row = self.order.len().saturating_sub(1);
        let max_col = result.columns.len().saturating_sub(1);
        let row = self.sel_row as isize + d_row;
        self.sel_row = row.clamp(0, max_row as isize) as usize;
        let col = self.sel_col as isize + d_col;
        self.sel_col = col.clamp(0, max_col as isize) as usize;
        self.ensure_visible();
    }

    pub fn page(&mut self, delta: isize, result: &QueryResult) {
        let step = self.view_rows.saturating_sub(2).max(1) as isize;
        self.move_cell(delta.saturating_mul(step), 0, result);
    }

    pub fn jump_row(&mut self, row: usize) {
        self.sel_row = row.min(self.order.len().saturating_sub(1));
        self.ensure_visible();
    }

    pub fn jump_column(&mut self, col: usize, result: &QueryResult) {
        self.sel_col = col.min(result.columns.len().saturating_sub(1));
        self.ensure_visible();
    }

    pub fn ensure_visible(&mut self) {
        let rows = self.view_rows.max(1) as usize;
        if self.sel_row < self.top_row {
            self.top_row = self.sel_row;
        } else if self.sel_row >= self.top_row + rows {
            self.top_row = self.sel_row + 1 - rows;
        }
        let max_top = self.order.len().saturating_sub(rows);
        self.top_row = self.top_row.min(max_top);

        if self.sel_col < self.h_col {
            self.h_col = self.sel_col;
        }
        let available = self.view_width.saturating_sub(7).max(1) as usize;
        while self.h_col < self.sel_col {
            let used: usize = (self.h_col..=self.sel_col)
                .map(|i| self.widths.get(i).copied().unwrap_or(10) as usize + 1)
                .sum();
            if used <= available {
                break;
            }
            self.h_col += 1;
        }
    }

    pub fn resize_selected_column(&mut self, delta: i16) {
        if let Some(width) = self.widths.get_mut(self.sel_col) {
            *width = (*width as i16 + delta).clamp(4, 100) as u16;
        }
    }

    pub fn scroll_horizontal(&mut self, delta: isize, result: &QueryResult) {
        let max = result.columns.len().saturating_sub(1);
        self.h_col = (self.h_col as isize + delta).clamp(0, max as isize) as usize;
    }

    pub fn scroll_vertical(&mut self, delta: isize) {
        let max_top = self
            .order
            .len()
            .saturating_sub(self.view_rows.max(1) as usize);
        self.top_row = (self.top_row as isize + delta).clamp(0, max_top as isize) as usize;
    }

    /// Selected row as tab-separated text (headers optional).
    pub fn row_tsv(&self, result: &QueryResult, headers: bool) -> Option<String> {
        let raw = self.selected_raw_row()?;
        let row = result.rows.get(raw)?;
        let mut lines = Vec::new();
        if headers {
            lines.push(
                result
                    .columns
                    .iter()
                    .map(|c| sanitize_field(&c.name))
                    .collect::<Vec<_>>()
                    .join("\t"),
            );
        }
        lines.push(
            row.iter()
                .map(|v| sanitize_field(v.as_deref().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\t"),
        );
        Some(lines.join("\n"))
    }

    /// Entire visible view as TSV (used for Ctrl+Y "copy all").
    pub fn all_tsv(&self, result: &QueryResult) -> String {
        let mut out = String::new();
        out.push_str(
            &result
                .columns
                .iter()
                .map(|c| sanitize_field(&c.name))
                .collect::<Vec<_>>()
                .join("\t"),
        );
        for &raw in &self.order {
            out.push('\n');
            if let Some(row) = result.rows.get(raw) {
                out.push_str(
                    &row.iter()
                        .map(|v| sanitize_field(v.as_deref().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join("\t"),
                );
            }
        }
        out
    }

    pub fn sort_label(&self) -> Option<String> {
        self.sort.map(|s| {
            format!(
                "{} {}",
                s.column, // replaced by caller with a column name when available
                s.direction.arrow()
            )
        })
    }
}

fn default_width(header: &str, idx: usize, result: &QueryResult) -> u16 {
    let header_w = UnicodeWidthStr::width(header).min(40);
    let mut sample = 0usize;
    for row in result.rows.iter().take(80) {
        if let Some(Some(cell)) = row.get(idx) {
            let width = cell.lines().map(UnicodeWidthStr::width).max().unwrap_or(0);
            sample = sample.max(width.min(40));
        }
    }
    (header_w.max(sample).max(6) as u16 + 2).clamp(6, 42)
}

fn compare_values(a: &str, b: &str) -> Ordering {
    let a_trim = a.trim();
    let b_trim = b.trim();
    if let (Ok(a_num), Ok(b_num)) = (a_trim.parse::<f64>(), b_trim.parse::<f64>()) {
        return a_num.partial_cmp(&b_num).unwrap_or(Ordering::Equal);
    }
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Make a value safe for a single spreadsheet row.
pub fn sanitize_field(value: &str) -> String {
    value.replace('\t', "    ").replace(['\r', '\n'], " ")
}

/// Truncate by display width, appending an ellipsis when clipped.
pub fn truncate_display(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(value) <= width {
        return sanitize_field(value);
    }
    let mut out = String::new();
    let mut used = 0usize;
    let limit = width.saturating_sub(1);
    for ch in value.chars() {
        let ch = if ch == '\n' || ch == '\r' || ch == '\t' {
            ' '
        } else {
            ch
        };
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > limit {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// Pad/truncate a string to exactly `width` display columns.
pub fn fit_display(value: &str, width: usize) -> String {
    let truncated = truncate_display(value, width);
    let used = UnicodeWidthStr::width(truncated.as_str());
    let mut out = truncated;
    if used < width {
        out.push_str(&" ".repeat(width - used));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ColumnMeta;

    fn sample() -> QueryResult {
        QueryResult {
            columns: vec![
                ColumnMeta {
                    name: "id".into(),
                    type_name: "INT4".into(),
                },
                ColumnMeta {
                    name: "name".into(),
                    type_name: "TEXT".into(),
                },
            ],
            rows: vec![
                vec![Some("10".into()), Some("Zoe".into())],
                vec![Some("2".into()), Some("alice".into())],
                vec![None, Some("bob".into())],
            ],
            affected: 0,
            duration_ms: 4,
            truncated: false,
            sql: "select * from t".into(),
            result_sets: 1,
        }
    }

    #[test]
    fn numeric_sorting_nulls_last() {
        let result = sample();
        let mut grid = ResultGrid::new(&result);
        grid.sel_col = 0;
        grid.cycle_sort(&result); // asc
        let sorted: Vec<usize> = grid.order.clone();
        assert_eq!(sorted, vec![1, 0, 2]);
        grid.cycle_sort(&result); // desc
        assert_eq!(grid.order, vec![0, 1, 2]);
    }

    #[test]
    fn filter_case_insensitive() {
        let result = sample();
        let mut grid = ResultGrid::new(&result);
        grid.set_filter(&result, "AL".into());
        assert_eq!(grid.order, vec![1]);
        assert_eq!(grid.visible_count(), 1);
    }

    #[test]
    fn copy_row_uses_headers() {
        let result = sample();
        let grid = ResultGrid::new(&result);
        let out = grid.row_tsv(&result, true).unwrap();
        assert_eq!(out, "id\tname\n10\tZoe");
    }

    #[test]
    fn selection_stays_in_bounds() {
        let result = sample();
        let mut grid = ResultGrid::new(&result);
        grid.move_cell(-5, -5, &result);
        assert_eq!((grid.sel_row, grid.sel_col), (0, 0));
        grid.move_cell(100, 100, &result);
        assert_eq!((grid.sel_row, grid.sel_col), (2, 1));
    }

    #[test]
    fn truncate_by_display_width() {
        assert_eq!(truncate_display("hello", 10), "hello");
        assert_eq!(truncate_display("hello", 4), "hel…");
        // Wide CJK characters consume two columns.
        assert_eq!(
            UnicodeWidthStr::width(truncate_display("你好世界", 5).as_str()),
            5
        );
    }
}
