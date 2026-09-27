//! Tables on the canvas: the table tool, the cell selection, the keys that
//! act on cells, the clipboard of cells and the row and column commands of
//! the context menu, the properties panel and the "+" buttons.
//!
//! Every change is built with the edits of [`crate::table`] and committed
//! as one [`Operation::SetTable`], so it is one undo step.

use std::ops::Range;

use gpui_kit::{ClipboardItem, Context, Keystroke, MouseDownEvent, Pixels, Point};

use crate::document::{Element, ElementId, ElementKind, Frame, Operation, TableElement};
use crate::editor::{Drag, EditorView, SlidePoint, TableEdit, Tool};
use crate::table::{CellRange, Clip, DRAW_STEP};

/// How far outside a selected table the "+" buttons show, in screen pixels.
const INSERT_REACH: f32 = 28.;

/// How close to a line between two rows or columns the pointer brings its
/// "+" button, in screen pixels.
const INSERT_SNAP: f32 = 12.;

/// Distance of the center of a "+" button from the table edge, in screen
/// pixels.
pub const INSERT_OFFSET: f32 = 14.;

impl EditorView {
    /// The selected table, its cells and the selected range.
    pub fn selected_cells(&self) -> Option<(ElementId, &TableElement, CellRange)> {
        let edit = self.table_edit.as_ref()?;
        let table = self.presentation.element(edit.id)?.as_table()?;
        Some((edit.id, table, edit.range(table)))
    }

    /// The table when it is the only selected element.
    pub fn selected_table(&self) -> Option<(ElementId, &TableElement)> {
        let id = self.single_selection()?;
        Some((id, self.presentation.element(id)?.as_table()?))
    }

    /// Drops a cell selection or cell edit that no longer fits the table,
    /// or whose table is not the selection.
    pub fn repair_table_edit(&mut self) {
        if let Some(edit) = &self.table_edit {
            let table = self
                .presentation
                .element(edit.id)
                .and_then(Element::as_table);
            match table {
                Some(table) if self.selection == [edit.id] => {
                    let clamp = |(row, column): (usize, usize)| {
                        (
                            row.min(table.row_count() - 1),
                            column.min(table.column_count() - 1),
                        )
                    };
                    let (anchor, focus) = (clamp(edit.anchor), clamp(edit.focus));
                    self.table_edit = Some(TableEdit {
                        id: edit.id,
                        anchor,
                        focus,
                    });
                }
                _ => self.table_edit = None,
            }
        }
        if let Some(edit) = &self.text_edit
            && let Some((row, column)) = edit.cell
        {
            let drawn = self
                .presentation
                .element(edit.id)
                .and_then(Element::as_table)
                .is_some_and(|table| {
                    table.cell(row, column).is_some()
                        && table.anchor_of(row, column) == (row, column)
                });
            if !drawn
                || self
                    .table_edit
                    .as_ref()
                    .is_none_or(|table| table.id != edit.id)
            {
                self.text_edit = None;
            }
        }
    }

    /// The anchor of the cell of table `id` under a slide point, clamped to
    /// the table.
    pub fn table_cell_at(&mut self, id: ElementId, at: SlidePoint) -> Option<(usize, usize)> {
        let frame = self.presentation.element(id)?.frame;
        let view = self.tables.get(&self.presentation, id)?;
        let (x, y) = frame.to_local(at.x, at.y);
        let (row, column) = view.layout.grid_at(x, y);
        Some(view.table.anchor_of(row, column))
    }

    /// Selects one cell of a table, leaving any text edit.
    pub fn select_cell(&mut self, id: ElementId, cell: (usize, usize)) {
        self.end_text_edit();
        self.selection = vec![id];
        self.table_edit = Some(TableEdit::cell(id, cell.0, cell.1));
    }

    /// Edits the text of a cell at a pointer press: places the caret, or
    /// selects the word on a double click.
    pub fn press_cell_text(
        &mut self,
        id: ElementId,
        cell: (usize, usize),
        at: SlidePoint,
        event: &MouseDownEvent,
    ) {
        let Some((layout, frame)) = self.cell_box(id, cell.0, cell.1) else {
            return;
        };
        let (x, y) = frame.to_local(at.x, at.y);
        let index = layout.index_at(x, y);
        let editing = self
            .text_edit
            .as_ref()
            .is_some_and(|edit| edit.id == id && edit.cell == Some(cell));
        if !editing {
            self.begin_cell_edit(id, cell, index, index);
        }
        if event.click_count >= 2 {
            self.select_word(index);
        } else {
            self.move_caret(index, editing && event.modifiers.shift);
        }
        self.drag = Some(Drag::SelectText { id });
    }

