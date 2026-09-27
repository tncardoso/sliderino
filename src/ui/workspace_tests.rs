//! The Home screen, the switch to the editor and back, and the question
//! about unsaved changes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
use serde_json::{Value, json};

use crate::api::protocol::{ApiError, ToolOutput};
use crate::api::server::Event;
use crate::document::{Operation, Presentation, Slide};
use crate::history::History;
use crate::ui::workspace::{Screen, Workspace};

type Handle = (WindowHandle<Root>, Entity<Workspace>);

fn open(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::Context<Workspace>) -> Workspace + 'static,
) -> Handle {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(cx);
        cx.text_system()
            .add_fonts(crate::assets::fonts())
            .expect("embedded Inter fonts load");
    });
    let slot = Rc::new(RefCell::new(None));
    let stored = slot.clone();
    let handle = cx.open_window(size(px(1440.), px(900.)), move |window, cx| {
        let workspace = cx.new(|cx| build(window, cx));
        *stored.borrow_mut() = Some(workspace.clone());
        let focus = workspace.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Root::new(workspace, window, cx)
    });
    let workspace = slot.borrow_mut().take().unwrap();
    render(cx, handle);
    (handle, workspace)
}

fn open_home(cx: &mut TestAppContext) -> Handle {
    open(cx, Workspace::home)
}

fn open_editor(cx: &mut TestAppContext, file: Option<PathBuf>) -> Handle {
    open(cx, move |window, cx| {
        Workspace::editor(Presentation::new(), History::default(), file, window, cx)
    })
}

fn render(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn with_window<R>(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    let result = cx
        .update_window(handle.into(), |_, window, cx| f(window, cx))
        .unwrap();
    render(cx, handle);
    result
}

fn on_home(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> bool {
    workspace.read_with(cx, |workspace, _| {
        matches!(workspace.screen, Screen::Home(_))
    })
}

/// Makes a change to the presentation in the editor.
fn edit(cx: &mut TestAppContext, (handle, workspace): &Handle) {
    let editor = workspace.read_with(cx, |workspace, _| workspace.editor_view().cloned().unwrap());
    editor.update(cx, |editor, cx| {
        let id = editor.presentation.new_slide_id();
        editor
            .presentation
            .apply(Operation::AddSlide {
                index: usize::MAX,
                slide: Slide::new(id),
            })
            .unwrap();
        cx.notify();
    });
    render(cx, *handle);
}

fn title(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
    workspace.read_with(cx, |workspace, cx| {
        workspace.editor_view().unwrap().read(cx).title()
    })
}

fn temp_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sliderino-workspace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn call(
    cx: &mut TestAppContext,
    (handle, workspace): &Handle,
    tool: &str,
    args: Value,
) -> Result<Value, ApiError> {
    let (reply, result) = mpsc::channel::<Result<ToolOutput, ApiError>>();
    let event = Event::Call {
        tool: tool.into(),
        args,
        reply,
    };
    let workspace = workspace.clone();
    with_window(cx, *handle, |window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.on_api_event(event, window, cx)
        });
    });
    result.recv().unwrap().map(|output| output.value)
}

#[gpui_kit::test]
fn sliderino_starts_on_home_and_ctrl_n_shows_a_new_presentation(cx: &mut TestAppContext) {
    let (handle, workspace) = open_home(cx);
    assert!(on_home(cx, &workspace));
    with_window(cx, handle, |window, _| {
        assert!(window.try_find("home-screen").is_some());
    });
    with_window(cx, handle, |window, cx| window.press("ctrl-n", cx));
    assert!(!on_home(cx, &workspace));
    assert_eq!(title(cx, &workspace), "Untitled");
    with_window(cx, handle, |window, _| {
        assert!(window.try_find("unsaved").is_none());
    });
}

#[gpui_kit::test]
fn home_in_the_editor_goes_home_at_once_without_changes(cx: &mut TestAppContext) {
    let (handle, workspace) = open_editor(cx, None);
    with_window(cx, handle, |window, cx| window.click("home", cx));
    assert!(on_home(cx, &workspace));
}

#[gpui_kit::test]
fn home_asks_about_unsaved_changes(cx: &mut TestAppContext) {
    let opened = open_editor(cx, None);
    let (handle, workspace) = opened.clone();
    edit(cx, &opened);
    with_window(cx, handle, |window, _| {
        assert!(window.try_find("unsaved").is_some(), "the dot shows");
    });

    with_window(cx, handle, |window, cx| window.click("home", cx));
    with_window(cx, handle, |window, _| {
        assert!(window.try_find("unsaved-message").is_some());
    });
    with_window(cx, handle, |window, cx| window.click("unsaved-cancel", cx));
    assert!(!on_home(cx, &workspace));
    with_window(cx, handle, |window, _| {
        assert!(window.try_find("unsaved-message").is_none());
    });

    with_window(cx, handle, |window, cx| window.click("home", cx));
    with_window(cx, handle, |window, cx| window.click("unsaved-discard", cx));
    assert!(on_home(cx, &workspace));
}

