//! End-to-end tests of tables in a test window: the table tool, editing
//! cells, the cell selection and its keys, the row and column commands, the
//! "+" buttons, moving rows, resizing and the properties panel.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    InputEvent as _, Modifiers, MouseButton, MouseMoveEvent, Pixels, Point, TestAppContext,
    WindowHandle,
};

use crate::document::tests::{add_shape, with_inter};
use crate::document::{ElementId, ElementKind, Fill, Frame, SolidFill, TableElement};
use crate::editor::{EditorView, Tool};
use crate::table::{Cell, CellRange, Clip, DRAW_STEP, Edge, Sides, TableSizing};
use crate::ui::shape_inspector::FillType;
use crate::ui::test_support::{
    click_at, click_with, drag_with, key, open, open_with, read, with_editor, with_window,
};

fn at(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, x: f32, y: f32) -> Point<Pixels> {
    read(cx, handle, |editor| editor.to_window(x, y).unwrap())
}

fn history(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<String> {
    read(cx, handle, |editor| {
        editor.history.done().map(String::from).collect()
    })
}

fn table_of(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    id: ElementId,
) -> TableElement {
    read(cx, handle, |editor| {
        editor
            .presentation
            .element(id)
            .unwrap()
            .as_table()
            .unwrap()
            .clone()
    })
}

fn frame_of(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, id: ElementId) -> Frame {
    read(cx, handle, |editor| {
        editor.presentation.element(id).unwrap().frame
    })
}

fn contents(table: &TableElement) -> Vec<Vec<String>> {
    table
        .rows
        .iter()
        .map(|row| row.iter().map(|cell| cell.content.clone()).collect())
        .collect()
}

/// A slide with one table of these texts at (100, 100).
fn with_table(cx: &mut TestAppContext, cells: &[&[&str]]) -> (WindowHandle<EditorView>, ElementId) {
    let mut presentation = with_inter();
    let clip = Clip {
        cells: cells
            .iter()
            .map(|row| row.iter().map(|text| Cell::text(text)).collect())
            .collect(),
        styled: false,
    };
    let id = add_shape(
        &mut presentation,
        Frame {
            x: 100.,
            y: 100.,
            ..Frame::default()
        },
        ElementKind::Table(Box::new(TableElement::from_clip(&clip))),
    );
    (open_with(cx, presentation), id)
}

/// The slide point at the middle of a cell.
fn cell_center(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    id: ElementId,
    row: usize,
    column: usize,
) -> Point<Pixels> {
    let (x, y) = with_editor(cx, handle, |editor, _| {
        let frame = editor.presentation.element(id).unwrap().frame;
        let view = editor.tables.get(&editor.presentation, id).unwrap();
        let rect = view.layout.range_rect(&CellRange::cell(row, column));
        frame.to_slide(rect.x + rect.width / 2., rect.y + rect.height / 2.)
    });
    at(cx, handle, x, y)
}

fn click_cell(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    id: ElementId,
    row: usize,
    column: usize,
    count: usize,
) {
    let position = cell_center(cx, handle, id, row, column);
    with_window(cx, handle, |window, cx| {
        click_at(window, position, count, cx)
    });
}

#[gpui_kit::test]
fn dragging_with_the_table_tool_draws_a_compact_table_and_edits_its_first_cell(
    cx: &mut TestAppContext,
) {
    let handle = open(cx);
    let from = at(cx, handle, 100., 100.);
    let to = at(
        cx,
        handle,
        100. + DRAW_STEP.0 * 3.5,
        100. + DRAW_STEP.1 * 1.5,
    );
    with_window(cx, handle, |window, cx| {
        window.click("tool-table", cx);
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        );
        window.input("Name", cx);
    });
    let (id, tool, editing) = read(cx, handle, |editor| {
        let elements = &editor.current_slide().elements;
        assert_eq!(elements.len(), 1);
        (
            elements[0].id,
            editor.active_tool,
            editor.text_edit.as_ref().and_then(|edit| edit.cell),
        )
    });
    let table = table_of(cx, handle, id);
    assert_eq!((table.row_count(), table.column_count()), (2, 4));
    assert_eq!(table.width, TableSizing::Auto);
    assert_eq!(table.rows[0][0].content, "Name");
    assert_eq!(tool, Tool::Move);
    assert_eq!(editing, Some((0, 0)));
    let frame = frame_of(cx, handle, id);
    assert!(
        frame.width < DRAW_STEP.0 * 4.,
        "compact, not the drag: {frame:?}"
    );
    assert_eq!(history(cx, handle), ["Create table", "Edit text"]);

    with_window(cx, handle, |window, cx| key(window, "ctrl-z", true, cx));
    assert_eq!(table_of(cx, handle, id).rows[0][0].content, "");
}

#[gpui_kit::test]
fn a_click_with_the_table_tool_makes_three_by_three(cx: &mut TestAppContext) {
    let handle = open(cx);
    let position = at(cx, handle, 300., 300.);
    with_window(cx, handle, |window, cx| {
        window.click("tool-table", cx);
        click_at(window, position, 1, cx);
    });
    let id = read(cx, handle, |editor| editor.current_slide().elements[0].id);
    let table = table_of(cx, handle, id);
    assert_eq!((table.row_count(), table.column_count()), (3, 3));
}

