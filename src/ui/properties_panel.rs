//! Right panel: design properties and diagnostics of the selection (or of
//! the slide when nothing is selected), speaker notes and the undo history.

use gpui_kit::assets::IconName;
use gpui_kit::component::color_picker::ColorPicker;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::Select;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement, Stateful, StatefulInteractiveElement as _, Styled, TestSupportExt as _, div, px,
};

use crate::document::{Element, HAlign, TextCase, TextSizing, TextStyle, TextStylePatch, VAlign};
use crate::editor::EditorView;
use crate::text_layout::TextLayout;
use crate::theme;
use crate::ui::inspector::{Field, number};
use crate::ui::widgets::{
    field, field_icon, field_letter, input_field, section, section_label, segment, segmented,
};

pub fn properties_panel(editor: &EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    let content = match editor.inspector_tab {
        0 => design_tab(editor, cx),
        2 => history_tab(editor),
        _ => div().into_any_element(),
    };
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
                .child(content),
        )
}

fn design_tab(editor: &EditorView, cx: &mut Context<EditorView>) -> AnyElement {
    let selected = editor.selection.and_then(|id| {
        let element = editor.presentation.element(id)?;
        element.as_text().map(|_| element)
    });
    match selected {
        Some(element) => text_design(editor, element, cx).into_any_element(),
        None => slide_design(editor).into_any_element(),
    }
}

fn header(icon: IconName, title: String, subtitle: String) -> impl IntoElement {
    v_flex()
        .gap(px(4.))
        .px(px(16.))
        .py(px(14.))
        .border_b_1()
        .border_color(theme::border())
        .child(
            h_flex()
                .gap(px(8.))
                .child(Icon::new(icon).size(px(14.)).text_color(theme::accent()))
                .child(
                    div()
                        .id("selection-name")
                        .test_support()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title),
                ),
        )
        .child(
            div()
                .pl(px(22.))
                .text_size(px(11.))
                .line_height(px(14.))
                .text_color(theme::text_muted())
                .child(subtitle),
        )
}

/// Design tab with nothing selected: the slide and its diagnostics.
fn slide_design(editor: &EditorView) -> impl IntoElement {
    let number = editor
        .presentation
        .index_of(editor.current_slide)
        .map_or(0, |ix| ix + 1);
    let size = editor.presentation.size;
    let problems: Vec<(Severity, String)> = editor
        .current_slide()
        .elements
        .iter()
        .flat_map(|element| {
            let name = element_name(element);
            editor
                .presentation
                .text_layout(element.id)
                .map(|layout| text_problems(&layout, &element_font(element)))
                .unwrap_or_default()
                .into_iter()
                .map(move |(severity, message)| (severity, format!("{name}: {message}")))
        })
        .collect();
    v_flex()
        .child(header(
            IconName::Square,
            format!("Slide {number}"),
            format!("{} × {}", size.width, size.height),
        ))
        .child(diagnostics_section(problems))
}

fn element_font(element: &Element) -> String {
    element
        .as_text()
        .map(|text| {
            format!(
                "{} {}",
                text.style.font.family,
                text.style.font.style_name()
            )
        })
        .unwrap_or_default()
}

/// A short name for an element: the start of its text.
fn element_name(element: &Element) -> String {
    let Some(text) = element.as_text() else {
        return "Element".into();
    };
    let first = text.content.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return "Text".into();
    }
    let mut name: String = first.chars().take(28).collect();
    if first.chars().count() > 28 {
        name.push('…');
    }
    name
}

fn sizing_name(sizing: TextSizing) -> &'static str {
    match sizing {
        TextSizing::AutoWidth => "Text · Auto width",
        TextSizing::AutoHeight => "Text · Auto height",
        TextSizing::Fixed => "Text · Fixed size",
    }
}

fn text_design(
    editor: &EditorView,
    element: &Element,
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let text = element.as_text().expect("a text element");
    let style = &text.style;
    let layout = editor.presentation.text_layout(element.id);
    let problems = layout
        .as_ref()
        .map(|layout| text_problems(layout, &element_font(element)))
        .unwrap_or_default();
    v_flex()
        .child(header(
            IconName::Type,
            element_name(element),
            sizing_name(text.sizing).into(),
        ))
        .child(position_section(editor, element, cx))
        .child(layout_section(text.sizing, cx))
        .child(text_section(editor, style, text.sizing, cx))
        .child(fill_section(editor, style))
        .child(diagnostics_section(problems))
}

