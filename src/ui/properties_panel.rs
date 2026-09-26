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

use crate::document::{
    Dash, Element, ElementKind, Fill, HAlign, HeadKind, HeadSize, ImageFit, TextCase, TextSizing,
    TextStyle, TextStylePatch, VAlign,
};
use crate::editor::EditorView;
use crate::text_layout::TextLayout;
use crate::theme;
use crate::ui::image_insert::ImageTarget;
use crate::ui::inspector::{Field, number};
use crate::ui::shape_inspector::{FillType, ShapeField, common};
use crate::ui::widgets::{
    field_icon, field_letter, input_field, section, section_label, segment, segmented, text_segment,
};

/// The diagnostics the Design tab shows: those of the selected text, or of
/// every text on the slide when nothing is selected. Uses the layouts the
/// canvas already computed.
pub fn diagnostics(editor: &mut EditorView) -> Vec<(Severity, String)> {
    let _span = crate::perf::span("diagnostics");
    if let Some(id) = editor.single_selection()
        && let Some(element) = editor.presentation.element(id).cloned()
        && element.as_text().is_some()
    {
        return editor
            .layout_of(id)
            .map(|layout| text_problems(&layout, &element_font(&element)))
            .unwrap_or_default();
    }
    let elements: Vec<Element> = editor
        .current_slide()
        .visible_texts()
        .map(|(element, _)| element.clone())
        .collect();
    let mut problems = Vec::new();
    for element in &elements {
        let name = element_name(element);
        let Some(layout) = editor.layout_of(element.id) else {
            continue;
        };
        for (severity, message) in text_problems(&layout, &element_font(element)) {
            problems.push((severity, format!("{name}: {message}")));
        }
    }
    problems
}