    /// A left press inside table `id` while its cells are selected or
    /// edited.
    pub fn press_table(&mut self, id: ElementId, at: SlidePoint, event: &MouseDownEvent) {
        let Some(cell) = self.table_cell_at(id, at) else {
            return;
        };
        let editing = self
            .text_edit
            .as_ref()
            .is_some_and(|edit| edit.id == id && edit.cell == Some(cell));
        if editing || event.click_count >= 2 {
            self.press_cell_text(id, cell, at, event);
            return;
        }
        if event.modifiers.shift
            && let Some(edit) = self.table_edit.clone().filter(|edit| edit.id == id)
        {
            self.end_text_edit();
            self.table_edit = Some(TableEdit {
                focus: cell,
                ..edit
            });
            self.drag = Some(Drag::SelectCells {
                id,
                anchor: edit.anchor,
            });
            return;
        }
        if let Some((_, table, range)) = self.selected_cells()
            && range.contains(cell.0, cell.1)
            && self.text_edit.is_none()
        {
            let whole_rows = range.columns.len() == table.column_count();
            let whole_columns = range.rows.len() == table.row_count();
            if whole_rows != whole_columns {
                let range = if whole_rows {
                    range.rows
                } else {
                    range.columns
                };
                let to = range.start;
                self.drag = Some(Drag::MoveCells {
                    id,
                    rows: whole_rows,
                    range,
                    to,
                });
                return;
            }
        }
        self.select_cell(id, cell);
        self.drag = Some(Drag::SelectCells { id, anchor: cell });
    }