#[gpui_kit::test]
fn save_in_the_question_saves_then_goes_home(cx: &mut TestAppContext) {
    let path = temp_file("save-then-home.sldr");
    let opened = open_editor(cx, Some(path.clone()));
    let (handle, workspace) = opened.clone();
    edit(cx, &opened);
    with_window(cx, handle, |window, cx| window.click("home", cx));
    with_window(cx, handle, |window, cx| window.click("unsaved-save", cx));
    render(cx, handle);
    assert!(on_home(cx, &workspace));
    assert_eq!(crate::file::load(&path).unwrap().slides.len(), 2);
}

#[gpui_kit::test]
fn ctrl_s_saves_to_the_file_and_clears_the_dot(cx: &mut TestAppContext) {
    let path = temp_file("ctrl-s.sldr");
    let opened = open_editor(cx, Some(path.clone()));
    let (handle, workspace) = opened.clone();
    edit(cx, &opened);
    with_window(cx, handle, |window, cx| window.press("ctrl-s", cx));
    render(cx, handle);
    with_window(cx, handle, |window, _| {
        assert!(window.try_find("unsaved").is_none());
    });
    assert_eq!(title(cx, &workspace), "ctrl-s");
    assert_eq!(crate::file::load(&path).unwrap().slides.len(), 2);
}

#[gpui_kit::test]
fn a_dirty_window_asks_before_it_closes(cx: &mut TestAppContext) {
    let opened = open_editor(cx, None);
    let (handle, workspace) = opened.clone();
    let should_close = |cx: &mut TestAppContext| {
        let workspace = workspace.clone();
        with_window(cx, handle, |window, cx| {
            workspace.update(cx, |workspace, cx| workspace.should_close(window, cx))
        })
    };
    assert!(should_close(cx));
    edit(cx, &opened);
    assert!(!should_close(cx));
    with_window(cx, handle, |window, _| {
        assert!(window.try_find("unsaved-message").is_some());
    });
}

#[gpui_kit::test]
fn agents_get_no_presentation_on_home(cx: &mut TestAppContext) {
    let opened = open_home(cx);
    let error = call(cx, &opened, "get_basic_info", json!({})).unwrap_err();
    assert_eq!(error.code, "no_presentation");

    let created = call(cx, &opened, "new_presentation", json!({})).unwrap();
    assert_eq!(created["file"], json!(null));
    assert!(!on_home(cx, &opened.1));
    let info = call(cx, &opened, "get_basic_info", json!({})).unwrap();
    assert_eq!(info["unsaved"], json!(false));
}

#[gpui_kit::test]
fn agents_do_not_drop_unsaved_changes_unless_they_discard(cx: &mut TestAppContext) {
    let path = temp_file("agent-open.sldr");
    crate::file::save(&Presentation::new(), &path).unwrap();
    let opened = open_editor(cx, None);
    edit(cx, &opened);

    let error = call(cx, &opened, "open_presentation", json!({"path": path})).unwrap_err();
    assert_eq!(error.code, "unsaved_changes");
    let error = call(cx, &opened, "new_presentation", json!({})).unwrap_err();
    assert_eq!(error.code, "unsaved_changes");

    let result = call(
        cx,
        &opened,
        "open_presentation",
        json!({"path": path, "discard": true}),
    )
    .unwrap();
    assert_eq!(result["file"], json!(path));
    let info = call(cx, &opened, "get_basic_info", json!({})).unwrap();
    assert_eq!(
        (info["file"].clone(), info["unsaved"].clone()),
        (json!(path), json!(false))
    );
}

#[gpui_kit::test]
fn a_file_that_cannot_be_read_leaves_the_screen(cx: &mut TestAppContext) {
    let path = temp_file("broken.sldr");
    std::fs::write(&path, "not a zip").unwrap();
    let (handle, workspace) = open_home(cx);
    let target = path.clone();
    with_window(cx, handle, |window, cx| {
        workspace.update(cx, |workspace, cx| workspace.open_path(target, window, cx));
    });
    render(cx, handle);
    assert!(on_home(cx, &workspace));
}
