//! Starting the GPUI application and opening editor windows. Shared by the
//! editor binary and `sliderino-debug scene`.

use gpui_kit::component::{Root, TitleBar};
use gpui_kit::{
    App, AppContext as _, Application, Bounds, Context, Entity, Window, WindowBounds,
    WindowOptions, px, size,
};

use crate::assets::{self, AppAssets};
use crate::document::Presentation;
use crate::editor::EditorView;
use crate::history::History;
use crate::theme;

/// The GPUI application with Sliderino's embedded assets.
pub fn application() -> Application {
    gpui_kit::application().with_assets(AppAssets)
}

/// Initializes the kit, the embedded fonts and the theme. Call first thing
/// in `Application::run`.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    cx.text_system()
        .add_fonts(assets::fonts())
        .expect("embedded Inter fonts load");
    theme::apply(cx);
}

/// Opens an editor window on `presentation` with its undo `history`, then
/// calls `opened` with the editor, e.g. to select an element.
pub fn open_editor(
    cx: &mut App,
    presentation: Presentation,
    history: History,
    opened: impl FnOnce(&mut EditorView, &mut Window, &mut Context<EditorView>) + 'static,
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
            let view: Entity<EditorView> = cx.new(|cx| {
                let mut editor = EditorView::with_document(presentation, history, window, cx);
                opened(&mut editor, window, cx);
                editor
            });
            window.focus(&view.read(cx).focus.clone(), cx);
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open the editor window");
    })
    .detach();
}