    /// The line a dragged row or column goes before, from a slide point:
    /// the nearest line between two rows (`rows`) or columns.
    pub fn table_line_at(&mut self, id: ElementId, rows: bool, at: SlidePoint) -> Option<usize> {
        let frame = self.presentation.element(id)?.frame;
        let view = self.tables.get(&self.presentation, id)?;
        let (x, y) = frame.to_local(at.x, at.y);
        let (edges, at) = if rows {
            (view.layout.row_edges(), y)
        } else {
            (view.layout.column_edges(), x)
        };
        edges
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - at).abs().total_cmp(&(b.1 - at).abs()))
            .map(|(line, _)| line)
    }

    /// Commits a new version of table `id` as one undo step, keeping the
    /// table selected and the cells `select` selected.
    pub fn commit_table(
        &mut self,
        label: &str,
        id: ElementId,
        table: TableElement,
        select: Option<TableEdit>,
    ) -> bool {
        self.end_text_edit();
        let committed = self.commit(
            label,
            Operation::SetTable {
                id,
                table: Box::new(table),
            },
            vec![id],
        );
        if committed {
            self.table_edit = select;
            self.repair_table_edit();
        }
        committed
    }

    /// Changes the selected table with `edit`, which gets the table and the
    /// selected range and returns the cells to select after it.
    pub fn edit_cells(
        &mut self,
        label: &str,
        edit: impl FnOnce(
            &mut TableElement,
            &CellRange,
        ) -> Result<Option<CellRange>, crate::document::ApplyError>,
    ) -> bool {
        let Some((id, table, range)) = self.selected_cells() else {
            return false;
        };
        if self.presentation.is_locked(id) {
            return false;
        }
        let mut table = table.clone();
        match edit(&mut table, &range) {
            Ok(select) => {
                let select = select.map(|range| TableEdit {
                    id,
                    anchor: (range.rows.start, range.columns.start),
                    focus: (range.rows.end - 1, range.columns.end - 1),
                });
                self.commit_table(label, id, table, select)
            }
            Err(error) => {
                eprintln!("sliderino: {label} failed: {error}");
                false
            }
        }
    }

    pub fn insert_rows_near(&mut self, below: bool) -> bool {
        self.edit_cells("Insert row", |table, range| {
            let (at, like) = if below {
                (range.rows.end, range.rows.end - 1)
            } else {
                (range.rows.start, range.rows.start)
            };
            table.insert_rows(at, 1, like)?;
            Ok(Some(CellRange::new(at..at + 1, range.columns.clone())))
        })
    }

    pub fn insert_columns_near(&mut self, right: bool) -> bool {
        self.edit_cells("Insert column", |table, range| {
            let (at, like) = if right {
                (range.columns.end, range.columns.end - 1)
            } else {
                (range.columns.start, range.columns.start)
            };
            table.insert_columns(at, 1, like)?;
            Ok(Some(CellRange::new(range.rows.clone(), at..at + 1)))
        })
    }

    /// Inserts a row (`rows`) or a column before line `index`: the "+"
    /// buttons of the canvas and the counters of the panel.
    pub fn insert_line(&mut self, id: ElementId, rows: bool, index: usize) -> bool {
        let Some(table) = self.presentation.element(id).and_then(Element::as_table) else {
            return false;
        };
        if self.presentation.is_locked(id) {
            return false;
        }
        let mut table = table.clone();
        let count = if rows {
            table.row_count()
        } else {
            table.column_count()
        };
        let like = index.saturating_sub(1).min(count - 1);
        let (result, label) = if rows {
            (table.insert_rows(index, 1, like), "Insert row")
        } else {
            (table.insert_columns(index, 1, like), "Insert column")
        };
        if result.is_err() {
            return false;
        }
        let select = if rows {
            TableEdit::cell(id, index, 0)
        } else {
            TableEdit::cell(id, 0, index)
        };
        self.commit_table(label, id, table, Some(select))
    }

    /// Removes the last row (`rows`) or column: the "−" of the panel.
    pub fn remove_last_line(&mut self, id: ElementId, rows: bool) -> bool {
        let Some(table) = self.presentation.element(id).and_then(Element::as_table) else {
            return false;
        };
        let mut table = table.clone();
        let (result, label) = if rows {
            let last = table.row_count() - 1;
            (table.remove_rows(last..last + 1), "Delete rows")
        } else {
            let last = table.column_count() - 1;
            (table.remove_columns(last..last + 1), "Delete columns")
        };
        if result.is_err() {
            return false;
        }
        let select = self.table_edit.clone();
        self.commit_table(label, id, table, select)
    }

    pub fn delete_rows(&mut self) -> bool {
        self.edit_cells("Delete rows", |table, range| {
            table.remove_rows(range.rows.clone())?;
            let row = range.rows.start.min(table.row_count() - 1);
            Ok(Some(CellRange::cell(row, range.columns.start)))
        })
    }

    pub fn delete_columns(&mut self) -> bool {
        self.edit_cells("Delete columns", |table, range| {
            table.remove_columns(range.columns.clone())?;
            let column = range.columns.start.min(table.column_count() - 1);
            Ok(Some(CellRange::cell(range.rows.start, column)))
        })
    }

    /// Moves the selected rows (`rows`) or columns one step back or
    /// forward.
    pub fn move_lines_by(&mut self, rows: bool, forward: bool) -> bool {
        let label = if rows { "Move rows" } else { "Move columns" };
        self.edit_cells(label, |table, range| {
            let lines = if rows {
                range.rows.clone()
            } else {
                range.columns.clone()
            };
            let count = if rows {
                table.row_count()
            } else {
                table.column_count()
            };
            if (forward && lines.end >= count) || (!forward && lines.start == 0) {
                return Err(crate::document::ApplyError::InvalidTable("no room to move"));
            }
            // The neighbor may be merged: step over all of it.
            let neighbor = if forward {
                let probe = if rows {
                    table.expand(&CellRange::new(
                        lines.end..lines.end + 1,
                        range.columns.clone(),
                    ))
                } else {
                    table.expand(&CellRange::new(
                        range.rows.clone(),
                        lines.end..lines.end + 1,
                    ))
                };
                if rows { probe.rows } else { probe.columns }
            } else {
                let probe = if rows {
                    table.expand(&CellRange::new(
                        lines.start - 1..lines.start,
                        range.columns.clone(),
                    ))
                } else {
                    table.expand(&CellRange::new(
                        range.rows.clone(),
                        lines.start - 1..lines.start,
                    ))
                };
                if rows { probe.rows } else { probe.columns }
            };
            let to = if forward {
                neighbor.end
            } else {
                neighbor.start
            };
            let moved = move_lines(table, rows, lines.clone(), to)?;
            Ok(Some(if rows {
                CellRange::new(moved, range.columns.clone())
            } else {
                CellRange::new(range.rows.clone(), moved)
            }))
        })
    }

    /// Moves rows (`rows`) or columns `range` of table `id` before line
    /// `to`, the end of a drag of whole rows or columns.
    pub fn move_lines_to(&mut self, id: ElementId, rows: bool, range: Range<usize>, to: usize) {
        if to >= range.start && to <= range.end {
            return;
        }
        let label = if rows { "Move rows" } else { "Move columns" };
        let Some(table) = self.presentation.element(id).and_then(Element::as_table) else {
            return;
        };
        let mut table = table.clone();
        match move_lines(&mut table, rows, range, to) {
            Ok(moved) => {
                let select = if rows {
                    TableEdit {
                        id,
                        anchor: (moved.start, 0),
                        focus: (moved.end - 1, table.column_count() - 1),
                    }
                } else {
                    TableEdit {
                        id,
                        anchor: (0, moved.start),
                        focus: (table.row_count() - 1, moved.end - 1),
                    }
                };
                self.commit_table(label, id, table, Some(select));
            }
            Err(error) => eprintln!("sliderino: {label} failed: {error}"),
        }
    }

    pub fn merge_cells(&mut self) -> bool {
        self.edit_cells("Merge cells", |table, range| {
            table.merge(range)?;
            Ok(Some(range.clone()))
        })
    }

    pub fn split_cells(&mut self) -> bool {
        self.edit_cells("Split cell", |table, range| {
            for (row, column) in table.anchors() {
                if range.contains(row, column) {
                    table.split(row, column)?;
                }
            }
            Ok(Some(CellRange::cell(range.rows.start, range.columns.start)))
        })
    }

    pub fn clear_cells(&mut self) -> bool {
        self.edit_cells("Clear cells", |table, range| {
            table.clear(range)?;
            Ok(Some(range.clone()))
        })
    }

    /// Whether the selected cells can merge: more than one cell.
    pub fn can_merge(&self) -> bool {
        self.selected_cells().is_some_and(|(_, table, range)| {
            table
                .anchors()
                .into_iter()
                .filter(|(row, column)| range.contains(*row, *column))
                .count()
                > 1
        })
    }

    /// Whether the selected cells hold a merge to split.
    pub fn can_split(&self) -> bool {
        self.selected_cells().is_some_and(|(_, table, range)| {
            table.anchors().into_iter().any(|(row, column)| {
                range.contains(row, column)
                    && table
                        .cell(row, column)
                        .is_some_and(|cell| cell.span.is_some())
            })
        })
    }

    /// Copies the selected cells: their text as tab-separated values for
    /// other applications, and the cells themselves for a paste here.
    pub fn copy_cells(&mut self, cut: bool, cx: &mut Context<Self>) -> bool {
        let Some((_, table, range)) = self.selected_cells() else {
            return false;
        };
        let Ok(clip) = table.copy(&range) else {
            return false;
        };
        let text = clip.to_tsv();
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.table_clip = Some((text, clip));
        if cut {
            self.clear_cells();
        }
        true
    }

    /// The cells on the clipboard: the ones copied here when the text still
    /// matches, else tab-separated values, else one cell of text.
    fn clipboard_clip(&self, cx: &mut Context<Self>) -> Option<Clip> {
        let text = cx.read_from_clipboard()?.text()?;
        if let Some((copied, clip)) = &self.table_clip
            && *copied == text
        {
            return Some(clip.clone());
        }
        Clip::from_tsv(&text).or_else(|| {
            Some(Clip {
                cells: vec![vec![crate::table::Cell::text(&text)]],
                styled: false,
            })
        })
    }

    /// Pastes the clipboard into the selected cells, from the top-left one.
    pub fn paste_cells(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(clip) = self.clipboard_clip(cx) else {
            return false;
        };
        self.edit_cells("Paste cells", |table, range| {
            let (row, column) = (range.rows.start, range.columns.start);
            table.paste(row, column, &clip)?;
            Ok(Some(CellRange::new(
                row..row + clip.rows(),
                column..column + clip.columns(),
            )))
        })
    }

    /// Pastes tab-separated values, or cells copied here, as a new table in
    /// the middle of the slide. False when the clipboard holds no table.
    pub fn paste_table(&mut self, cx: &mut Context<Self>) -> bool {
        if self.text_edit.is_some() {
            return false;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        let clip = match &self.table_clip {
            Some((copied, clip)) if *copied == text => clip.clone(),
            _ => match Clip::from_tsv(&text) {
                Some(clip) => clip,
                None => return false,
            },
        };
        let mut table = TableElement::from_clip(&clip);
        table.normalize();
        let size = self.presentation.size;
        let center = (size.width as f32 / 2., size.height as f32 / 2.);
        let layout = crate::table::layout(&table, &self.presentation.fonts)
            .or_else(|_| {
                // The default face may not be embedded yet.
                let face = table.text.font.clone();
                let data = crate::fonts::data(&face).ok_or(crate::text_layout::LayoutError)?;
                crate::table::layout(&table, &self.presentation.fonts.with(face, data))
            })
            .ok();
        let (width, height) = layout.map_or((0., 0.), |layout| (layout.width(), layout.height()));
        let at = (center.0 - width / 2., center.1 - height / 2.);
        self.insert_table(at, table, "Paste table")
    }

    /// Adds `table` with its top-left corner at `at` on the current slide,
    /// embedding its faces when needed, and selects it.
    fn insert_table(&mut self, at: (f32, f32), table: TableElement, label: &str) -> bool {
        let mut operations = Vec::new();
        for face in table.faces() {
            if !self.presentation.fonts.contains(&face) {
                let Some(data) = crate::fonts::data(&face) else {
                    eprintln!("sliderino: the font of the table is missing");
                    return false;
                };
                operations.push(Operation::AddFont { face, data });
            }
        }
        let id = self.presentation.new_element_id();
        let frame = Frame {
            x: at.0,
            y: at.1,
            ..Frame::default()
        };
        operations.push(Operation::AddElement {
            slide: self.current_slide,
            parent: None,
            index: usize::MAX,
            element: Element::new(id, frame, ElementKind::Table(Box::new(table))),
        });
        self.end_text_edit();
        self.table_edit = None;
        self.commit(label, Operation::Batch(operations), vec![id])
    }

    /// Adds an empty table of `rows` × `columns` with its top-left corner at
    /// `at`, and edits its first cell.
    pub fn create_table(&mut self, at: SlidePoint, rows: usize, columns: usize) {
        let table = TableElement::new(rows, columns);
        if self.insert_table((at.x, at.y), table, "Create table")
            && let Some(id) = self.single_selection()
        {
            self.begin_cell_edit(id, (0, 0), 0, 0);
        }
        self.active_tool = Tool::Move;
    }

    /// Moves the text edit, or the cell selection, to the next cell in
    /// reading order (the previous one with `back`) and selects its text.
    /// Stops at the first and the last cell.
    pub fn tab_cell(&mut self, back: bool) {
        let Some((id, table, range)) = self.selected_cells() else {
            return;
        };
        let anchors = table.anchors();
        let from = self
            .text_edit
            .as_ref()
            .and_then(|edit| edit.cell)
            .unwrap_or((range.rows.start, range.columns.start));
        let Some(index) = anchors.iter().position(|cell| *cell == from) else {
            return;
        };
        let next = if back {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|next| *next < anchors.len())
        };
        let Some(cell) = next.map(|next| anchors[next]) else {
            return;
        };
        let len = table
            .cell(cell.0, cell.1)
            .map_or(0, |cell| cell.content.len());
        self.begin_cell_edit(id, cell, 0, len);
    }

    /// Moves the cell selection by one cell, over merges; `extend` moves
    /// only its focus.
    fn step_cell(&mut self, rows: isize, columns: isize, extend: bool) {
        let Some((id, table, _)) = self.selected_cells() else {
            return;
        };
        let Some(edit) = self.table_edit.clone() else {
            return;
        };
        let (row, column) = edit.focus;
        let span = table.span_of(
            table.anchor_of(row, column).0,
            table.anchor_of(row, column).1,
        );
        let (anchor_row, anchor_column) = table.anchor_of(row, column);
        let step = |at: usize, delta: isize, span: usize, anchor: usize, count: usize| {
            if delta > 0 {
                (anchor + span).min(count - 1)
            } else if delta < 0 {
                anchor.saturating_sub(1)
            } else {
                at
            }
        };
        let target = (
            step(row, rows, span.rows, anchor_row, table.row_count()),
            step(
                column,
                columns,
                span.columns,
                anchor_column,
                table.column_count(),
            ),
        );
        let target = if extend {
            target
        } else {
            table.anchor_of(target.0, target.1)
        };
        self.table_edit = Some(if extend {
            TableEdit {
                focus: target,
                ..edit
            }
        } else {
            TableEdit::cell(id, target.0, target.1)
        });
    }

    /// Keys that act on the selected cells while no text is edited. Returns
    /// false for the other keys: typed characters reach the input handler
    /// and replace the text of the cell.
    pub fn on_table_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        if self.table_edit.is_none() || self.text_edit.is_some() {
            return false;
        }
        let modifiers = &keystroke.modifiers;
        let shift = modifiers.shift;
        if modifiers.secondary() && !modifiers.alt && !shift {
            return match keystroke.key.as_str() {
                "c" => self.copy_cells(false, cx),
                "x" => self.copy_cells(true, cx),
                "v" => self.paste_cells(cx),
                _ => false,
            };
        }
        if modifiers.control || modifiers.alt || modifiers.platform {
            return false;
        }
        match keystroke.key.as_str() {
            "up" => self.step_cell(-1, 0, shift),
            "down" => self.step_cell(1, 0, shift),
            "left" => self.step_cell(0, -1, shift),
            "right" => self.step_cell(0, 1, shift),
            "tab" => self.tab_cell(shift),
            "enter" if !shift => {
                let Some((id, table, range)) = self.selected_cells() else {
                    return false;
                };
                let cell = (range.rows.start, range.columns.start);
                let len = table
                    .cell(cell.0, cell.1)
                    .map_or(0, |cell| cell.content.len());
                self.begin_cell_edit(id, cell, len, len);
            }
            "backspace" | "delete" if !shift => {
                self.clear_cells();
            }
            "escape" => self.table_edit = None,
            _ => return false,
        }
        true
    }

    /// Starts editing the selected cell when the author types over it: its
    /// text is selected, so the typed text replaces it.
    pub fn type_over_cell(&mut self) -> bool {
        if self.text_edit.is_some() {
            return true;
        }
        let Some((id, table, range)) = self.selected_cells() else {
            return false;
        };
        if self.presentation.is_locked(id) {
            return false;
        }
        let cell = (range.rows.start, range.columns.start);
        let len = table
            .cell(cell.0, cell.1)
            .map_or(0, |cell| cell.content.len());
        self.begin_cell_edit(id, cell, 0, len);
        true
    }

    /// The "+" button of the selected table under a window position: a row
    /// (`true`) or a column, and the line it inserts before.
    pub fn table_insert_at(&mut self, position: Point<Pixels>) -> Option<(bool, usize)> {
        if self.effective_tool() != Tool::Move || self.drag.is_some() {
            return None;
        }
        let (id, _) = self.selected_table()?;
        if self.presentation.is_locked(id) {
            return None;
        }
        let zoom = self.camera?.zoom;
        let at = self.to_slide(position)?;
        let frame = self.presentation.element(id)?.frame;
        let view = self.tables.get(&self.presentation, id)?;
        let (x, y) = frame.to_local(at.x, at.y);
        let reach = INSERT_REACH / zoom;
        let snap = INSERT_SNAP / zoom;
        let nearest = |edges: Vec<f32>, at: f32| {
            edges
                .iter()
                .enumerate()
                .map(|(line, edge)| (line, (edge - at).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, distance)| *distance <= snap)
                .map(|(line, _)| line)
        };
        if (-reach..0.).contains(&y) && (-snap..frame.width + snap).contains(&x) {
            return nearest(view.layout.column_edges(), x).map(|line| (false, line));
        }
        if (-reach..0.).contains(&x) && (-snap..frame.height + snap).contains(&y) {
            return nearest(view.layout.row_edges(), y).map(|line| (true, line));
        }
        None
    }

    /// The rows and columns a drag of the table tool gives.
    pub fn drawn_table_size(start: SlidePoint, current: SlidePoint) -> (usize, usize) {
        let count = |delta: f32, step: f32| ((delta.abs() / step).ceil() as usize).max(1);
        (
            count(current.y - start.y, DRAW_STEP.1),
            count(current.x - start.x, DRAW_STEP.0),
        )
    }
}