#[gpui_kit::test]
fn tab_moves_to_the_next_cell_and_stops_at_the_last(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"], &["c", "d"]]);
    click_cell(cx, handle, id, 0, 0, 2);
    with_window(cx, handle, |window, cx| {
        key(window, "tab", true, cx);
        window.input("B", cx);
        key(window, "tab", true, cx);
        key(window, "tab", true, cx);
        key(window, "tab", true, cx);
        window.input("D", cx);
    });
    let table = table_of(cx, handle, id);
    assert_eq!(contents(&table), [["a", "B"], ["c", "D"]]);
}

#[gpui_kit::test]
fn typing_over_a_selected_cell_replaces_it_and_delete_clears_cells(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["one", "two"], &["three", "four"]]);
    // A double click edits; Escape leaves the text with the cell selected.
    click_cell(cx, handle, id, 0, 1, 2);
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    let selected = read(cx, handle, |editor| {
        (
            editor.text_edit.is_none(),
            editor.table_edit.clone().map(|edit| edit.focus),
        )
    });
    assert_eq!(selected, (true, Some((0, 1))));
    with_window(cx, handle, |window, cx| window.input("2", cx));
    assert_eq!(table_of(cx, handle, id).rows[0][1].content, "2");

    with_window(cx, handle, |window, cx| {
        key(window, "escape", true, cx);
        key(window, "down", true, cx);
        key(window, "shift-left", true, cx);
        key(window, "delete", true, cx);
    });
    let table = table_of(cx, handle, id);
    assert_eq!(contents(&table), [["one", "2"], ["", ""]]);
    assert_eq!(history(cx, handle).last().unwrap(), "Clear cells");
}

#[gpui_kit::test]
fn shift_click_selects_a_range_to_merge_and_split(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b", "c"], &["d", "e", "f"]]);
    click_cell(cx, handle, id, 0, 0, 2);
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    let corner = cell_center(cx, handle, id, 1, 1);
    with_window(cx, handle, |window, cx| {
        click_with(window, corner, 1, Modifiers::shift(), cx)
    });
    assert!(with_editor(cx, handle, |editor, _| editor.merge_cells()));
    let table = table_of(cx, handle, id);
    assert_eq!(table.rows[0][0].content, "a\nb\nd\ne");
    assert_eq!(table.anchors().len(), 3);

    assert!(with_editor(cx, handle, |editor, _| editor.split_cells()));
    assert_eq!(table_of(cx, handle, id).anchors().len(), 6);
}

#[gpui_kit::test]
fn row_and_column_commands_act_on_the_selected_cells(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"], &["c", "d"]]);
    click_cell(cx, handle, id, 0, 1, 2);
    with_editor(cx, handle, |editor, _| {
        editor.insert_rows_near(true);
        editor.insert_columns_near(false);
    });
    let table = table_of(cx, handle, id);
    assert_eq!(
        contents(&table),
        [["a", "", "b"], ["", "", ""], ["c", "", "d"]]
    );
    let focus = read(cx, handle, |editor| {
        editor.table_edit.clone().unwrap().focus
    });
    assert_eq!(focus, (1, 1), "the new column of the new row");

    with_editor(cx, handle, |editor, _| {
        editor.delete_rows();
        editor.delete_columns();
    });
    assert_eq!(
        contents(&table_of(cx, handle, id)),
        [["a", "b"], ["c", "d"]]
    );
    assert_eq!(
        history(cx, handle),
        [
            "Insert row",
            "Insert column",
            "Delete rows",
            "Delete columns"
        ]
    );
}

#[gpui_kit::test]
fn the_plus_button_above_a_line_inserts_a_column_there(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"]]);
    click_cell(cx, handle, id, 0, 0, 1);
    let (x, y) = with_editor(cx, handle, |editor, _| {
        let frame = editor.presentation.element(id).unwrap().frame;
        let view = editor.tables.get(&editor.presentation, id).unwrap();
        let zoom = editor.camera.unwrap().zoom;
        let x = view.layout.column_edges()[1];
        frame.to_slide(x, -crate::ui::table_edit::INSERT_OFFSET / zoom)
    });
    let position = at(cx, handle, x, y);
    with_window(cx, handle, |window, cx| {
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Modifiers::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    });
    assert_eq!(
        read(cx, handle, |editor| editor.table_insert),
        Some((false, 1))
    );
    with_window(cx, handle, |window, cx| click_at(window, position, 1, cx));
    assert_eq!(contents(&table_of(cx, handle, id)), [["a", "", "b"]]);
}

