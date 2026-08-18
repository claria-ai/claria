//! The one rule for reading a Word table's merged geometry.
//!
//! A `w:gridSpan` cell covers several columns of the table grid and a
//! `w:vMerge` continuation is painted over by the `restart` cell above it, so
//! a merged table is a rectangle whose cells do not line up one-to-one with
//! its grid positions. Both halves of the pipeline need that rectangle:
//! import turns it into a [`claria_core::models::report::ReportBlock::Table`],
//! and export has to recognise the same table as a flow span to write a
//! draft's rows back into it.
//!
//! The two read the same table from different representations — import walks
//! docx-rs structures, export walks XML events — so the rule is **shared**,
//! not mirrored: each side reduces its own representation to [`CellGeometry`]
//! and hands it here. A rule stated twice is a rule that drifts, and here
//! drift is not cosmetic: a table one half rectangularises and the other does
//! not becomes a block the draft holds and the export cannot place.

use claria_core::models::report::MAX_TABLE_COLUMNS;

/// How one cell occupies its row's grid positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellGeometry {
    /// `w:gridSpan` — how many grid columns this cell covers. 1 when absent.
    pub(crate) span: usize,
    /// `w:vMerge` continues a vertical merge started in a row above. Word
    /// paints the `restart` cell's content across the whole merge and never
    /// shows this cell's own, so the position carries no text of its own.
    pub(crate) continues_merge: bool,
}

impl CellGeometry {
    /// One column, no merge — what a cell with no `w:tcPr` geometry is.
    pub(crate) const PLAIN: Self = Self {
        span: 1,
        continues_merge: false,
    };
}

/// One row expanded to grid positions: the index of the row's cell that owns
/// each position, or `None` where a merge covers it.
///
/// A covered position holds the empty string on import and accepts only an
/// empty draft value on export — nothing written there would be visible.
pub(crate) type GridRow = Vec<Option<usize>>;

/// What a table's cell geometry expands to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TableGeometry {
    /// A rectangle: one [`GridRow`] per table row, all the same width.
    Rectangular(Vec<GridRow>),
    /// Merged geometry that does not expand to a rectangle — the case
    /// `MergedTablesOmitted` still reports.
    Merged,
    /// No merges, and the rows are not a usable rectangle.
    Irregular,
}

/// Expand a table's rows into rectangular grid positions.
///
/// `declared_columns` is the `w:tblGrid` column count, and it is consulted
/// only for tables that actually merge. Word leaves stale `w:gridCol`
/// entries behind on ordinary tables often enough that trusting the grid
/// everywhere would reject documents that import fine today, while for a
/// merged table the grid is the only statement of how wide its rows are
/// meant to be — and the only way to catch a `gridSpan` that lies.
pub(crate) fn expand_rows(
    rows: &[Vec<CellGeometry>],
    declared_columns: Option<usize>,
) -> TableGeometry {
    let merged = rows
        .iter()
        .flatten()
        .any(|geometry| *geometry != CellGeometry::PLAIN);
    let refusal = if merged {
        TableGeometry::Merged
    } else {
        TableGeometry::Irregular
    };

    let mut grid: Vec<GridRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let mut positions: GridRow = Vec::with_capacity(row.len());
        for (index, geometry) in row.iter().enumerate() {
            // A span of zero has no position to own, and one wider than the
            // model accepts is refused before it is allocated.
            if geometry.span == 0
                || positions.len().saturating_add(geometry.span) > MAX_TABLE_COLUMNS
            {
                return refusal;
            }
            positions.push((!geometry.continues_merge).then_some(index));
            positions.resize(positions.len() + geometry.span - 1, None);
        }
        grid.push(positions);
    }

    let Some(columns) = grid.first().map(Vec::len) else {
        return refusal;
    };
    if columns == 0 || grid.iter().any(|row| row.len() != columns) {
        return refusal;
    }
    if merged && declared_columns.is_some_and(|declared| declared != columns) {
        return TableGeometry::Merged;
    }
    TableGeometry::Rectangular(grid)
}
