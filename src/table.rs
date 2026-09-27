//! Tables: a grid of text cells, with a style for the whole table, style
//! overrides per cell, a stroke per edge and merged cells.
//!
//! The table is always as compact as its content: every column is as wide as
//! its widest cell and every row as tall as its tallest cell. Text never
//! wraps. The author can make an axis larger with [`TableSizing::Fixed`]; the
//! extra space is shared equally between the columns or the rows.
//!
//! The functions here are the only edits of a table. The editor and the
//! agent API both call them and send the result as
//! [`Operation::SetTable`](crate::operation::Operation::SetTable).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::document::{
    ApplyError, Element, ElementId, ElementKind, Fill, FontFace, FontLibrary, Frame, LineElement,
    RectangleElement, Rgb, Stroke, TextElement, TextSizing, TextStyle, TextStylePatch, VAlign,
};
use crate::text_layout::{self, LayoutError, TextLayout};

/// Size of each cell of the ghost grid of the table tool, in slide units.
pub const DRAW_STEP: (f32, f32) = (160., 60.);

/// How far an opaque cell fill reaches under its neighbors, in slide units.
const SEAM: f32 = 0.5;

/// A column is at least this many font sizes wide, so an empty cell stays
/// visible and clickable.
const MIN_WIDTH_EM: f32 = 2.;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TableElement {
    /// `rows[r][c]`. Every row has the same number of cells; at least one
    /// row and one column.
    pub rows: Vec<Vec<Cell>>,
    /// The text style of every cell without an override.
    pub text: TextStyle,
    /// The fill of every cell without an override.
    pub fill: Fill,
    /// The stroke of every edge set to [`Edge::Inherit`]; `null` draws no
    /// line.
    pub stroke: Option<Stroke>,
    /// Space between the cell edges and the text, in slide units.
    pub padding: f32,
    pub width: TableSizing,
    pub height: TableSizing,
    /// `(rows + 1) × columns`: `horizontal[r][c]` is the edge above row `r`
    /// in column `c`; the last line is the bottom of the table. Empty means
    /// every edge inherits.
    #[serde(skip_serializing_if = "all_inherit")]
    pub horizontal: Vec<Vec<Edge>>,
    /// `rows × (columns + 1)`: `vertical[r][c]` is the edge left of column
    /// `c` in row `r`. Empty means every edge inherits.
    #[serde(skip_serializing_if = "all_inherit")]
    pub vertical: Vec<Vec<Edge>>,
}

fn all_inherit(edges: &[Vec<Edge>]) -> bool {
    edges.iter().flatten().all(|edge| *edge == Edge::Inherit)
}

impl Default for TableElement {
    /// What JSON leaves out: one empty cell, the default style, and edge
    /// grids that [`TableElement::normalize`] fills.
    fn default() -> Self {
        Self {
            horizontal: Vec::new(),
            vertical: Vec::new(),
            ..Self::new(1, 1)
        }
    }
}

/// One cell. Only the anchor (top-left cell) of a merge is drawn; the cells
/// it covers keep their place in the grid.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cell {
    /// Plain text; `\n` separates lines.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,
    /// Replaces the table fill; `"none"` shows no fill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    /// Fields that replace those of the table text style.
    #[serde(skip_serializing_if = "TextStylePatch::is_empty")]
    pub text: TextStylePatch,
    /// Set on the anchor of merged cells.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl Cell {
    pub fn text(content: &str) -> Self {
        Self {
            content: content.into(),
            ..Self::default()
        }
    }

    /// The same style, without content or merge.
    fn style_only(&self) -> Cell {
        Cell {
            fill: self.fill.clone(),
            text: self.text.clone(),
            ..Cell::default()
        }
    }
}

/// Number of rows and columns a merged cell covers, itself included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub rows: usize,
    pub columns: usize,
}

/// How one axis of the table is sized. In JSON, `"auto"` or
/// `{"fixed": 400}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableSizing {
    /// As small as the content.
    #[default]
    Auto,
    /// The author's size, in slide units. The table is larger when its
    /// content needs more.
    Fixed(f32),
}

/// One edge between two cells, or on the outside of the table. In JSON,
/// `"inherit"`, `"none"` or `{"stroke": {..}}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// Uses the table stroke.
    #[default]
    Inherit,
    /// No line.
    None,
    Stroke(Stroke),
}

/// The edges of a cell range that a border edit changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sides {
    All,
    Outside,
    Inside,
    InsideHorizontal,
    InsideVertical,
    Top,
    Bottom,
    Left,
    Right,
}

impl Sides {
    pub const ALL: [Sides; 9] = [
        Sides::All,
        Sides::Outside,
        Sides::Inside,
        Sides::InsideHorizontal,
        Sides::InsideVertical,
        Sides::Top,
        Sides::Bottom,
        Sides::Left,
        Sides::Right,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Sides::All => "All borders",
            Sides::Outside => "Outside borders",
            Sides::Inside => "Inside borders",
            Sides::InsideHorizontal => "Inside horizontal borders",
            Sides::InsideVertical => "Inside vertical borders",
            Sides::Top => "Top border",
            Sides::Bottom => "Bottom border",
            Sides::Left => "Left border",
            Sides::Right => "Right border",
        }
    }
}

/// A rectangle of cells: rows `rows`, columns `columns`, both non-empty.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellRange {
    pub rows: Range<usize>,
    pub columns: Range<usize>,
}

impl CellRange {
    pub fn new(rows: Range<usize>, columns: Range<usize>) -> Self {
        Self { rows, columns }
    }

    pub fn cell(row: usize, column: usize) -> Self {
        Self::new(row..row + 1, column..column + 1)
    }

    /// The smallest range holding both cells.
    pub fn between(a: (usize, usize), b: (usize, usize)) -> Self {
        Self::new(
            a.0.min(b.0)..a.0.max(b.0) + 1,
            a.1.min(b.1)..a.1.max(b.1) + 1,
        )
    }

    pub fn contains(&self, row: usize, column: usize) -> bool {
        self.rows.contains(&row) && self.columns.contains(&column)
    }

    fn intersects(&self, other: &CellRange) -> bool {
        self.rows.start < other.rows.end
            && other.rows.start < self.rows.end
            && self.columns.start < other.columns.end
            && other.columns.start < self.columns.end
    }
}

/// Cells copied from a table. `styled` is false for text pasted from
/// another application: pasting it changes the content only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub cells: Vec<Vec<Cell>>,
    pub styled: bool,
}

impl Clip {
    pub fn rows(&self) -> usize {
        self.cells.len()
    }

    pub fn columns(&self) -> usize {
        self.cells.first().map_or(0, Vec::len)
    }