pub fn properties_panel(
    editor: &EditorView,
    problems: Vec<(Severity, String)>,
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let _span = crate::perf::span("properties_panel");
    let content = match editor.inspector_tab {
        0 => design_tab(editor, problems, cx),
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

fn design_tab(
    editor: &EditorView,
    problems: Vec<(Severity, String)>,
    cx: &mut Context<EditorView>,
) -> AnyElement {
    let selected = editor.single_selection().and_then(|id| {
        let element = editor.presentation.element(id)?;
        element.as_text().map(|_| element)
    });
    let shapes = editor.selected_shapes();
    let content = match (selected, shapes) {
        (Some(element), _) => text_design(editor, element, problems, cx).into_any_element(),
        (None, Some(ids)) => shape_design(editor, &ids, cx).into_any_element(),
        (None, None) if !editor.selection.is_empty() => {
            selection_design(editor, cx).into_any_element()
        }
        (None, None) => slide_design(editor, problems).into_any_element(),
    };
    if !editor.selection_locked() {
        return content;
    }
    // A locked selection shows its properties read-only.
    div()
        .id("locked-design")
        .test_support()
        .relative()
        .child(div().opacity(0.6).child(content))
        .child(div().absolute().inset_0().occlude())
        .into_any_element()
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
fn slide_design(editor: &EditorView, problems: Vec<(Severity, String)>) -> impl IntoElement {
    let number = editor
        .presentation
        .index_of(editor.current_slide)
        .map_or(0, |ix| ix + 1);
    let size = editor.presentation.size;
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

/// Design tab for a group or several elements: their union's position
/// and size.
fn selection_design(editor: &EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    let (icon, title, subtitle) = match editor.single_selection() {
        Some(id) => {
            let element = editor.presentation.element(id);
            let children = element.map_or(0, |element| element.children().len());
            (
                IconName::Group,
                element.map(element_name).unwrap_or_default(),
                format!("Group · {children} layers"),
            )
        }
        None => (
            IconName::Layers,
            format!("{} elements", editor.selection.len()),
            "Mixed selection".to_string(),
        ),
    };
    let subtitle = if editor.selection_locked() {
        format!("{subtitle} · Locked")
    } else {
        subtitle
    };
    v_flex()
        .child(header(icon, title, subtitle))
        .child(position_section(editor, false, cx))
}

/// The kind of an element as the panels name it. A rectangle filled with
/// an image is an image.
pub fn kind_name(element: &Element) -> &'static str {
    match &element.kind {
        ElementKind::Text(_) => "Text",
        ElementKind::Group(_) => "Group",
        ElementKind::Rectangle(shape) if matches!(shape.fill, Fill::Image(_)) => "Image",
        ElementKind::Rectangle(_) => "Rectangle",
        ElementKind::Ellipse(_) => "Ellipse",
        ElementKind::Line(_) => "Line",
    }
}

/// The icon of an element's kind in the hierarchy and the inspector.
pub fn element_icon(element: &Element) -> IconName {
    match kind_name(element) {
        "Text" => IconName::Type,
        "Group" => IconName::Group,
        "Image" => IconName::Image,
        "Rectangle" => IconName::Square,
        "Ellipse" => IconName::Circle,
        _ => IconName::Slash,
    }
}

/// A short name for an element: its name, the start of its text, or its
/// kind and its id, such as "Group 4".
pub fn element_name(element: &Element) -> String {
    if let Some(name) = &element.name {
        return name.clone();
    }
    let Some(text) = element.as_text() else {
        return format!("{} {}", kind_name(element), element.id.0);
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
    problems: Vec<(Severity, String)>,
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let text = element.as_text().expect("a text element");
    let style = &text.style;
    v_flex()
        .child(header(
            IconName::Type,
            element_name(element),
            sizing_name(text.sizing).into(),
        ))
        .child(position_section(editor, false, cx))
        .child(layout_section(text.sizing, cx))
        .child(text_section(editor, style, text.sizing, cx))
        .child(fill_section(editor, style))
        .child(diagnostics_section(problems))
}

fn field_row(left: impl IntoElement, right: impl IntoElement) -> impl IntoElement {
    h_flex().gap(px(8.)).child(left).child(right)
}

/// Position, size, rotation and opacity. A line shows its length (L) and
/// no height.
fn position_section(
    editor: &EditorView,
    line: bool,
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
        .child(if line {
            field_row(
                input_field(field_letter("L"), inputs.input(Field::Width)),
                div().flex_1(),
            )
            .into_any_element()
        } else {
            field_row(
                input_field(field_letter("W"), inputs.input(Field::Width)),
                input_field(field_letter("H"), inputs.input(Field::Height)),
            )
            .into_any_element()
        })
        .child(field_row(
            input_field(
                field_icon(IconName::RotateCw),
                inputs.input(Field::Rotation),
            ),
            input_field(field_icon(IconName::Blend), inputs.input(Field::Opacity)),
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
    patch: TextStylePatch,
    cx: &mut Context<EditorView>,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    segment(id, icon, selected, disabled).when_enabled(!disabled, move |this| {
        this.on_click(cx.listener(move |this, _, _, cx| {
            this.set_style(patch.clone());
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
            ),
    )
}

/// Design tab for one shape or several: position, then the style
/// sections every selected shape has.
fn shape_design(
    editor: &EditorView,
    ids: &[crate::document::ElementId],
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let elements: Vec<&Element> = ids
        .iter()
        .filter_map(|id| editor.presentation.element(*id))
        .collect();
    let kinds: Vec<&ElementKind> = elements.iter().map(|element| &element.kind).collect();
    let lines = kinds
        .iter()
        .all(|kind| matches!(kind, ElementKind::Line(_)));
    let rectangles = kinds
        .iter()
        .all(|kind| matches!(kind, ElementKind::Rectangle(_)));
    let filled = kinds.iter().all(|kind| kind.fill().is_some());
    let (icon, title, subtitle) = match elements.as_slice() {
        [element] => (
            element_icon(element),
            element_name(element),
            kind_name(element).to_string(),
        ),
        _ => (
            IconName::Layers,
            format!("{} shapes", elements.len()),
            "Shapes".to_string(),
        ),
    };
    let subtitle = if editor.selection_locked() {
        format!("{subtitle} · Locked")
    } else {
        subtitle
    };
    v_flex()
        .child(header(icon, title, subtitle))
        .child(position_section(editor, lines && ids.len() == 1, cx))
        .when_enabled(rectangles, |this| this.child(corners_section(editor)))
        .when_enabled(filled, |this| {
            this.child(shape_fill_section(editor, &kinds, cx))
        })
        .child(stroke_section(editor, &kinds, cx))
        .when_enabled(lines, |this| this.child(line_ends_section(&kinds, cx)))
}

fn shape_input(editor: &EditorView, prefix: impl IntoElement, field: ShapeField) -> Div {
    input_field(prefix, editor.shape_inspector.input(field))
}

fn corners_section(editor: &EditorView) -> impl IntoElement {
    section().child(section_label("CORNERS")).child(field_row(
        shape_input(
            editor,
            field_icon(IconName::SquareRoundCorner),
            ShapeField::CornerRadius,
        ),
        div().flex_1(),
    ))
}

/// A color swatch with the hex value or "Mixed", and a field after it.
fn color_row(
    picker: &gpui_kit::Entity<gpui_kit::component::color_picker::ColorPickerState>,
    color: Option<crate::document::Rgb>,
    trailing: impl IntoElement,
) -> impl IntoElement {
    h_flex()
        .gap(px(8.))
        .child(
            h_flex()
                .flex_1()
                .h(px(28.))
                .pl(px(4.))
                .gap(px(6.))
                .rounded(px(6.))
                .bg(theme::field())
                .child(ColorPicker::new(picker).xsmall())
                .child(
                    div()
                        .flex_1()
                        .text_color(theme::text())
                        .child(color.map_or("Mixed".to_string(), |color| color.hex())),
                ),
        )
        .child(div().w(px(72.)).flex_shrink_0().flex().child(trailing))
}

fn shape_fill_section(
    editor: &EditorView,
    kinds: &[&ElementKind],
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let fills: Vec<&Fill> = kinds.iter().filter_map(|kind| kind.fill()).collect();
    let fill_type = common(fills.iter().map(|fill| FillType::of(fill)));
    let types = [
        ("fill-none", "None", FillType::None),
        ("fill-solid", "Solid", FillType::Solid),
        ("fill-linear", "Linear", FillType::Linear),
        ("fill-radial", "Radial", FillType::Radial),
        ("fill-image", "Image", FillType::Image),
    ];
    let picker = segmented().children(types.into_iter().map(|(id, label, kind)| {
        text_segment(id, label, fill_type == Some(kind)).on_click(cx.listener(
            move |this, _, window, cx| {
                if kind == FillType::Image {
                    if this.selected_fill_type() != Some(FillType::Image) {
                        this.choose_image(ImageTarget::Fill, window, cx);
                    }
                } else {
                    this.set_fill_type(kind);
                }
                cx.notify();
            },
        ))
    }));
    let inspector = &editor.shape_inspector;
    let mut section = section().child(section_label("FILL")).child(picker);
    match fill_type {
        Some(FillType::Solid) => {
            let color = common(fills.iter().filter_map(|fill| match fill {
                Fill::Solid(solid) => Some(solid.color),
                _ => None,
            }));
            section = section.child(color_row(
                &inspector.fill_color,
                color,
                shape_input(editor, div(), ShapeField::FillOpacity),
            ));
        }
        Some(FillType::Linear) => {
            section = section
                .child(field_row(
                    shape_input(
                        editor,
                        field_icon(IconName::RotateCw),
                        ShapeField::GradientAngle,
                    ),
                    div().flex_1(),
                ))
                .child(stops(editor, &fills, cx));
        }
        Some(FillType::Radial) => {
            section = section
                .child(field_row(
                    shape_input(editor, field_letter("X"), ShapeField::CenterX),
                    shape_input(editor, field_letter("Y"), ShapeField::CenterY),
                ))
                .child(field_row(
                    shape_input(editor, field_letter("W"), ShapeField::RadiusX),
                    shape_input(editor, field_letter("H"), ShapeField::RadiusY),
                ))
                .child(stops(editor, &fills, cx));
        }
        Some(FillType::Image) => {
            let fit = common(fills.iter().filter_map(|fill| match fill {
                Fill::Image(image) => Some(image.fit),
                _ => None,
            }));
            let fits = ImageFit::ALL.into_iter().map(|option| {
                let id = match option {
                    ImageFit::Cover => "fit-cover",
                    ImageFit::Contain => "fit-contain",
                    ImageFit::Stretch => "fit-stretch",
                };
                text_segment(id, option.label(), fit == Some(option)).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.set_image_fit(option);
                        cx.notify();
                    },
                ))
            });
            section = section.child(segmented().children(fits)).child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        h_flex()
                            .id("replace-image")
                            .test_support()
                            .flex_1()
                            .h(px(28.))
                            .px(px(8.))
                            .gap(px(6.))
                            .rounded(px(6.))
                            .bg(theme::field())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_image(ImageTarget::Fill, window, cx);
                            }))
                            .child(field_icon(IconName::Image))
                            .child(div().text_color(theme::text()).child("Replace…")),
                    )
                    .child(div().w(px(72.)).flex_shrink_0().flex().child(shape_input(
                        editor,
                        div(),
                        ShapeField::FillOpacity,
                    ))),
            );
        }
        _ => {}
    }
    section
}

/// One row per gradient stop, when the gradients have the same number of
/// stops, and a button that adds one.
fn stops(editor: &EditorView, fills: &[&Fill], cx: &mut Context<EditorView>) -> AnyElement {
    let Some(count) = common(fills.iter().filter_map(|fill| fill.stops()).map(<[_]>::len)) else {
        return div()
            .text_color(theme::text_muted())
            .child("Mixed stops")
            .into_any_element();
    };
    let inspector = &editor.shape_inspector;
    let rows = (0..count).map(|index| {
        let color = common(
            fills
                .iter()
                .filter_map(|fill| Some(fill.stops()?.get(index)?.color)),
        );
        let remove = div()
            .id(("remove-stop", index))
            .test_support()
            .size(px(20.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when_enabled(count > 2, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(theme::field()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_gradient_stop(index);
                        cx.notify();
                    }))
            })
            .child(
                Icon::new(IconName::X)
                    .size(px(12.))
                    .text_color(if count > 2 {
                        theme::text_muted()
                    } else {
                        theme::text_faint()
                    }),
            );
        h_flex()
            .gap(px(4.))
            .child(div().flex_1().child(color_row(
                &inspector.stop_colors[index],
                color,
                shape_input(editor, div(), ShapeField::StopPosition(index)),
            )))
            .child(remove)
    });
    let full = count >= crate::style::MAX_STOPS;
    v_flex()
        .gap(px(8.))
        .children(rows)
        .child(
            h_flex()
                .id("add-stop")
                .test_support()
                .h(px(24.))
                .gap(px(6.))
                .text_color(if full {
                    theme::text_faint()
                } else {
                    theme::text_muted()
                })
                .when_enabled(!full, |this| {
                    this.cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.add_gradient_stop();
                            cx.notify();
                        }))
                })
                .child(Icon::new(IconName::Plus).size(px(12.)))
                .child("Add stop"),
        )
        .into_any_element()
}