/// Moves rows (`rows`) or columns `lines` before line `to` and returns
/// where they are after the move.
fn move_lines(
    table: &mut TableElement,
    rows: bool,
    lines: Range<usize>,
    to: usize,
) -> Result<Range<usize>, crate::document::ApplyError> {
    if rows {
        table.move_rows(lines.clone(), to)?;
    } else {
        table.move_columns(lines.clone(), to)?;
    }
    let start = if to > lines.end { to - lines.len() } else { to };
    Ok(start..start + lines.len())
}

/// Style edits of the properties panel on a table: without selected cells
/// they change the style of the whole table, with selected cells the
/// overrides of those cells and the edges of the chosen sides.
impl EditorView {
    /// The rectangle that stands for a table in the fill and stroke
    /// inspector: the fill of the table or of the first selected cell, and
    /// the stroke of the table or of the chosen edges.
    pub fn table_proxy(&self, id: ElementId) -> Option<ElementKind> {
        let table = self.presentation.element(id)?.as_table()?;
        let (fill, stroke) = match self
            .selected_cells()
            .filter(|(table_id, ..)| *table_id == id)
        {
            Some((_, _, range)) => {
                let first = table
                    .anchors()
                    .into_iter()
                    .find(|(row, column)| range.contains(*row, *column))
                    .unwrap_or((range.rows.start, range.columns.start));
                let strokes = table.border_strokes(&range, self.border_sides);
                let stroke = strokes.iter().flatten().next().copied();
                (table.fill_of(first.0, first.1).clone(), stroke)
            }
            None => (table.fill.clone(), table.stroke),
        };
        Some(ElementKind::Rectangle(crate::document::RectangleElement {
            fill,
            stroke,
            corner_radius: 0.,
        }))
    }