    /// Tab-separated values, as spreadsheets copy them. A value with a tab,
    /// a line break or a quote is quoted.
    pub fn to_tsv(&self) -> String {
        self.cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| quote(&cell.content))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Reads tab-separated values; `None` when the text has no tab and no
    /// line break, or is empty. Short rows are filled with empty cells.
    pub fn from_tsv(text: &str) -> Option<Clip> {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        if text.is_empty() || !(text.contains('\t') || text.contains('\n')) {
            return None;
        }
        let mut rows: Vec<Vec<Cell>> = vec![Vec::new()];
        let mut value = String::new();
        let mut chars = text.chars().peekable();
        let mut quoted = false;
        let mut at_start = true;
        while let Some(ch) = chars.next() {
            if quoted {
                match ch {
                    '"' if chars.peek() == Some(&'"') => {
                        chars.next();
                        value.push('"');
                    }
                    '"' => quoted = false,
                    other => value.push(other),
                }
                continue;
            }
            match ch {
                '"' if at_start => {
                    quoted = true;
                    at_start = false;
                }
                '\t' => {
                    rows.last_mut().expect("a row").push(Cell::text(&value));
                    value.clear();
                    at_start = true;
                }
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' => {
                    rows.last_mut().expect("a row").push(Cell::text(&value));
                    value.clear();
                    rows.push(Vec::new());
                    at_start = true;
                }
                other => {
                    value.push(other);
                    at_start = false;
                }
            }
        }
        rows.last_mut().expect("a row").push(Cell::text(&value));
        let columns = rows.iter().map(Vec::len).max().unwrap_or(1);
        for row in &mut rows {
            row.resize_with(columns, Cell::default);
        }
        Some(Clip {
            cells: rows,
            styled: false,
        })
    }
}