fn stroke_section(
    editor: &EditorView,
    kinds: &[&ElementKind],
    cx: &mut Context<EditorView>,
) -> impl IntoElement {
    let strokes: Vec<_> = kinds.iter().filter_map(|kind| kind.stroke()).collect();
    let optional = kinds
        .iter()
        .any(|kind| !matches!(kind, ElementKind::Line(_)));
    let all = strokes.len() == kinds.len();
    let toggle = h_flex()
        .id("stroke-toggle")
        .test_support()
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|style| style.bg(theme::field()))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_stroke(!all);
            cx.notify();
        }))
        .child(
            Icon::new(if all { IconName::Minus } else { IconName::Plus })
                .size(px(12.))
                .text_color(theme::text_muted()),
        );
    let mut section = section().child(
        h_flex()
            .justify_between()
            .child(section_label("STROKE"))
            .when_enabled(optional, |this| this.child(toggle)),
    );
    if !all {
        return section;
    }
    let inspector = &editor.shape_inspector;
    let dash = common(strokes.iter().map(|stroke| stroke.dash));
    section = section
        .child(color_row(
            &inspector.stroke_color,
            common(strokes.iter().map(|stroke| stroke.color)),
            shape_input(editor, div(), ShapeField::StrokeOpacity),
        ))
        .child(
            h_flex()
                .gap(px(8.))
                .child(div().w(px(84.)).flex_shrink_0().flex().child(shape_input(
                    editor,
                    field_letter("W"),
                    ShapeField::StrokeWidth,
                )))
                .child(
                    segmented()
                        .flex_1()
                        .children(Dash::ALL.into_iter().map(|option| {
                            let id = match option {
                                Dash::Solid => "dash-solid",
                                Dash::Dashed => "dash-dashed",
                                Dash::Dotted => "dash-dotted",
                            };
                            text_segment(id, option.label(), dash == Some(option)).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.set_dash(option);
                                    cx.notify();
                                }),
                            )
                        })),
                ),
        );
    section
}