    /// The operation of a fill or stroke patch on a shape, or on a table
    /// through its stand-in rectangle.
    pub fn style_operation(
        &self,
        id: ElementId,
        patch: crate::document::ShapeStylePatch,
    ) -> Option<Operation> {
        let Some(table) = self.presentation.element(id)?.as_table() else {
            return Some(Operation::SetShapeStyle { id, patch });
        };
        let mut table = table.clone();
        let cells = self
            .selected_cells()
            .filter(|(table_id, ..)| *table_id == id)
            .map(|(_, _, range)| range);
        if let Some(fill) = patch.fill {
            match &cells {
                Some(range) => table
                    .set_cell_styles(range, Some(Some(fill)), &Default::default(), false)
                    .ok()?,
                None => table.fill = fill,
            }
        }
        if let Some(stroke) = patch.stroke {
            match &cells {
                Some(range) => {
                    let edge = stroke.map_or(crate::table::Edge::None, crate::table::Edge::Stroke);
                    table.set_borders(range, self.border_sides, edge).ok()?;
                }
                None => table.stroke = stroke,
            }
        }
        Some(Operation::SetTable {
            id,
            table: Box::new(table),
        })
    }

    /// The operation of a text style patch on a text, or on a table: its
    /// text style, or the overrides of the selected cells.
    pub fn text_style_operation(
        &self,
        id: ElementId,
        patch: crate::document::TextStylePatch,
    ) -> Option<Operation> {
        let Some(table) = self.presentation.element(id)?.as_table() else {
            return Some(Operation::SetTextStyle { id, patch });
        };
        let mut table = table.clone();
        match self
            .selected_cells()
            .filter(|(table_id, ..)| *table_id == id)
        {
            Some((_, _, range)) => table.set_cell_styles(&range, None, &patch, false).ok()?,
            None => {
                patch.validate().ok()?;
                patch.apply_to(&mut table.text);
            }
        }
        Some(Operation::SetTable {
            id,
            table: Box::new(table),
        })
    }

