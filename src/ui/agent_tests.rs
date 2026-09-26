//! Tests of agent edits in a test window: they share the person's undo
//! history and keep the person's drag, text edit and view consistent.

use gpui_kit::{TestAppContext, WindowHandle};
use serde_json::{Value, json};

use crate::api::protocol::ApiError;
use crate::api::tools;
use crate::document::{ElementId, Frame, SlideId, TextSizing};
use crate::editor::{Drag, EditorView};
use crate::ui::test_support::{open, read, with_editor};

fn call(
    cx: &mut TestAppContext,
    handle: WindowHandle<EditorView>,
    tool: &str,
    args: Value,
) -> Result<Value, ApiError> {
    with_editor(cx, handle, |editor, _| {
        tools::handle(editor, tool, args).map(|output| output.value)
    })
}

/// Creates a text box as the person does and leaves its editing.
fn create_text(cx: &mut TestAppContext, handle: WindowHandle<EditorView>, text: &str) -> ElementId {
    with_editor(cx, handle, |editor, _| {
        editor.create_text(Frame::default(), TextSizing::AutoWidth);
        editor.type_text(0..0, text);
        let id = editor.selection.unwrap();
        editor.end_text_edit();
        id
    })
}

fn history(cx: &mut TestAppContext, handle: WindowHandle<EditorView>) -> Vec<String> {
    read(cx, handle, |editor| {
        editor.history.done().map(String::from).collect()
    })
}

#[gpui_kit::test]
fn an_agent_edit_is_one_step_of_the_shared_history(cx: &mut TestAppContext) {
    let handle = open(cx);
    create_text(cx, handle, "Mine");
    call(
        cx,
        handle,
        "apply_operations",
        json!({"label": "Agent title", "ops": [
            {"op": "add_element", "slide": 1, "element": {"id": "$t", "text": {"content": "Theirs"}}},
            {"op": "set_text_style", "id": "$t", "patch": {"size": 60}}
        ]}),
    )
    .unwrap();
    assert_eq!(
        history(cx, handle),
        ["Create text", "Edit text", "Agent title"]
    );
    // The person's undo reverts the agent's step first.
    with_editor(cx, handle, |editor, _| editor.undo());
    let count = read(cx, handle, |editor| editor.current_slide().elements.len());
    assert_eq!(count, 1);
}

#[gpui_kit::test]
fn an_agent_edit_cancels_a_drag_of_the_element_it_changes(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_text(cx, handle, "Drag me");
    let start = |editor: &mut EditorView| {
        let origin = editor.presentation.element(id).unwrap().frame;
        editor.drag = Some(Drag::Move {
            id,
            grab: Default::default(),
            origin,
            current: Frame { x: 50., ..origin },
            moved: true,
        });
    };
    with_editor(cx, handle, |editor, _| start(editor));
    // A change elsewhere leaves the drag alone.
    call(
        cx,
        handle,
        "apply_operations",
        json!({"ops": [{"op": "add_slide"}]}),
    )
    .unwrap();
    assert!(read(cx, handle, |editor| editor.drag.is_some()));
    call(
        cx,
        handle,
        "apply_operations",
        json!({"ops": [{"op": "set_frame", "id": id, "frame": {"x": 300, "y": 300}}]}),
    )
    .unwrap();
    assert!(read(cx, handle, |editor| editor.drag.is_none()));
}

#[gpui_kit::test]
fn an_agent_edit_closes_the_typing_burst(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_text(cx, handle, "Hi");
    with_editor(cx, handle, |editor, _| {
        editor.begin_text_edit(id, 2, 2);
        editor.type_text(2..2, "!");
    });
    call(
        cx,
        handle,
        "apply_operations",
        json!({"ops": [{"op": "set_text_style", "id": id, "patch": {"underline": true}}]}),
    )
    .unwrap();
    with_editor(cx, handle, |editor, _| editor.type_text(3..3, "?"));
    assert_eq!(
        history(cx, handle),
        [
            "Create text",
            "Edit text",
            "Edit text",
            "Underline",
            "Edit text"
        ]
    );
}

#[gpui_kit::test]
fn deleting_the_edited_text_ends_its_editing(cx: &mut TestAppContext) {
    let handle = open(cx);
    let id = create_text(cx, handle, "Going away");
    with_editor(cx, handle, |editor, _| editor.begin_text_edit(id, 0, 5));
    call(
        cx,
        handle,
        "apply_operations",
        json!({"ops": [{"op": "remove_element", "id": id}]}),
    )
    .unwrap();
    read(cx, handle, |editor| {
        assert!(editor.text_edit.is_none());
        assert!(editor.selection.is_none());
    });
}

#[gpui_kit::test]
fn the_view_follows_agent_edits_only_when_asked(cx: &mut TestAppContext) {
    let handle = open(cx);
    let add = json!({"ops": [
        {"op": "add_slide", "slide": {"id": "$s"}},
        {"op": "add_element", "slide": "$s", "element": {"id": "$t", "text": {"content": "New"}}}
    ]});
    call(cx, handle, "apply_operations", add.clone()).unwrap();
    assert_eq!(read(cx, handle, |editor| editor.current_slide), SlideId(1));

    with_editor(cx, handle, |editor, _| editor.agents.follow = true);
    let refs = call(cx, handle, "apply_operations", add).unwrap()["refs"].clone();
    read(cx, handle, |editor| {
        assert_eq!(editor.current_slide.0, refs["$s"]);
        assert_eq!(editor.selection.map(|id| id.0), refs["$t"].as_u64());
    });
}