fn field_row(left: impl IntoElement, right: impl IntoElement) -> impl IntoElement {
    h_flex().gap(px(8.)).child(left).child(right)
}

fn position_section(
    editor: &EditorView,
    element: &Element,
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let inputs = &editor.inspector;
    // Horizontal left, center, right, then vertical top, middle, bottom.
    let aligns: [(&'static str, IconName, Option<f32>, Option<f32>); 6] = [
        ("align-left", IconName::AlignStartVertical, Some(0.), None),
        (
            "align-center",
            IconName::AlignCenterVertical,
            Some(0.5),
            None,
        ),
        ("align-right", IconName::AlignEndVertical, Some(1.), None),
        ("align-top", IconName::AlignStartHorizontal, None, Some(0.)),
        (
            "align-middle",
            IconName::AlignCenterHorizontal,
            None,
            Some(0.5),
        ),
        ("align-bottom", IconName::AlignEndHorizontal, None, Some(1.)),
    ];
    section()
        .child(section_label("POSITION"))
        .child(
            segmented().children(aligns.into_iter().map(|(id, icon, h, v)| {
                segment(id, icon, false, false).on_click(cx.listener(move |this, _, _, cx| {
                    this.align_to_slide(h, v);
                    cx.notify();
                }))
            })),
        )
        .child(field_row(
            input_field(field_letter("X"), inputs.input(Field::X)),
            input_field(field_letter("Y"), inputs.input(Field::Y)),
        ))
        .child(field_row(
            input_field(field_letter("W"), inputs.input(Field::Width)),
            input_field(field_letter("H"), inputs.input(Field::Height)),
        ))
        .child(field_row(
            field(
                field_icon(IconName::RotateCw),
                format!("{}°", number(element.frame.rotation)),
            ),
            div().flex_1(),
        ))
}

fn layout_section(sizing: TextSizing, cx: &mut Context<EditorView>) -> impl IntoElement {
    let modes = [
        (
            "sizing-auto-width",
            IconName::ArrowLeftRight,
            TextSizing::AutoWidth,
        ),
        (
            "sizing-auto-height",
            IconName::ArrowUpDown,
            TextSizing::AutoHeight,
        ),
        ("sizing-fixed", IconName::Square, TextSizing::Fixed),
    ];
    section()
        .child(section_label("LAYOUT"))
        .child(
            segmented().children(modes.into_iter().map(|(id, icon, mode)| {
                segment(id, icon, sizing == mode, false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.set_sizing(mode);
                        cx.notify();
                    },
                ))
            })),
        )
}

/// A toggle or choice that writes one style patch when clicked.
fn style_segment(
    id: &'static str,
    icon: IconName,
    selected: bool,
    disabled: bool,
    label: &'static str,
    patch: TextStylePatch,
    cx: &mut Context<EditorView>,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    segment(id, icon, selected, disabled).when_enabled(!disabled, move |this| {
        this.on_click(cx.listener(move |this, _, _, cx| {
            this.set_style(label, patch.clone());
            cx.notify();
        }))
    })
}

/// `when` for stateful divs that must not install a handler when disabled.
trait WhenEnabled: Sized {
    fn when_enabled(self, enabled: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if enabled { f(self) } else { self }
    }
}

impl<T> WhenEnabled for T {}