    /// Replaces the text style operations on tables in `operation` by
    /// their table edits.
    pub fn route_table_styles(&self, operation: Operation) -> Operation {
        match operation {
            Operation::SetTextStyle { id, patch }
                if self
                    .presentation
                    .element(id)
                    .is_some_and(|element| element.as_table().is_some()) =>
            {
                self.text_style_operation(id, patch)
                    .unwrap_or(Operation::Batch(Vec::new()))
            }
            Operation::Batch(operations) => Operation::Batch(
                operations
                    .into_iter()
                    .map(|operation| self.route_table_styles(operation))
                    .collect(),
            ),
            operation => operation,
        }
    }

    /// The text style the panel shows for a table: that of the first
    /// selected cell, or of the table.
    pub fn table_text_style(&self, id: ElementId) -> Option<crate::document::TextStyle> {
        let table = self.presentation.element(id)?.as_table()?;
        Some(
            match self
                .selected_cells()
                .filter(|(table_id, ..)| *table_id == id)
            {
                Some((_, table, range)) => table
                    .anchors()
                    .into_iter()
                    .find(|(row, column)| range.contains(*row, *column))
                    .map_or_else(
                        || table.text.clone(),
                        |(row, column)| table.style_of(row, column),
                    ),
                None => table.text.clone(),
            },
        )
    }