fn quote(value: &str) -> String {
    if value.contains(['\t', '\n', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

impl TableElement {
    /// An empty table with the default style: no fill, a thin gray grid,
    /// Inter 24 centered vertically in the cells.
    pub fn new(rows: usize, columns: usize) -> Self {
        let rows = rows.max(1);
        let columns = columns.max(1);
        Self {
            rows: vec![vec![Cell::default(); columns]; rows],
            text: TextStyle {
                size: 24.,
                vertical_align: VAlign::Middle,
                ..TextStyle::default()
            },
            fill: Fill::None,
            stroke: Some(Stroke {
                color: Rgb(0x111111),
                opacity: 0.4,
                width: 2.,
                ..Stroke::default()
            }),
            padding: 12.,
            width: TableSizing::Auto,
            height: TableSizing::Auto,
            horizontal: vec![vec![Edge::Inherit; columns]; rows + 1],
            vertical: vec![vec![Edge::Inherit; columns + 1]; rows],
        }
    }

    /// A table with the default style and these contents.
    pub fn from_clip(clip: &Clip) -> Self {
        let mut table = Self::new(clip.rows(), clip.columns());
        table.rows = clip.cells.clone();
        table.normalize();
        table
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    pub fn cell(&self, row: usize, column: usize) -> Option<&Cell> {
        self.rows.get(row)?.get(column)
    }

    pub fn cell_mut(&mut self, row: usize, column: usize) -> Option<&mut Cell> {
        self.rows.get_mut(row)?.get_mut(column)
    }

    /// The text style of a cell: the table style with the cell overrides.
    pub fn style_of(&self, row: usize, column: usize) -> TextStyle {
        let mut style = self.text.clone();
        if let Some(cell) = self.cell(row, column) {
            cell.text.clone().apply_to(&mut style);
        }
        style
    }

    /// The fill of a cell: its override or the table fill.
    pub fn fill_of(&self, row: usize, column: usize) -> &Fill {
        self.cell(row, column)
            .and_then(|cell| cell.fill.as_ref())
            .unwrap_or(&self.fill)
    }

    /// The span of a cell: 1 × 1 unless it anchors a merge.
    pub fn span_of(&self, row: usize, column: usize) -> Span {
        self.cell(row, column)
            .and_then(|cell| cell.span)
            .unwrap_or(Span {
                rows: 1,
                columns: 1,
            })
    }

    /// For every cell, the anchor of the merge that holds it: itself when it
    /// is not merged.
    pub fn owners(&self) -> Vec<Vec<(usize, usize)>> {
        let mut owners: Vec<Vec<(usize, usize)>> = (0..self.row_count())
            .map(|row| (0..self.column_count()).map(|col| (row, col)).collect())
            .collect();
        for (row, cells) in self.rows.iter().enumerate() {
            for (column, cell) in cells.iter().enumerate() {
                if let Some(span) = cell.span {
                    let columns = column..(column + span.columns).min(self.column_count());
                    for line in owners.iter_mut().skip(row).take(span.rows) {
                        line[columns.clone()].fill((row, column));
                    }
                }
            }
        }
        owners
    }

    /// The anchor of the merge holding the cell.
    pub fn anchor_of(&self, row: usize, column: usize) -> (usize, usize) {
        self.owners()
            .get(row)
            .and_then(|cells| cells.get(column))
            .copied()
            .unwrap_or((row, column))
    }

    /// The anchors, in reading order: the cells that are drawn.
    pub fn anchors(&self) -> Vec<(usize, usize)> {
        let owners = self.owners();
        let mut anchors = Vec::new();
        for (row, cells) in owners.iter().enumerate() {
            for (column, owner) in cells.iter().enumerate() {
                if *owner == (row, column) {
                    anchors.push((row, column));
                }
            }
        }
        anchors
    }

    /// The range grown until no merge crosses its edges, clamped to the
    /// table.
    pub fn expand(&self, range: &CellRange) -> CellRange {
        let rows = self.row_count();
        let columns = self.column_count();
        let mut range = CellRange::new(
            range.rows.start.min(rows - 1)..range.rows.end.clamp(1, rows),
            range.columns.start.min(columns - 1)..range.columns.end.clamp(1, columns),
        );
        if range.rows.is_empty() {
            range.rows = range.rows.start..range.rows.start + 1;
        }
        if range.columns.is_empty() {
            range.columns = range.columns.start..range.columns.start + 1;
        }
        loop {
            let before = range.clone();
            for (row, column) in self.anchors() {
                let span = self.span_of(row, column);
                let merge = CellRange::new(row..row + span.rows, column..column + span.columns);
                if merge.intersects(&range) {
                    range.rows =
                        range.rows.start.min(merge.rows.start)..range.rows.end.max(merge.rows.end);
                    range.columns = range.columns.start.min(merge.columns.start)
                        ..range.columns.end.max(merge.columns.end);
                }
            }
            if range == before {
                return range;
            }
        }
    }

    /// The whole table as a range.
    pub fn all(&self) -> CellRange {
        CellRange::new(0..self.row_count(), 0..self.column_count())
    }

    /// Every font face the table uses.
    pub fn faces(&self) -> Vec<FontFace> {
        let mut faces = vec![self.text.font.clone()];
        for cell in self.rows.iter().flatten() {
            if let Some(font) = &cell.text.font
                && !faces.contains(font)
            {
                faces.push(font.clone());
            }
        }
        faces
    }

    /// Every fill the table uses, overrides included.
    pub fn fills(&self) -> impl Iterator<Item = &Fill> {
        std::iter::once(&self.fill).chain(
            self.rows
                .iter()
                .flatten()
                .filter_map(|cell| cell.fill.as_ref()),
        )
    }

    /// Every stroke the table uses.
    fn strokes(&self) -> impl Iterator<Item = &Stroke> {
        self.stroke.iter().chain(
            self.horizontal
                .iter()
                .chain(&self.vertical)
                .flatten()
                .filter_map(|edge| match edge {
                    Edge::Stroke(stroke) => Some(stroke),
                    _ => None,
                }),
        )
    }

    /// Fills empty edge grids with [`Edge::Inherit`]: JSON may leave them
    /// out.
    pub fn normalize(&mut self) {
        let (rows, columns) = (self.row_count(), self.column_count());
        if self.horizontal.is_empty() {
            self.horizontal = vec![vec![Edge::Inherit; columns]; rows + 1];
        }
        if self.vertical.is_empty() {
            self.vertical = vec![vec![Edge::Inherit; columns + 1]; rows];
        }
    }

    pub fn validate(&self) -> Result<(), ApplyError> {
        let rows = self.row_count();
        let columns = self.column_count();
        if rows == 0 || columns == 0 || self.rows.iter().any(|row| row.len() != columns) {
            return Err(ApplyError::InvalidTable(
                "rows must all have the same cells",
            ));
        }
        if self.horizontal.len() != rows + 1
            || self.horizontal.iter().any(|line| line.len() != columns)
            || self.vertical.len() != rows
            || self.vertical.iter().any(|line| line.len() != columns + 1)
        {
            return Err(ApplyError::InvalidTable(
                "edge grids do not match the cells",
            ));
        }
        if !(self.padding.is_finite() && self.padding >= 0.) {
            return Err(ApplyError::InvalidStyle);
        }
        for sizing in [self.width, self.height] {
            if let TableSizing::Fixed(size) = sizing
                && !(size.is_finite() && size >= 0.)
            {
                return Err(ApplyError::InvalidFrame);
            }
        }
        let mut covered = vec![vec![false; columns]; rows];
        for (row, cells) in self.rows.iter().enumerate() {
            for (column, cell) in cells.iter().enumerate() {
                let span = cell.span.unwrap_or(Span {
                    rows: 1,
                    columns: 1,
                });
                if span.rows == 0
                    || span.columns == 0
                    || row + span.rows > rows
                    || column + span.columns > columns
                {
                    return Err(ApplyError::InvalidCell { row, column });
                }
                if covered[row][column] {
                    if cell.span.is_some() {
                        return Err(ApplyError::InvalidTable("merged cells overlap"));
                    }
                    continue;
                }
                for line in covered.iter_mut().skip(row).take(span.rows) {
                    for slot in &mut line[column..column + span.columns] {
                        if *slot {
                            return Err(ApplyError::InvalidTable("merged cells overlap"));
                        }
                        *slot = true;
                    }
                }
                cell.text.validate()?;
            }
        }
        TextStylePatch::from(self.text.clone()).validate()?;
        for fill in self.fills() {
            fill.validate()?;
        }
        for stroke in self.strokes() {
            stroke.validate()?;
        }
        Ok(())
    }

    fn check(&self, row: usize, column: usize) -> Result<(), ApplyError> {
        if row < self.row_count() && column < self.column_count() {
            Ok(())
        } else {
            Err(ApplyError::InvalidCell { row, column })
        }
    }

    fn check_range(&self, range: &CellRange) -> Result<(), ApplyError> {
        if range.rows.is_empty() || range.columns.is_empty() {
            return Err(ApplyError::InvalidTable("the cell range is empty"));
        }
        self.check(range.rows.end - 1, range.columns.end - 1)
    }

    /// The same table with rows and columns swapped. Column edits are row
    /// edits of the transposed table.
    fn transposed(&self) -> TableElement {
        let (rows, columns) = (self.row_count(), self.column_count());
        let cells = (0..columns)
            .map(|c| {
                (0..rows)
                    .map(|r| {
                        let mut cell = self.rows[r][c].clone();
                        cell.span = cell.span.map(|span| Span {
                            rows: span.columns,
                            columns: span.rows,
                        });
                        cell
                    })
                    .collect()
            })
            .collect();
        let flip = |grid: &Vec<Vec<Edge>>| -> Vec<Vec<Edge>> {
            let outer = grid.first().map_or(0, Vec::len);
            (0..outer)
                .map(|i| grid.iter().map(|line| line[i]).collect())
                .collect()
        };
        TableElement {
            rows: cells,
            text: self.text.clone(),
            fill: self.fill.clone(),
            stroke: self.stroke,
            padding: self.padding,
            width: self.height,
            height: self.width,
            horizontal: flip(&self.vertical),
            vertical: flip(&self.horizontal),
        }
    }

    fn transpose_edit<T>(
        &mut self,
        edit: impl FnOnce(&mut TableElement) -> Result<T, ApplyError>,
    ) -> Result<T, ApplyError> {
        let mut flipped = self.transposed();
        let result = edit(&mut flipped)?;
        *self = flipped.transposed();
        Ok(result)
    }

    /// Inserts `count` rows before row `at` (`at` may be the row count). The
    /// new rows copy the style and the side edges of row `like`, not its
    /// content. A merge the new rows cut through grows over them.
    pub fn insert_rows(&mut self, at: usize, count: usize, like: usize) -> Result<(), ApplyError> {
        let rows = self.row_count();
        if at > rows || count == 0 {
            return Err(ApplyError::InvalidCell { row: at, column: 0 });
        }
        self.check(like, 0)?;
        for row in 0..rows {
            for column in 0..self.column_count() {
                let span = self.span_of(row, column);
                if row < at && at < row + span.rows {
                    self.rows[row][column].span = Some(Span {
                        rows: span.rows + count,
                        ..span
                    });
                }
            }
        }
        let template: Vec<Cell> = self.rows[like].iter().map(Cell::style_only).collect();
        let sides = self.vertical[like].clone();
        for _ in 0..count {
            self.rows.insert(at, template.clone());
            self.vertical.insert(at, sides.clone());
        }
        // The outer lines stay outside: new lines go inside the table.
        let (index, source) = match at {
            0 => (1, 1.min(rows)),
            at if at == rows => (rows, if rows > 1 { rows - 1 } else { rows }),
            at => (at, at),
        };
        let line = self.horizontal[source].clone();
        for _ in 0..count {
            self.horizontal.insert(index, line.clone());
        }
        Ok(())
    }

    /// Removes rows `range`. A merge loses the removed rows; when its anchor
    /// goes, the first remaining row takes the anchor, its content and its
    /// style.
    pub fn remove_rows(&mut self, range: Range<usize>) -> Result<(), ApplyError> {
        let rows = self.row_count();
        if range.is_empty() || range.end > rows {
            return Err(ApplyError::InvalidCell {
                row: range.end,
                column: 0,
            });
        }
        if range.len() == rows {
            return Err(ApplyError::InvalidTable("a table keeps at least one row"));
        }
        for row in 0..rows {
            for column in 0..self.column_count() {
                let Some(span) = self.rows[row][column].span else {
                    continue;
                };
                let end = row + span.rows;
                let removed = end.min(range.end).saturating_sub(row.max(range.start));
                if removed == 0 {
                    continue;
                }
                let left = span.rows - removed;
                if left == 0 {
                    self.rows[row][column].span = None;
                    continue;
                }
                let span = Some(Span { rows: left, ..span });
                if range.contains(&row) {
                    let moved = std::mem::take(&mut self.rows[row][column]);
                    self.rows[range.end][column] = Cell { span, ..moved };
                } else {
                    self.rows[row][column].span = span;
                }
            }
        }
        self.rows.drain(range.clone());
        self.vertical.drain(range.clone());
        if range.end == rows {
            self.horizontal.drain(range.start..range.end);
        } else {
            self.horizontal.drain(range.start + 1..range.end + 1);
        }
        Ok(())
    }

    /// Moves rows `range` before row `to` of the table as it is now (`to`
    /// may be the row count). Refused when it would cut a merge.
    pub fn move_rows(&mut self, range: Range<usize>, to: usize) -> Result<(), ApplyError> {
        let rows = self.row_count();
        if range.is_empty() || range.end > rows || to > rows {
            return Err(ApplyError::InvalidCell { row: to, column: 0 });
        }
        if range.contains(&to) || to == range.end {
            return Ok(());
        }
        let cuts = |line: usize| {
            line > 0
                && line < rows
                && self.anchors().into_iter().any(|(row, column)| {
                    let span = self.span_of(row, column);
                    row < line && line < row + span.rows
                })
        };
        if cuts(range.start) || cuts(range.end) || cuts(to) {
            return Err(ApplyError::CutsMerge);
        }
        let mut order: Vec<usize> = (0..rows).filter(|row| !range.contains(row)).collect();
        let at = order
            .iter()
            .position(|row| *row >= to)
            .unwrap_or(order.len());
        order.splice(at..at, range.clone());
        let tops: Vec<Vec<Edge>> = self.horizontal[..rows].to_vec();
        let mut new_tops: Vec<Vec<Edge>> = order.iter().map(|row| tops[*row].clone()).collect();
        // The outer top line stays on top.
        if order[0] != 0 {
            let old_first = order.iter().position(|row| *row == 0).expect("row 0");
            new_tops[old_first] = tops[order[0]].clone();
            new_tops[0] = tops[0].clone();
        }
        let old_rows = std::mem::take(&mut self.rows);
        let old_sides = std::mem::take(&mut self.vertical);
        self.rows = order.iter().map(|row| old_rows[*row].clone()).collect();
        self.vertical = order.iter().map(|row| old_sides[*row].clone()).collect();
        new_tops.push(self.horizontal[rows].clone());
        self.horizontal = new_tops;
        Ok(())
    }

    pub fn insert_columns(
        &mut self,
        at: usize,
        count: usize,
        like: usize,
    ) -> Result<(), ApplyError> {
        self.transpose_edit(|table| table.insert_rows(at, count, like))
            .map_err(transpose_error)
    }

    pub fn remove_columns(&mut self, range: Range<usize>) -> Result<(), ApplyError> {
        let result = self.transpose_edit(|table| table.remove_rows(range));
        match result {
            Err(ApplyError::InvalidTable(_)) => Err(ApplyError::InvalidTable(
                "a table keeps at least one column",
            )),
            other => other.map_err(transpose_error),
        }
    }

    pub fn move_columns(&mut self, range: Range<usize>, to: usize) -> Result<(), ApplyError> {
        self.transpose_edit(|table| table.move_rows(range, to))
            .map_err(transpose_error)
    }

    /// Merges the cells of the range, grown to hold whole merges. The anchor
    /// keeps its style; the texts that are not empty become its lines, in
    /// reading order.
    pub fn merge(&mut self, range: &CellRange) -> Result<(), ApplyError> {
        self.check_range(range)?;
        let range = self.expand(range);
        let texts: Vec<String> = self
            .anchors()
            .into_iter()
            .filter(|(row, column)| range.contains(*row, *column))
            .map(|(row, column)| self.rows[row][column].content.clone())
            .filter(|content| !content.is_empty())
            .collect();
        for row in range.rows.clone() {
            for column in range.columns.clone() {
                let cell = &mut self.rows[row][column];
                cell.span = None;
                cell.content.clear();
            }
        }
        let anchor = &mut self.rows[range.rows.start][range.columns.start];
        anchor.content = texts.join("\n");
        if range.rows.len() > 1 || range.columns.len() > 1 {
            anchor.span = Some(Span {
                rows: range.rows.len(),
                columns: range.columns.len(),
            });
        }
        Ok(())
    }

    /// Splits the merge holding the cell back into single cells. The text
    /// stays in the anchor.
    pub fn split(&mut self, row: usize, column: usize) -> Result<(), ApplyError> {
        self.check(row, column)?;
        let (row, column) = self.anchor_of(row, column);
        self.rows[row][column].span = None;
        Ok(())
    }

    /// Whether any cell of the range is part of a merge.
    pub fn has_merge(&self, range: &CellRange) -> bool {
        let owners = self.owners();
        range.rows.clone().any(|row| {
            range.columns.clone().any(|column| {
                owners.get(row).and_then(|cells| cells.get(column)) != Some(&(row, column))
                    || self.span_of(row, column)
                        != Span {
                            rows: 1,
                            columns: 1,
                        }
            })
        })
    }

    /// Sets the fill and text overrides of the anchors in the range.
    /// `fill: Some(None)` removes the fill override; `reset` removes the
    /// text overrides before `text` applies.
    pub fn set_cell_styles(
        &mut self,
        range: &CellRange,
        fill: Option<Option<Fill>>,
        text: &TextStylePatch,
        reset: bool,
    ) -> Result<(), ApplyError> {
        self.check_range(range)?;
        text.validate()?;
        if let Some(Some(fill)) = &fill {
            fill.validate()?;
        }
        let range = self.expand(range);
        for (row, column) in self.anchors() {
            if !range.contains(row, column) {
                continue;
            }
            let cell = &mut self.rows[row][column];
            if let Some(fill) = &fill {
                cell.fill = fill.clone();
            }
            if reset {
                cell.text = TextStylePatch::default();
            }
            cell.text.merge(text.clone());
        }
        Ok(())
    }

    /// Sets the edges of the range that `sides` names.
    pub fn set_borders(
        &mut self,
        range: &CellRange,
        sides: Sides,
        edge: Edge,
    ) -> Result<(), ApplyError> {
        self.check_range(range)?;
        if let Edge::Stroke(stroke) = &edge {
            stroke.validate()?;
        }
        for (horizontal, line, index) in self.edges_of(range, sides) {
            if horizontal {
                self.horizontal[line][index] = edge;
            } else {
                self.vertical[index][line] = edge;
            }
        }
        Ok(())
    }

    /// The strokes drawn on the edges of the range that `sides` names.
    pub fn border_strokes(&self, range: &CellRange, sides: Sides) -> Vec<Option<Stroke>> {
        let owners = self.owners();
        self.edges_of(range, sides)
            .into_iter()
            .map(|(horizontal, line, index)| self.edge_stroke(&owners, horizontal, line, index))
            .collect()
    }

    /// The edges of the range, grown to hold whole merges, that `sides`
    /// names: whether the edge is horizontal, its line and its index
    /// along the line.
    fn edges_of(&self, range: &CellRange, sides: Sides) -> Vec<(bool, usize, usize)> {
        let range = self.expand(range);
        let (rows, columns) = (range.rows.clone(), range.columns.clone());
        let (top, bottom, left, right, inside_h, inside_v) = match sides {
            Sides::All => (true, true, true, true, true, true),
            Sides::Outside => (true, true, true, true, false, false),
            Sides::Inside => (false, false, false, false, true, true),
            Sides::InsideHorizontal => (false, false, false, false, true, false),
            Sides::InsideVertical => (false, false, false, false, false, true),
            Sides::Top => (true, false, false, false, false, false),
            Sides::Bottom => (false, true, false, false, false, false),
            Sides::Left => (false, false, true, false, false, false),
            Sides::Right => (false, false, false, true, false, false),
        };
        let mut edges = Vec::new();
        for line in rows.start..=rows.end {
            let wanted = (line == rows.start && top)
                || (line == rows.end && bottom)
                || (line != rows.start && line != rows.end && inside_h);
            if wanted {
                edges.extend(columns.clone().map(|column| (true, line, column)));
            }
        }
        for line in columns.start..=columns.end {
            let wanted = (line == columns.start && left)
                || (line == columns.end && right)
                || (line != columns.start && line != columns.end && inside_v);
            if wanted {
                edges.extend(rows.clone().map(|row| (false, line, row)));
            }
        }
        edges
    }

    /// Empties the text of the cells in the range; styles stay.
    pub fn clear(&mut self, range: &CellRange) -> Result<(), ApplyError> {
        self.check_range(range)?;
        let range = self.expand(range);
        for row in range.rows.clone() {
            for column in range.columns.clone() {
                self.rows[row][column].content.clear();
            }
        }
        Ok(())
    }

    /// The cells of the range, grown to hold whole merges.
    pub fn copy(&self, range: &CellRange) -> Result<Clip, ApplyError> {
        self.check_range(range)?;
        let range = self.expand(range);
        let cells = range
            .rows
            .clone()
            .map(|row| self.rows[row][range.columns.clone()].to_vec())
            .collect();
        Ok(Clip {
            cells,
            styled: true,
        })
    }

    /// Writes the clip with its top-left cell at (`row`, `column`), which may
    /// be past the table. The table grows when the clip does not fit; merges the pasted area touches are
    /// split first. A styled clip brings its styles and merges.
    pub fn paste(&mut self, row: usize, column: usize, clip: &Clip) -> Result<(), ApplyError> {
        if clip.rows() == 0 || clip.columns() == 0 {
            return Ok(());
        }
        let rows_needed = row + clip.rows();
        if rows_needed > self.row_count() {
            let last = self.row_count() - 1;
            self.insert_rows(self.row_count(), rows_needed - self.row_count(), last)?;
        }
        let columns_needed = column + clip.columns();
        if columns_needed > self.column_count() {
            let last = self.column_count() - 1;
            self.insert_columns(
                self.column_count(),
                columns_needed - self.column_count(),
                last,
            )?;
        }
        let area = CellRange::new(row..rows_needed, column..columns_needed);
        for (r, c) in self.anchors() {
            let span = self.span_of(r, c);
            if CellRange::new(r..r + span.rows, c..c + span.columns).intersects(&area) {
                self.rows[r][c].span = None;
            }
        }
        for (dr, cells) in clip.cells.iter().enumerate() {
            for (dc, source) in cells.iter().enumerate() {
                let target = &mut self.rows[row + dr][column + dc];
                if clip.styled {
                    *target = source.clone();
                } else {
                    target.content = source.content.clone();
                }
            }
        }
        Ok(())
    }

    /// The stroke drawn on an edge, `None` for no line. `horizontal` picks
    /// the grid; interior edges of a merge are never drawn.
    pub fn edge_stroke(
        &self,
        owners: &[Vec<(usize, usize)>],
        horizontal: bool,
        line: usize,
        index: usize,
    ) -> Option<Stroke> {
        let (edge, interior) = if horizontal {
            let interior = line > 0
                && line < self.row_count()
                && owners[line - 1][index] == owners[line][index];
            (self.horizontal[line][index], interior)
        } else {
            let interior = line > 0
                && line < self.column_count()
                && owners[index][line - 1] == owners[index][line];
            (self.vertical[index][line], interior)
        };
        if interior {
            return None;
        }
        match edge {
            Edge::Inherit => self.stroke,
            Edge::None => None,
            Edge::Stroke(stroke) => Some(stroke),
        }
    }

    /// The name of the table in the hierarchy: its first text, if any.
    pub fn first_text(&self) -> Option<&str> {
        self.rows
            .iter()
            .flatten()
            .map(|cell| cell.content.trim())
            .find(|content| !content.is_empty())
            .map(|content| content.lines().next().unwrap_or(content))
    }
}

/// Errors of a transposed edit name columns, not rows.
fn transpose_error(error: ApplyError) -> ApplyError {
    match error {
        ApplyError::InvalidCell { row, column } => ApplyError::InvalidCell {
            row: column,
            column: row,
        },
        other => other,
    }
}

/// Where the cells of a table are, in slide units from the top-left corner
/// of the unrotated table frame.
#[derive(Clone, Debug, PartialEq)]
pub struct TableLayout {
    /// Width of every column.
    pub columns: Vec<f32>,
    /// Height of every row.
    pub rows: Vec<f32>,
    /// The drawn cells, anchors only, in reading order.
    pub cells: Vec<CellLayout>,
    /// Characters the fonts have no glyph for.
    pub missing_glyphs: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellLayout {
    pub row: usize,
    pub column: usize,
    /// The cell, edges included.
    pub rect: Frame,
    /// The cell without its padding: the box of the text.
    pub inner: Frame,
    /// The text, laid out as a fixed box of the size of `inner`.
    pub text: TextElement,
    pub layout: TextLayout,
}

impl TableLayout {
    pub fn width(&self) -> f32 {
        self.columns.iter().sum()
    }

    pub fn height(&self) -> f32 {
        self.rows.iter().sum()
    }

    /// The x of the left edge of each column, plus the right edge.
    pub fn column_edges(&self) -> Vec<f32> {
        edges(&self.columns)
    }

    /// The y of the top edge of each row, plus the bottom edge.
    pub fn row_edges(&self) -> Vec<f32> {
        edges(&self.rows)
    }

    /// The row and column under a point, clamped to the table.
    pub fn grid_at(&self, x: f32, y: f32) -> (usize, usize) {
        let find = |edges: Vec<f32>, at: f32| {
            let count = edges.len() - 1;
            (0..count)
                .find(|ix| at < edges[ix + 1])
                .unwrap_or(count.saturating_sub(1))
        };
        (find(self.row_edges(), y), find(self.column_edges(), x))
    }

    /// The layout of the drawn cell holding (`row`, `column`).
    pub fn cell_holding(&self, row: usize, column: usize) -> Option<&CellLayout> {
        self.cells.iter().find(|cell| {
            let rows = cell.rect.y..cell.rect.y + cell.rect.height;
            let columns = cell.rect.x..cell.rect.x + cell.rect.width;
            let (x, y) = self.center_of(row, column);
            rows.contains(&y) && columns.contains(&x)
        })
    }

    /// The layout of an anchor.
    pub fn cell(&self, row: usize, column: usize) -> Option<&CellLayout> {
        self.cells
            .iter()
            .find(|cell| cell.row == row && cell.column == column)
    }

    fn center_of(&self, row: usize, column: usize) -> (f32, f32) {
        let columns = self.column_edges();
        let rows = self.row_edges();
        (
            (columns[column] + columns[column + 1]) / 2.,
            (rows[row] + rows[row + 1]) / 2.,
        )
    }

    /// The box of a range of cells.
    pub fn range_rect(&self, range: &CellRange) -> Frame {
        let columns = self.column_edges();
        let rows = self.row_edges();
        Frame {
            x: columns[range.columns.start],
            y: rows[range.rows.start],
            width: columns[range.columns.end] - columns[range.columns.start],
            height: rows[range.rows.end] - rows[range.rows.start],
            rotation: 0.,
        }
    }
}

fn edges(sizes: &[f32]) -> Vec<f32> {
    let mut edges = Vec::with_capacity(sizes.len() + 1);
    let mut at = 0.;
    edges.push(at);
    for size in sizes {
        at += size;
        edges.push(at);
    }
    edges
}

/// Lays out the table: the size of every column and row, and the text of
/// every drawn cell. Fails when a face the table uses is not in `fonts`.
pub fn layout(table: &TableElement, fonts: &FontLibrary) -> Result<TableLayout, LayoutError> {
    let _span = crate::perf::span("table_layout");
    let rows = table.row_count();
    let columns = table.column_count();
    let padding = table.padding;
    let anchors = table.anchors();
    let mut sizes = Vec::with_capacity(anchors.len());
    for &(row, column) in &anchors {
        let style = table.style_of(row, column);
        let font = fonts.get(&style.font).ok_or(LayoutError)?;
        let text = TextElement {
            content: table.rows[row][column].content.clone(),
            style,
            sizing: TextSizing::AutoWidth,
        };
        let (width, height) = text_layout::measure(&text, 0., font)?;
        let width = width.max(text.style.size * MIN_WIDTH_EM) + 2. * padding;
        sizes.push((row, column, width, height + 2. * padding));
    }
    let mut column_widths = vec![0f32; columns];
    let mut row_heights = vec![0f32; rows];
    for &(row, column, width, height) in &sizes {
        let span = table.span_of(row, column);
        if span.columns == 1 {
            column_widths[column] = column_widths[column].max(width);
        }
        if span.rows == 1 {
            row_heights[row] = row_heights[row].max(height);
        }
    }
    // Merged cells spread what they need past their columns (rows) equally,
    // the smallest merges first.
    let mut merged: Vec<_> = sizes
        .iter()
        .map(|&(row, column, width, height)| {
            (row, column, table.span_of(row, column), width, height)
        })
        .collect();
    merged.sort_by_key(|(_, _, span, _, _)| span.columns);
    for &(_, column, span, width, _) in &merged {
        if span.columns > 1 {
            spread(&mut column_widths[column..column + span.columns], width);
        }
    }
    merged.sort_by_key(|(_, _, span, _, _)| span.rows);
    for &(row, _, span, _, height) in &merged {
        if span.rows > 1 {
            spread(&mut row_heights[row..row + span.rows], height);
        }
    }
    if let TableSizing::Fixed(width) = table.width {
        spread(&mut column_widths, width);
    }
    if let TableSizing::Fixed(height) = table.height {
        spread(&mut row_heights, height);
    }
    let column_edges = edges(&column_widths);
    let row_edges = edges(&row_heights);
    let mut cells = Vec::with_capacity(anchors.len());
    let mut missing_glyphs = 0;
    for (row, column) in anchors {
        let span = table.span_of(row, column);
        let rect = Frame {
            x: column_edges[column],
            y: row_edges[row],
            width: column_edges[column + span.columns] - column_edges[column],
            height: row_edges[row + span.rows] - row_edges[row],
            rotation: 0.,
        };
        let inner = Frame {
            x: rect.x + padding,
            y: rect.y + padding,
            width: (rect.width - 2. * padding).max(0.),
            height: (rect.height - 2. * padding).max(0.),
            rotation: 0.,
        };
        let style = table.style_of(row, column);
        let font = fonts.get(&style.font).ok_or(LayoutError)?;
        let text = TextElement {
            content: table.rows[row][column].content.clone(),
            style,
            sizing: TextSizing::Fixed,
        };
        let layout = text_layout::layout(&text, &inner, font)?;
        missing_glyphs += layout.missing_glyphs;
        cells.push(CellLayout {
            row,
            column,
            rect,
            inner,
            text,
            layout,
        });
    }
    Ok(TableLayout {
        columns: column_widths,
        rows: row_heights,
        cells,
        missing_glyphs,
    })
}

/// The table with the sizing a frame change from `old` to `new` gives: a
/// changed width or height becomes the fixed size of that axis, or goes
/// back to automatic when it is not larger than the content.
pub fn resized(
    table: &TableElement,
    fonts: &FontLibrary,
    old: &Frame,
    new: &Frame,
) -> Result<TableElement, LayoutError> {
    let mut compact = table.clone();
    compact.width = TableSizing::Auto;
    compact.height = TableSizing::Auto;
    let natural = layout(&compact, fonts)?;
    let axis = |size: f32, old_size: f32, natural: f32, current: TableSizing| {
        if size == old_size {
            current
        } else if size > natural + 0.01 {
            TableSizing::Fixed(size)
        } else {
            TableSizing::Auto
        }
    };
    let mut after = table.clone();
    after.width = axis(new.width, old.width, natural.width(), table.width);
    after.height = axis(new.height, old.height, natural.height(), table.height);
    Ok(after)
}

/// Grows `sizes` equally until they add up to at least `total`.
fn spread(sizes: &mut [f32], total: f32) {
    let sum: f32 = sizes.iter().sum();
    if total > sum && !sizes.is_empty() {
        let extra = (total - sum) / sizes.len() as f32;
        for size in sizes {
            *size += extra;
        }
    }
}

/// A piece of a table drawn by the painters of the other elements.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// The fill of a cell: a rectangle.
    Fill(Element),
    /// A border: a line.
    Border(Element),
    /// The text of a cell: a fixed text box, with its layout.
    Text {
        element: Element,
        layout: TextLayout,
        row: usize,
        column: usize,
    },
}

impl Part {
    pub fn element(&self) -> &Element {
        match self {
            Part::Fill(element) | Part::Border(element) => element,
            Part::Text { element, .. } => element,
        }
    }
}

/// The frame on the slide of a box given in the unrotated table frame.
pub fn place(table: &Frame, rect: &Frame) -> Frame {
    let (cx, cy) = table.to_slide(rect.x + rect.width / 2., rect.y + rect.height / 2.);
    Frame {
        x: cx - rect.width / 2.,
        y: cy - rect.height / 2.,
        width: rect.width,
        height: rect.height,
        rotation: table.rotation,
    }
}

/// The pieces that draw the table `id` placed at `frame`, in paint order:
/// cell fills, borders, then texts. Each piece has the id of the table and
/// a frame on the slide.
pub fn parts(
    id: ElementId,
    frame: &Frame,
    table: &TableElement,
    layout: &TableLayout,
) -> Vec<Part> {
    let mut parts = Vec::new();
    let (width, height) = (layout.width(), layout.height());
    for cell in &layout.cells {
        let fill = table.fill_of(cell.row, cell.column);
        if fill.is_none() {
            continue;
        }
        // An opaque fill reaches under its right and bottom neighbors, so
        // no seam of the background shows between two cells.
        let mut rect = cell.rect;
        if fill.opacity() == 1. {
            if rect.x + rect.width < width - 0.01 {
                rect.width += SEAM;
            }
            if rect.y + rect.height < height - 0.01 {
                rect.height += SEAM;
            }
        }
        let kind = ElementKind::Rectangle(RectangleElement {
            fill: fill.clone(),
            stroke: None,
            corner_radius: 0.,
        });
        parts.push(Part::Fill(Element::new(id, place(frame, &rect), kind)));
    }
    let owners = table.owners();
    let columns = layout.column_edges();
    let rows = layout.row_edges();
    for (line, y) in rows.iter().enumerate() {
        runs(table.column_count(), |index| {
            table.edge_stroke(&owners, true, line, index)
        })
        .into_iter()
        .for_each(|(range, stroke)| {
            let half = stroke.width / 2.;
            let start = frame.to_slide(columns[range.start] - half, *y);
            let end = frame.to_slide(columns[range.end] + half, *y);
            parts.push(border(id, start, end, frame.rotation, stroke));
        });
    }
    for (line, x) in columns.iter().enumerate() {
        runs(table.row_count(), |index| {
            table.edge_stroke(&owners, false, line, index)
        })
        .into_iter()
        .for_each(|(range, stroke)| {
            let start = frame.to_slide(*x, rows[range.start]);
            let end = frame.to_slide(*x, rows[range.end]);
            parts.push(border(id, start, end, frame.rotation + 90., stroke));
        });
    }
    for cell in &layout.cells {
        if cell.text.content.is_empty() {
            continue;
        }
        let kind = ElementKind::Text(cell.text.clone());
        parts.push(Part::Text {
            element: Element::new(id, place(frame, &cell.inner), kind),
            layout: cell.layout.clone(),
            row: cell.row,
            column: cell.column,
        });
    }
    parts
}

/// Runs of consecutive edges drawn with the same stroke.
fn runs(count: usize, stroke: impl Fn(usize) -> Option<Stroke>) -> Vec<(Range<usize>, Stroke)> {
    let mut runs: Vec<(Range<usize>, Stroke)> = Vec::new();
    for index in 0..count {
        let Some(stroke) = stroke(index) else {
            continue;
        };
        match runs.last_mut() {
            Some((range, last)) if range.end == index && *last == stroke => range.end = index + 1,
            _ => runs.push((index..index + 1, stroke)),
        }
    }
    runs
}

fn border(
    id: ElementId,
    start: (f32, f32),
    end: (f32, f32),
    rotation: f32,
    stroke: Stroke,
) -> Part {
    let kind = ElementKind::Line(LineElement {
        stroke,
        ..LineElement::default()
    });
    Part::Border(Element::new(
        id,
        Frame::from_line(start, end, rotation),
        kind,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::with_inter;

    fn table(cells: &[&[&str]]) -> TableElement {
        let clip = Clip {
            cells: cells
                .iter()
                .map(|row| row.iter().map(|text| Cell::text(text)).collect())
                .collect(),
            styled: false,
        };
        TableElement::from_clip(&clip)
    }

    fn sizes(table: &TableElement) -> (Vec<f32>, Vec<f32>) {
        let presentation = with_inter();
        let layout = layout(table, &presentation.fonts).expect("layout");
        (layout.columns, layout.rows)
    }

    #[test]
    fn a_column_is_as_wide_as_its_widest_cell() {
        let narrow = table(&[&["a"]]);
        let wide = table(&[&["a much longer text"]]);
        let both = table(&[&["a", "b"], &["a much longer text", "b"]]);
        let (narrow, _) = sizes(&narrow);
        let (wide, _) = sizes(&wide);
        let (both, _) = sizes(&both);
        assert_eq!(both[0], wide[0]);
        assert_eq!(both[1], narrow[0]);
    }

    #[test]
    fn an_empty_column_keeps_two_font_sizes_of_width() {
        let empty = TableElement::new(1, 1);
        let (columns, rows) = sizes(&empty);
        assert_eq!(columns[0], 24. * 2. + 24.);
        assert!(rows[0] > 24.);
    }

    #[test]
    fn a_new_line_makes_only_its_row_taller() {
        let one = table(&[&["a", "b"], &["c", "d"]]);
        let two = table(&[&["a", "b"], &["c\nmore", "d"]]);
        let (_, one) = sizes(&one);
        let (_, two) = sizes(&two);
        assert_eq!(one[0], two[0]);
        assert!(two[1] > one[1]);
    }

    #[test]
    fn a_fixed_width_spreads_the_extra_space_equally_and_never_shrinks() {
        let mut fixed = table(&[&["a", "a much longer text"]]);
        let (compact, _) = sizes(&fixed);
        fixed.width = TableSizing::Fixed(compact.iter().sum::<f32>() + 100.);
        let (wider, _) = sizes(&fixed);
        assert_eq!(wider[0], compact[0] + 50.);
        assert_eq!(wider[1], compact[1] + 50.);
        fixed.width = TableSizing::Fixed(10.);
        assert_eq!(sizes(&fixed).0, compact);
    }

    #[test]
    fn a_merged_cell_spreads_what_it_needs_over_its_columns() {
        let mut merged = table(&[&["a much longer text", ""], &["a", "b"]]);
        let (single, _) = sizes(&table(&[&["a", "b"]]));
        let (long, _) = sizes(&table(&[&["a much longer text"]]));
        merged.merge(&CellRange::new(0..1, 0..2)).expect("merge");
        let (columns, _) = sizes(&merged);
        let extra = (long[0] - single[0] - single[1]) / 2.;
        assert!((columns[0] - (single[0] + extra)).abs() < 0.01);
        assert!((columns[1] - (single[1] + extra)).abs() < 0.01);
    }

    #[test]
    fn merging_joins_the_texts_and_splitting_keeps_them_in_the_anchor() {
        let mut merged = table(&[&["a", "b"], &["", "d"]]);
        merged.merge(&merged.all()).expect("merge");
        assert_eq!(merged.rows[0][0].content, "a\nb\nd");
        assert_eq!(
            merged.rows[0][0].span,
            Some(Span {
                rows: 2,
                columns: 2
            })
        );
        assert_eq!(merged.anchors(), vec![(0, 0)]);
        merged.split(1, 1).expect("split");
        assert_eq!(merged.anchors().len(), 4);
        assert_eq!(merged.rows[0][0].content, "a\nb\nd");
        assert_eq!(merged.rows[1][1].content, "");
    }

    #[test]
    fn a_range_grows_to_hold_whole_merges() {
        let mut merged = TableElement::new(3, 3);
        merged.merge(&CellRange::new(1..3, 1..3)).expect("merge");
        assert_eq!(
            merged.expand(&CellRange::cell(2, 2)),
            CellRange::new(1..3, 1..3)
        );
        assert_eq!(
            merged.expand(&CellRange::new(0..2, 0..2)),
            CellRange::new(0..3, 0..3)
        );
    }

    #[test]
    fn inserting_rows_inside_a_merge_grows_it_and_copies_the_style() {
        let mut grown = TableElement::new(3, 2);
        grown.rows[1][0].fill = Some(Fill::None);
        grown.merge(&CellRange::new(0..2, 1..2)).expect("merge");
        grown.insert_rows(1, 2, 1).expect("insert");
        assert_eq!(grown.row_count(), 5);
        assert_eq!(grown.horizontal.len(), 6);
        assert_eq!(grown.vertical.len(), 5);
        assert_eq!(grown.span_of(0, 1).rows, 4);
        assert_eq!(grown.rows[1][0].fill, Some(Fill::None));
        grown.validate().expect("valid");
    }

    #[test]
    fn removing_the_anchor_row_moves_the_anchor_down() {
        let mut shrunk = table(&[&["merged", "x"], &["", "y"], &["z", "w"]]);
        shrunk.merge(&CellRange::new(0..2, 0..1)).expect("merge");
        shrunk.remove_rows(0..1).expect("remove");
        assert_eq!(shrunk.rows[0][0].content, "merged");
        assert_eq!(
            shrunk.span_of(0, 0),
            Span {
                rows: 1,
                columns: 1
            }
        );
        assert_eq!(shrunk.rows[0][1].content, "y");
        shrunk.validate().expect("valid");
        assert_eq!(
            shrunk.remove_rows(0..2),
            Err(ApplyError::InvalidTable("a table keeps at least one row"))
        );
    }

    #[test]
    fn removing_rows_keeps_the_outer_edges() {
        let mut edged = TableElement::new(3, 1);
        let thick = Stroke {
            width: 8.,
            ..Stroke::default()
        };
        edged
            .set_borders(&edged.all(), Sides::Outside, Edge::Stroke(thick))
            .expect("borders");
        edged.remove_rows(2..3).expect("remove");
        assert_eq!(edged.horizontal[2][0], Edge::Stroke(thick));
        edged.remove_rows(0..1).expect("remove");
        assert_eq!(edged.horizontal[0][0], Edge::Stroke(thick));
        assert_eq!(edged.horizontal[1][0], Edge::Stroke(thick));
    }

    #[test]
    fn moving_rows_reorders_them_and_refuses_to_cut_a_merge() {
        let mut moved = table(&[&["a"], &["b"], &["c"]]);
        moved.move_rows(0..1, 3).expect("move");
        let order: Vec<&str> = moved
            .rows
            .iter()
            .map(|row| row[0].content.as_str())
            .collect();
        assert_eq!(order, ["b", "c", "a"]);
        moved.merge(&CellRange::new(0..2, 0..1)).expect("merge");
        assert_eq!(moved.move_rows(1..2, 3), Err(ApplyError::CutsMerge));
        assert_eq!(moved.move_rows(2..3, 1), Err(ApplyError::CutsMerge));
        moved.move_rows(2..3, 0).expect("move over the merge");
        assert_eq!(moved.rows[0][0].content, "a");
        moved.validate().expect("valid");
    }

    #[test]
    fn column_edits_work_like_row_edits() {
        let mut edited = table(&[&["a", "b"], &["c", "d"]]);
        edited.insert_columns(1, 1, 0).expect("insert");
        assert_eq!(edited.column_count(), 3);
        assert_eq!(edited.rows[0][2].content, "b");
        edited.move_columns(2..3, 0).expect("move");
        assert_eq!(edited.rows[1][0].content, "d");
        edited.remove_columns(1..3).expect("remove");
        assert_eq!(edited.rows[0][0].content, "b");
        assert_eq!(
            edited.remove_columns(0..1),
            Err(ApplyError::InvalidTable(
                "a table keeps at least one column"
            ))
        );
        edited.validate().expect("valid");
    }

    #[test]
    fn outside_borders_of_a_two_by_two_range_set_its_eight_outer_edges() {
        let mut edged = TableElement::new(4, 4);
        edged
            .set_borders(&CellRange::new(1..3, 1..3), Sides::Outside, Edge::None)
            .expect("borders");
        let set =
            |grid: &Vec<Vec<Edge>>| grid.iter().flatten().filter(|e| **e == Edge::None).count();
        assert_eq!(set(&edged.horizontal) + set(&edged.vertical), 8);
        assert_eq!(edged.horizontal[1][1], Edge::None);
        assert_eq!(edged.horizontal[3][2], Edge::None);
        assert_eq!(edged.vertical[2][3], Edge::None);
        assert_eq!(edged.horizontal[2][1], Edge::Inherit);
    }

    #[test]
    fn pasting_grows_the_table_and_splits_the_merges_it_touches() {
        let mut target = TableElement::new(2, 2);
        target.merge(&CellRange::new(0..2, 0..1)).expect("merge");
        let clip = Clip::from_tsv("a\tb\tc\nd\te\tf\n").expect("tsv");
        target.paste(1, 0, &clip).expect("paste");
        assert_eq!(target.row_count(), 3);
        assert_eq!(target.column_count(), 3);
        assert_eq!(
            target.span_of(0, 0),
            Span {
                rows: 1,
                columns: 1
            }
        );
        assert_eq!(target.rows[2][2].content, "f");
        target.validate().expect("valid");
    }

    #[test]
    fn a_styled_clip_brings_styles_and_merges() {
        let mut source = table(&[&["a", "b"], &["c", "d"]]);
        source.rows[0][0].fill = Some(Fill::None);
        source.merge(&CellRange::new(0..1, 0..2)).expect("merge");
        let clip = source.copy(&CellRange::cell(0, 1)).expect("copy");
        assert_eq!(clip.columns(), 2);
        let mut target = TableElement::new(1, 1);
        target.paste(0, 0, &clip).expect("paste");
        assert_eq!(target.span_of(0, 0).columns, 2);
        assert_eq!(target.rows[0][0].fill, Some(Fill::None));
        assert_eq!(target.rows[0][0].content, "a\nb");
    }

    #[test]
    fn tsv_round_trips_quoted_values() {
        let clip = table(&[&["plain", "two\nlines"], &["tab\there", "a \"quote\""]]);
        let clip = clip.copy(&clip.all()).expect("copy");
        let text = clip.to_tsv();
        let back = Clip::from_tsv(&text).expect("tsv");
        let contents = |clip: &Clip| -> Vec<String> {
            clip.cells
                .iter()
                .flatten()
                .map(|cell| cell.content.clone())
                .collect()
        };
        assert_eq!(contents(&back), contents(&clip));
        assert_eq!(Clip::from_tsv("just text"), None);
    }

    #[test]
    fn cell_overrides_change_only_their_cells() {
        let mut styled = TableElement::new(2, 2);
        let bold = TextStylePatch {
            size: Some(40.),
            ..TextStylePatch::default()
        };
        styled
            .set_cell_styles(
                &CellRange::new(0..1, 0..2),
                Some(Some(Fill::None)),
                &bold,
                false,
            )
            .expect("style");
        assert_eq!(styled.style_of(0, 1).size, 40.);
        assert_eq!(styled.style_of(1, 1).size, 24.);
        styled
            .set_cell_styles(
                &CellRange::cell(0, 0),
                Some(None),
                &TextStylePatch::default(),
                true,
            )
            .expect("reset");
        assert_eq!(styled.style_of(0, 0).size, 24.);
        assert_eq!(styled.rows[0][0].fill, None);
    }

    #[test]
    fn json_leaves_out_default_edges_and_reads_them_back() {
        let original = table(&[&["a", "b"]]);
        let json = serde_json::to_value(&original).expect("json");
        assert!(json.get("horizontal").is_none());
        let mut back: TableElement = serde_json::from_value(json).expect("read");
        back.normalize();
        assert_eq!(back, original);
    }

    #[test]
    fn parts_join_equal_edges_and_skip_the_inside_of_merges() {
        let presentation = with_inter();
        let mut merged = table(&[&["a", "b"], &["c", "d"]]);
        merged.merge(&CellRange::new(0..1, 0..2)).expect("merge");
        let layout = layout(&merged, &presentation.fonts).expect("layout");
        let parts = parts(ElementId(1), &Frame::default(), &merged, &layout);
        let borders = parts
            .iter()
            .filter(|part| matches!(part, Part::Border(_)))
            .count();
        // Three horizontal lines, left and right sides, and the middle line
        // of the second row only.
        assert_eq!(borders, 3 + 2 + 1);
        let texts = parts
            .iter()
            .filter(|part| matches!(part, Part::Text { .. }))
            .count();
        assert_eq!(texts, 3);
    }
}