fn text_section(
    editor: &EditorView,
    style: &TextStyle,
    sizing: TextSizing,
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let inputs = &editor.inspector;
    let aligns = [
        ("text-align-left", IconName::TextAlignStart, HAlign::Left),
        (
            "text-align-center",
            IconName::TextAlignCenter,
            HAlign::Center,
        ),
        ("text-align-right", IconName::TextAlignEnd, HAlign::Right),
        (
            "text-align-justify",
            IconName::TextAlignJustify,
            HAlign::Justify,
        ),
    ];
    let vertical = [
        (
            "text-align-top",
            IconName::AlignVerticalJustifyStart,
            VAlign::Top,
        ),
        (
            "text-align-middle",
            IconName::AlignVerticalJustifyCenter,
            VAlign::Middle,
        ),
        (
            "text-align-bottom",
            IconName::AlignVerticalJustifyEnd,
            VAlign::Bottom,
        ),
    ];
    // Vertical alignment only places text inside a fixed-height box.
    let fixed = sizing == TextSizing::Fixed;
    let patch = TextStylePatch::default;
    let mut horizontal_segments = Vec::new();
    for (id, icon, align) in aligns {
        horizontal_segments.push(style_segment(
            id,
            icon,
            style.align == align,
            false,
            "Text alignment",
            TextStylePatch {
                align: Some(align),
                ..patch()
            },
            cx,
        ));
    }
    let mut vertical_segments = Vec::new();
    for (id, icon, align) in vertical {
        vertical_segments.push(style_segment(
            id,
            icon,
            fixed && style.vertical_align == align,
            !fixed,
            "Vertical alignment",
            TextStylePatch {
                vertical_align: Some(align),
                ..patch()
            },
            cx,
        ));
    }

    section()
        .child(section_label("TEXT"))
        .child(select_field(
            Select::new(&inputs.family)
                .id("font-family")
                .xsmall()
                .appearance(false)
                .search_placeholder("Search fonts")
                .menu_width(px(232.))
                .menu_max_h(px(320.)),
        ))
        .child(
            h_flex()
                .gap(px(8.))
                .child(select_field(
                    Select::new(&inputs.face)
                        .id("font-face")
                        .xsmall()
                        .appearance(false)
                        .menu_width(px(232.)),
                ))
                .child(div().w(px(84.)).flex_shrink_0().flex().child(input_field(
                    field_icon(IconName::Type),
                    inputs.input(Field::Size),
                ))),
        )
        .child(field_row(
            input_field(
                field_icon(IconName::UnfoldVertical),
                inputs.input(Field::LineHeight),
            ),
            input_field(
                field_icon(IconName::ChevronsLeftRight),
                inputs.input(Field::LetterSpacing),
            ),
        ))
        .child(
            h_flex()
                .gap(px(8.))
                .child(segmented().flex_1().children(horizontal_segments))
                .child(segmented().w(px(84.)).children(vertical_segments)),
        )
        .child(field_row(
            input_field(
                field_icon(IconName::Pilcrow),
                inputs.input(Field::ParagraphSpacing),
            ),
            div().flex_1(),
        ))
        .child(
            h_flex()
                .gap(px(8.))
                .child(
                    segmented()
                        .flex_1()
                        .child(style_segment(
                            "text-underline",
                            IconName::Underline,
                            style.underline,
                            false,
                            "Underline",
                            TextStylePatch {
                                underline: Some(!style.underline),
                                ..patch()
                            },
                            cx,
                        ))
                        .child(style_segment(
                            "text-strikethrough",
                            IconName::Strikethrough,
                            style.strikethrough,
                            false,
                            "Strikethrough",
                            TextStylePatch {
                                strikethrough: Some(!style.strikethrough),
                                ..patch()
                            },
                            cx,
                        )),
                )
                .child(
                    segmented()
                        .flex_1()
                        .child(style_segment(
                            "text-case-original",
                            IconName::CaseSensitive,
                            style.case == TextCase::Original,
                            false,
                            "Letter case",
                            TextStylePatch {
                                case: Some(TextCase::Original),
                                ..patch()
                            },
                            cx,
                        ))
                        .child(style_segment(
                            "text-case-upper",
                            IconName::CaseUpper,
                            style.case == TextCase::Upper,
                            false,
                            "Letter case",
                            TextStylePatch {
                                case: Some(TextCase::Upper),
                                ..patch()
                            },
                            cx,
                        )),
                ),
        )
}