    /// Removes the fill and text overrides of the selected cells and gives
    /// their edges back to the table stroke.
    pub fn reset_cells(&mut self) -> bool {
        self.edit_cells("Reset cells", |table, range| {
            table.set_cell_styles(range, Some(None), &Default::default(), true)?;
            table.set_borders(range, crate::table::Sides::All, crate::table::Edge::Inherit)?;
            Ok(Some(range.clone()))
        })
    }

    /// Sets the padding of the selected table.
    pub fn set_table_padding(&mut self, padding: f32) -> bool {
        let Some((id, table)) = self.selected_table() else {
            return false;
        };
        if !(padding.is_finite() && padding >= 0.) || padding == table.padding {
            return false;
        }
        let mut table = table.clone();
        table.padding = padding;
        let select = self.table_edit.clone();
        self.commit_table("Cell padding", id, table, select)
    }

    /// Gives an axis of the selected table back to the size of its content.
    pub fn set_table_auto(&mut self, width: bool) -> bool {
        let Some((id, table)) = self.selected_table() else {
            return false;
        };
        let mut table = table.clone();
        let axis = if width {
            &mut table.width
        } else {
            &mut table.height
        };
        if *axis == crate::table::TableSizing::Auto {
            return false;
        }
        *axis = crate::table::TableSizing::Auto;
        let select = self.table_edit.clone();
        self.commit_table("Table size", id, table, select)
    }
}

