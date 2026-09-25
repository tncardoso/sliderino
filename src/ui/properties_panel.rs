//! Right panel: design properties, diagnostics and component actions of the
//! current selection.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement, Styled, div, px,
};

use crate::editor::EditorView;
use crate::mock::{DIAGNOSTICS, SELECTION, Severity};
use crate::theme;
use crate::ui::widgets::{field, field_icon, field_letter, section, section_label};

pub fn properties_panel(editor: &EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    v_flex()
        .w(px(264.))
        .flex_shrink_0()
        .h_full()
        .bg(theme::background())
        .border_l_1()
        .border_color(theme::border())
        .child(
            TabBar::new("inspector-tabs")
                .underline()
                .xsmall()
                .h(px(40.))
                .px(px(8.))
                .selected_index(editor.inspector_tab)
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    this.inspector_tab = *ix;
                    cx.notify();
                }))
                .child(Tab::new().label("Design"))
                .child(Tab::new().label("Notes"))
                .child(Tab::new().label("History")),
        )
        .child(
            v_flex()
                .id("inspector")
                .flex_1()
                .overflow_y_scrollbar()
                .child(selection_header())
                .child(position_section())
                .child(fill_section())
                .child(collapsed_section("STROKE"))
                .child(collapsed_section("SHADOW"))
                .child(diagnostics_section())
                .child(component_section()),
        )
}

fn selection_header() -> impl IntoElement {
    v_flex()
        .gap(px(4.))
        .px(px(16.))
        .py(px(14.))
        .border_b_1()
        .border_color(theme::border())
        .child(
            h_flex()
                .gap(px(8.))
                .child(
                    Icon::new(IconName::Component)
                        .size(px(14.))
                        .text_color(theme::accent()),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SELECTION.name),
                ),
        )
        .child(
            div()
                .pl(px(22.))
                .text_size(px(11.))
                .line_height(px(14.))
                .text_color(theme::text_muted())
                .child(SELECTION.origin),
        )
}

fn position_section() -> impl IntoElement {
    let align = [
        IconName::AlignStartVertical,
        IconName::AlignCenterVertical,
        IconName::AlignEndVertical,
        IconName::AlignStartHorizontal,
        IconName::AlignCenterHorizontal,
        IconName::AlignEndHorizontal,
    ];

    section()
        .child(section_label("POSITION"))
        .child(
            h_flex()
                .h(px(28.))
                .px(px(6.))
                .justify_between()
                .rounded(px(6.))
                .bg(theme::field())
                .children(
                    align
                        .into_iter()
                        .map(|icon| Icon::new(icon).size(px(16.)).text_color(theme::ink())),
                ),
        )
        .child(field_row(
            field(field_letter("X"), SELECTION.x),
            field(field_letter("Y"), SELECTION.y),
        ))
        .child(field_row(
            field(field_letter("W"), SELECTION.width),
            field(field_letter("H"), SELECTION.height),
        ))
        .child(field_row(
            field(field_icon(IconName::RotateCw), SELECTION.rotation),
            field(field_icon(IconName::Radius), SELECTION.radius),
        ))
}

fn field_row(left: impl IntoElement, right: impl IntoElement) -> impl IntoElement {
    h_flex().gap(px(8.)).child(left).child(right)
}

fn section_header(label: &'static str) -> impl IntoElement {
    h_flex()
        .justify_between()
        .child(section_label(label))
        .child(
            Icon::new(IconName::Plus)
                .size(px(14.))
                .text_color(theme::ink()),
        )
}

fn fill_section() -> impl IntoElement {
    let swatch = div()
        .size(px(14.))
        .flex_shrink_0()
        .rounded(px(3.))
        .border_1()
        .border_color(gpui_kit::rgb(0xD6D6D6))
        .bg(theme::surface());

    section().child(section_header("FILL")).child(
        field(swatch, SELECTION.fill).child(
            div()
                .ml_auto()
                .text_color(theme::text_muted())
                .child(SELECTION.fill_opacity),
        ),
    )
}

/// An empty property group that only offers to add an entry.
fn collapsed_section(label: &'static str) -> impl IntoElement {
    div()
        .px(px(16.))
        .py(px(14.))
        .border_b_1()
        .border_color(theme::border())
        .child(section_header(label))
}

fn diagnostics_section() -> impl IntoElement {
    section()
        .child(section_label("DIAGNOSTICS"))
        .children(DIAGNOSTICS.iter().map(|diagnostic| {
            let (icon, color, ground) = match diagnostic.severity {
                Severity::Ok => (IconName::CircleCheck, theme::ok(), theme::ok_soft()),
                Severity::Warning => (IconName::CircleAlert, theme::warn(), theme::warn_soft()),
            };
            h_flex()
                .gap(px(8.))
                .child(status_badge(icon, color, ground))
                .child(div().child(diagnostic.message))
        }))
}

fn status_badge(icon: IconName, color: Hsla, ground: Hsla) -> impl IntoElement {
    div()
        .size(px(14.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(ground)
        .child(Icon::new(icon).size(px(12.)).text_color(color))
}

fn component_section() -> impl IntoElement {
    section()
        .gap(px(8.))
        .border_b_0()
        .child(section_label("COMPONENT"))
        .child(
            h_flex()
                .gap(px(8.))
                .child(
                    Button::new("save-to-library")
                        .outline()
                        .xsmall()
                        .label("Save to library")
                        .h(px(28.))
                        .flex_1(),
                )
                .child(
                    Button::new("go-to-source")
                        .outline()
                        .xsmall()
                        .label("Go to source")
                        .h(px(28.))
                        .flex_1(),
                ),
        )
}