/// A dropdown drawn as a grey property field.
fn select_field(select: impl IntoElement) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .h(px(28.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .bg(theme::field())
        .child(div().w_full().child(select))
}

fn fill_section(editor: &EditorView, style: &TextStyle) -> impl IntoElement {
    section().child(section_label("FILL")).child(
        h_flex()
            .h(px(28.))
            .pl(px(4.))
            .gap(px(6.))
            .rounded(px(6.))
            .bg(theme::field())
            .child(ColorPicker::new(&editor.inspector.color).xsmall())
            .child(
                div()
                    .flex_1()
                    .text_color(theme::text())
                    .child(style.color.hex()),
            )
            .child(
                div()
                    .w(px(64.))
                    .flex_shrink_0()
                    .flex()
                    .child(input_field(div(), editor.inspector.input(Field::Opacity))),
            ),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Severity {
    Warning,
}

/// Diagnostics of one text box's layout.
fn text_problems(layout: &TextLayout, font: &str) -> Vec<(Severity, String)> {
    let mut problems = Vec::new();
    let overflow = layout.overflow();
    if overflow > 0. {
        problems.push((
            Severity::Warning,
            format!("Text overflows its box by {} px", number(overflow.ceil())),
        ));
    }
    if layout.missing_glyphs > 0 {
        let count = layout.missing_glyphs;
        let characters = if count == 1 {
            "character"
        } else {
            "characters"
        };
        problems.push((
            Severity::Warning,
            format!("{count} {characters} missing from {font}"),
        ));
    }
    problems
}

fn diagnostics_section(problems: Vec<(Severity, String)>) -> impl IntoElement {
    let rows: Vec<AnyElement> = if problems.is_empty() {
        vec![diagnostic_row(
            IconName::CircleCheck,
            theme::ok(),
            theme::ok_soft(),
            "All text fits its box".into(),
        )]
    } else {
        problems
            .into_iter()
            .map(|(severity, message)| match severity {
                Severity::Warning => diagnostic_row(
                    IconName::CircleAlert,
                    theme::warn(),
                    theme::warn_soft(),
                    message,
                ),
            })
            .collect()
    };
    section()
        .border_b_0()
        .child(section_label("DIAGNOSTICS"))
        .child(
            v_flex()
                .id("diagnostics")
                .test_support()
                .gap(px(8.))
                .children(rows),
        )
}

fn diagnostic_row(icon: IconName, color: Hsla, ground: Hsla, message: String) -> AnyElement {
    h_flex()
        .gap(px(8.))
        .items_start()
        .child(status_badge(icon, color, ground))
        .child(div().flex_1().child(message))
        .into_any_element()
}

fn status_badge(icon: IconName, color: Hsla, ground: Hsla) -> impl IntoElement {
    div()
        .mt(px(1.))
        .size(px(14.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(ground)
        .child(Icon::new(icon).size(px(12.)).text_color(color))
}

/// The History tab: the undo steps, oldest first, with the current one
/// marked and undone steps faded.
fn history_tab(editor: &EditorView) -> AnyElement {
    let done: Vec<&str> = editor.history.done().collect();
    let undone: Vec<&str> = editor.history.undone().collect();
    let current = done.len();
    let row = |ix: usize, label: String, state: StepState| {
        h_flex()
            .id(("history-step", ix))
            .test_support()
            .h(px(28.))
            .px(px(16.))
            .gap(px(8.))
            .when_enabled(state == StepState::Current, |this| {
                this.bg(theme::accent_soft())
            })
            .child(div().size(px(6.)).rounded_full().bg(match state {
                StepState::Current => theme::accent(),
                StepState::Done => theme::text_muted(),
                StepState::Undone => theme::text_faint(),
            }))
            .child(
                div()
                    .text_color(match state {
                        StepState::Undone => theme::text_faint(),
                        _ => theme::text(),
                    })
                    .child(label),
            )
    };
    let state = |ix: usize| {
        if ix == current {
            StepState::Current
        } else {
            StepState::Done
        }
    };
    v_flex()
        .py(px(8.))
        .child(row(0, "Opened presentation".into(), state(0)))
        .children(
            done.iter()
                .enumerate()
                .map(|(ix, label)| row(ix + 1, label.to_string(), state(ix + 1))),
        )
        .children(
            undone
                .iter()
                .enumerate()
                .map(|(ix, label)| row(current + 1 + ix, label.to_string(), StepState::Undone)),
        )
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StepState {
    Done,
    Current,
    Undone,
}