fn line_ends_section(kinds: &[&ElementKind], cx: &mut Context<EditorView>) -> impl IntoElement {
    let heads = |start: bool| {
        common(kinds.iter().filter_map(|kind| match kind {
            ElementKind::Line(line) => Some(if start { line.start } else { line.end }),
            _ => None,
        }))
    };
    let row = |start: bool, cx: &mut Context<EditorView>| {
        let head = heads(start);
        let prefix = if start { "start" } else { "end" };
        let kinds = HeadKind::ALL.into_iter().map(|kind| {
            let icon = match kind {
                HeadKind::None => IconName::Minus,
                HeadKind::Triangle => IconName::Triangle,
                HeadKind::Arrow if start => IconName::ChevronLeft,
                HeadKind::Arrow => IconName::ChevronRight,
                HeadKind::Diamond => IconName::Diamond,
                HeadKind::Circle => IconName::Circle,
            };
            let id = (
                if start { "start-head" } else { "end-head" },
                HeadKind::ALL
                    .iter()
                    .position(|other| *other == kind)
                    .unwrap_or(0),
            );
            segment(id, icon, head.map(|head| head.kind) == Some(kind), false).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.set_arrowhead(start, Some(kind), None);
                    cx.notify();
                }),
            )
        });
        let sizes = HeadSize::ALL.into_iter().map(|size| {
            let (label, index) = match size {
                HeadSize::Small => ("S", 0usize),
                HeadSize::Medium => ("M", 1),
                HeadSize::Large => ("L", 2),
            };
            text_segment(
                (if start { "start-size" } else { "end-size" }, index),
                label,
                head.map(|head| head.size) == Some(size),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_arrowhead(start, None, Some(size));
                cx.notify();
            }))
        });
        v_flex()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(if start { "Start" } else { "End" }),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(segmented().flex_1().children(kinds))
                    .child(segmented().w(px(72.)).children(sizes)),
            )
            .id(prefix)
    };
    section()
        .child(
            h_flex()
                .justify_between()
                .child(section_label("LINE ENDS"))
                .child(
                    div()
                        .id("swap-ends")
                        .test_support()
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::field()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.swap_arrowheads();
                            cx.notify();
                        }))
                        .child(
                            Icon::new(IconName::ArrowLeftRight)
                                .size(px(12.))
                                .text_color(theme::text_muted()),
                        ),
                ),
        )
        .child(row(true, cx))
        .child(row(false, cx))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
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