#[gpui_kit::test]
fn dragging_a_selected_row_moves_it(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["1", "x"], &["2", "y"], &["3", "z"]]);
    click_cell(cx, handle, id, 0, 0, 2);
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    let end = cell_center(cx, handle, id, 0, 1);
    with_window(cx, handle, |window, cx| {
        click_with(window, end, 1, Modifiers::shift(), cx)
    });
    let from = cell_center(cx, handle, id, 0, 0);
    let (x, y) = with_editor(cx, handle, |editor, _| {
        let frame = editor.presentation.element(id).unwrap().frame;
        let view = editor.tables.get(&editor.presentation, id).unwrap();
        frame.to_slide(10., view.layout.height() - 2.)
    });
    let to = at(cx, handle, x, y);
    with_window(cx, handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        )
    });
    let table = table_of(cx, handle, id);
    assert_eq!(contents(&table), [["2", "y"], ["3", "z"], ["1", "x"]]);
    assert_eq!(history(cx, handle).last().unwrap(), "Move rows");
}

#[gpui_kit::test]
fn a_side_handle_fixes_only_the_width(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"]]);
    click_cell(cx, handle, id, 0, 0, 1);
    let frame = frame_of(cx, handle, id);
    let from = at(
        cx,
        handle,
        frame.x + frame.width,
        frame.y + frame.height / 2.,
    );
    let to = at(
        cx,
        handle,
        frame.x + frame.width + 200.,
        frame.y + frame.height / 2.,
    );
    with_window(cx, handle, |window, cx| {
        drag_with(
            window,
            MouseButton::Left,
            from,
            to,
            Modifiers::default(),
            cx,
        )
    });
    let table = table_of(cx, handle, id);
    assert!(
        matches!(table.width, TableSizing::Fixed(width) if (width - frame.width - 200.).abs() < 1.)
    );
    assert_eq!(table.height, TableSizing::Auto);
    let wider = frame_of(cx, handle, id);
    assert!((wider.width - frame.width - 200.).abs() < 1.);
    assert_eq!(wider.height, frame.height);
}

#[gpui_kit::test]
fn the_panel_fills_the_selected_cells_and_resets_them(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"], &["c", "d"]]);
    click_cell(cx, handle, id, 0, 1, 2);
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::Solid)
    });
    let table = table_of(cx, handle, id);
    assert!(matches!(
        table.rows[0][1].fill,
        Some(Fill::Solid(SolidFill { .. }))
    ));
    assert_eq!(table.rows[0][0].fill, None);
    assert_eq!(table.fill, Fill::None, "the table fill stays");

    with_editor(cx, handle, |editor, _| editor.reset_cells());
    assert_eq!(table_of(cx, handle, id).rows[0][1].fill, None);

    // Without selected cells, the panel edits the whole table.
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::Solid)
    });
    assert!(matches!(table_of(cx, handle, id).fill, Fill::Solid(_)));
}

#[gpui_kit::test]
fn the_panel_strokes_the_chosen_sides_of_the_selected_cells(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"], &["c", "d"]]);
    click_cell(cx, handle, id, 0, 0, 2);
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    with_editor(cx, handle, |editor, _| {
        editor.border_sides = Sides::Bottom;
        editor.set_stroke(false);
    });
    let table = table_of(cx, handle, id);
    assert_eq!(table.horizontal[1][0], Edge::None);
    assert_eq!(table.horizontal[0][0], Edge::Inherit);
    assert_eq!(table.vertical[0][1], Edge::Inherit);
}

#[gpui_kit::test]
fn the_text_panel_styles_the_selected_cells(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"]]);
    click_cell(cx, handle, id, 0, 1, 2);
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    with_editor(cx, handle, |editor, _| {
        editor.set_style(crate::document::TextStylePatch {
            size: Some(40.),
            ..Default::default()
        })
    });
    let table = table_of(cx, handle, id);
    assert_eq!(table.style_of(0, 1).size, 40.);
    assert_eq!(table.style_of(0, 0).size, 24.);
}

#[gpui_kit::test]
fn copied_cells_paste_with_their_style_and_tsv_pastes_a_new_table(cx: &mut TestAppContext) {
    let (handle, id) = with_table(cx, &[&["a", "b"], &["c", "d"]]);
    click_cell(cx, handle, id, 0, 0, 2);
    with_window(cx, handle, |window, cx| key(window, "escape", true, cx));
    with_editor(cx, handle, |editor, _| {
        editor.set_fill_type(FillType::Solid)
    });
    with_window(cx, handle, |window, cx| {
        key(window, "secondary-c", true, cx);
        key(window, "down", true, cx);
        key(window, "right", true, cx);
        key(window, "secondary-v", true, cx);
    });
    let table = table_of(cx, handle, id);
    assert_eq!(table.rows[1][1].content, "a");
    assert!(matches!(table.rows[1][1].fill, Some(Fill::Solid(_))));

    with_window(cx, handle, |window, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("x\ty\n1\t2".into()));
        key(window, "escape", true, cx);
        key(window, "escape", true, cx);
        key(window, "secondary-v", true, cx);
    });
    let tables = read(cx, handle, |editor| editor.current_slide().elements.len());
    assert_eq!(tables, 2);
    let pasted = read(cx, handle, |editor| editor.current_slide().elements[1].id);
    assert_eq!(
        contents(&table_of(cx, handle, pasted)),
        [["x", "y"], ["1", "2"]]
    );
}