/// The field of the properties panel that only tables have: the padding of
/// their cells.
pub struct TableInspector {
    pub padding: gpui_kit::Entity<gpui_kit::component::input::InputState>,
    /// The author typed into the field and has not committed it yet.
    dirty: bool,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

impl TableInspector {
    pub fn new(window: &mut gpui_kit::Window, cx: &mut Context<EditorView>) -> Self {
        use gpui_kit::AppContext as _;
        use gpui_kit::component::input::{InputEvent, InputState};
        let padding = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(
            &padding,
            window,
            |this: &mut EditorView, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.table_inspector.dirty = true,
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.commit_table_padding(window, cx);
                }
                InputEvent::Focus => {}
            },
        );
        Self {
            padding,
            dirty: false,
            _subscriptions: vec![subscription],
        }
    }
}

impl EditorView {
    /// Shows the padding of the selected table in its field, unless the
    /// author is typing in it. Runs before every render.
    pub fn sync_table_inspector(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        use gpui_kit::Focusable as _;
        let input = self.table_inspector.padding.clone();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        if self.table_inspector.dirty && !focused {
            self.commit_table_padding(window, cx);
        }
        let Some((_, table)) = self.selected_table() else {
            return;
        };
        let shown = crate::ui::inspector::number(table.padding);
        if !focused && input.read(cx).value() != shown.as_str() {
            input.update(cx, |state, cx| state.set_value(shown, window, cx));
        }
    }

    fn commit_table_padding(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        self.table_inspector.dirty = false;
        let input = self.table_inspector.padding.clone();
        let typed = input.read(cx).value().to_string();
        if let Some(value) = crate::ui::inspector::parse(&typed) {
            self.set_table_padding(value.max(0.));
        }
        if let Some((_, table)) = self.selected_table() {
            let shown = crate::ui::inspector::number(table.padding);
            input.update(cx, |state, cx| state.set_value(shown, window, cx));
        }
        cx.notify();
    }
}
