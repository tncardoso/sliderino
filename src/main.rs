use sliderino::app;
use sliderino::document::Presentation;
use sliderino::history::History;

fn main() {
    app::application().run(|cx| {
        app::init(cx);
        app::open_editor(cx, Presentation::new(), History::default(), |_, _, _| {});
    });
}
