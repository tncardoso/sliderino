//! Left panel: slide thumbnails, later the component library, and the
//! hierarchy of the current slide.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    ContentMask, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, TestSupportExt as _, canvas as paint_canvas, div, px,
};

use crate::editor::EditorView;
use crate::theme;
use crate::ui::canvas::{PaintText, paint_texts};
use crate::ui::hierarchy_panel::hierarchy_panel;

const THUMB_WIDTH: f32 = 184.;

pub fn slides_panel(editor: &mut EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    let _span = crate::perf::span("slides_panel");
    let hierarchy = editor.library_tab == 2;
    let slides: Vec<_> = editor
        .presentation
        .slides
        .iter()
        .map(|slide| slide.id)
        .collect();
    let thumbnails: Vec<_> = slides
        .into_iter()
        .filter(|_| !hierarchy)
        .enumerate()
        .map(|(ix, id)| {
            let texts = editor.paint_texts(id, false, cx);
            let active = id == editor.current_slide;
            h_flex()
                .id(("slide", ix))
                .test_support()
                .items_start()
                .gap(px(8.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_slide(id);
                    cx.notify();
                }))
                .child(slide_number(ix + 1, active))
                .child(thumbnail(editor, texts, active))
        })
        .collect();

    v_flex()
        .w(px(232.))
        .flex_shrink_0()
        .h_full()
        .bg(theme::background())
        .border_r_1()
        .border_color(theme::border())
        .child(
            TabBar::new("library-tabs")
                .underline()
                .xsmall()
                .h(px(40.))
                .pl(px(8.))
                .pr(px(8.))
                .selected_index(editor.library_tab)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.library_tab = *ix;
                    cx.notify();
                }))
                .child(Tab::new().label("Slides"))
                .child(Tab::new().label("Components"))
                .child(Tab::new().label("Hierarchy"))
                .when(!hierarchy, |this| {
                    this.suffix(
                        Button::new("add-slide")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.add_slide();
                                cx.notify();
                            })),
                    )
                }),
        )
        .map(|this| {
            if hierarchy {
                this.child(hierarchy_panel(editor, cx).into_any_element())
            } else {
                this.child(
                    v_flex()
                        .id("thumbnails")
                        .flex_1()
                        .gap(px(14.))
                        .pt(px(12.))
                        .pb(px(16.))
                        .pl(px(12.))
                        .pr(px(16.))
                        .overflow_y_scrollbar()
                        .children(thumbnails)
                        .into_any_element(),
                )
            }
        })
}

fn slide_number(number: usize, active: bool) -> impl IntoElement {
    div()
        .w(px(16.))
        .flex_shrink_0()
        .pt(px(2.))
        .flex()
        .justify_end()
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(if active {
            theme::accent()
        } else {
            theme::text_muted()
        })
        .when(active, |this| this.font_weight(FontWeight::SEMIBOLD))
        .child(number.to_string())
}

/// The slide drawn small, clipped to its edges; the active slide gets a 2px
/// accent ring.
fn thumbnail(editor: &EditorView, texts: Vec<PaintText>, active: bool) -> Div {
    let size = editor.presentation.size;
    let zoom = THUMB_WIDTH / size.width as f32;
    let height = size.height as f32 * zoom;
    let frame = div()
        .relative()
        .w(px(THUMB_WIDTH))
        .h(px(height))
        .flex_shrink_0()
        .rounded(px(4.))
        .overflow_hidden()
        .bg(theme::background())
        .child(
            paint_canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        paint_texts(&texts, bounds.origin, zoom, true, window);
                    });
                },
            )
            .absolute()
            .size_full(),
        );
    if active {
        frame.border_2().border_color(theme::accent())
    } else {
        frame.border_1().border_color(theme::border())
    }
}
