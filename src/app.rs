//! Starting the GPUI application and opening Sliderino windows. Shared by the
//! editor binary and `sliderino-debug scene`.

use std::path::PathBuf;

use gpui_kit::component::{Root, TitleBar};
use gpui_kit::{
    App, AppContext as _, Application, Bounds, Context, Entity, Window, WindowBounds,
    WindowOptions, px, size,
};

use crate::api::server;
use crate::assets::{self, AppAssets};
use crate::document::Presentation;
use crate::editor::EditorView;
use crate::history::History;
use crate::theme;
use crate::ui::workspace::Workspace;

/// The GPUI application with Sliderino's embedded assets.
pub fn application() -> Application {
    gpui_kit::application().with_assets(AppAssets)
}

/// Initializes the kit, the embedded fonts and the theme. Call first thing
/// in `Application::run`.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    crate::editor::bind_keys(cx);
    cx.text_system()
        .add_fonts(assets::fonts())
        .expect("embedded Inter fonts load");
    theme::apply(cx);
}

/// What a new window shows first.
pub enum Start {
    Home,
    /// The editor on a new presentation.
    New,
    /// The editor on a `.sldr` file; the Home screen when it cannot be read.
    File(PathBuf),
}

/// Opens a Sliderino window. With `api`, the window serves the agent API.
pub fn open_window(cx: &mut App, start: Start, api: bool) {
    open(cx, api, move |window, cx| match start {
        Start::Home => Workspace::home(window, cx),
        Start::New => Workspace::editor(Presentation::new(), History::default(), None, window, cx),
        Start::File(path) => {
            let mut workspace = Workspace::home(window, cx);
            workspace.open_path(path, window, cx);
            workspace
        }
    });
}

/// Opens a window with the editor on `presentation` and its undo
/// `history`, without the agent API, then calls `opened` with the editor,
/// e.g. to select an element.
pub fn open_editor(
    cx: &mut App,
    presentation: Presentation,
    history: History,
    opened: impl FnOnce(&mut EditorView, &mut Window, &mut Context<EditorView>) + 'static,
) {
    open(cx, false, move |window, cx| {
        let workspace = Workspace::editor(presentation, history, None, window, cx);
        if let Some(editor) = workspace.editor_view() {
            editor.update(cx, |editor, cx| opened(editor, window, cx));
        }
        workspace
    });
}

fn open(
    cx: &mut App,
    api: bool,
    build: impl FnOnce(&mut Window, &mut Context<Workspace>) -> Workspace + 'static,
) {
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(960.), px(600.))),
        app_id: Some("sliderino".into()),
        ..TitleBar::window_options()
    };
    cx.spawn(async move |cx| {
        cx.open_window(options, |window, cx| {
            let workspace: Entity<Workspace> = cx.new(|cx| {
                let workspace = build(window, cx);
                if api && let Err(error) = server::start(window, cx) {
                    eprintln!("sliderino: the agent API is off: {error}");
                }
                workspace
            });
            let focus = workspace.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            let weak = workspace.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                weak.update(cx, |workspace, cx| workspace.should_close(window, cx))
                    .unwrap_or(true)
            });
            cx.new(|cx| Root::new(workspace, window, cx))
        })
        .expect("failed to open the Sliderino window");
    })
    .detach();
}
