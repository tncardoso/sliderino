mod assets;
mod camera;
mod document;
mod editor;
mod mock;
mod shortcuts;
mod theme;
mod ui;

use gpui_kit::component::{Root, TitleBar};
use gpui_kit::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};

use crate::assets::AppAssets;
use crate::editor::EditorView;

fn main() {
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            cx.text_system()
                .add_fonts(assets::fonts())
                .expect("embedded Inter fonts load");
            theme::apply(cx);

            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(960.), px(600.))),
                app_id: Some("sliderino".into()),
                ..TitleBar::window_options()
            };

            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    let view = cx.new(|cx| EditorView::new(window, cx));
                    window.focus(&view.read(cx).focus.clone(), cx);
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("failed to open the editor window");
            })
            .detach();
        });
}
