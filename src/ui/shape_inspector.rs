//! State of the inspector's shape fields and the edits they make.
//!
//! The fields show the selected shapes: one, or several of any kind. A
//! value the shapes do not share shows as "Mixed"; typing a value writes it
//! into every selected shape that has the field, as one undo step. Like the
//! text fields, a field commits on Enter or when it loses the focus.

use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, Focusable as _, Hsla, Subscription, Window};

use crate::document::{
    Arrowhead, Dash, ElementId, ElementKind, Fill, GradientStop, HeadKind, HeadSize,
    LinearGradient, Operation, RadialGradient, Rgb, ShapeStylePatch, SolidFill, Stroke, Vec2,
};
use crate::editor::EditorView;
use crate::style::MAX_STOPS;
use crate::ui::inspector::number;

/// A numeric field of the shape sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeField {
    FillOpacity,
    StrokeWidth,
    StrokeOpacity,
    CornerRadius,
    GradientAngle,
    CenterX,
    CenterY,
    RadiusX,
    RadiusY,
    /// The position of a gradient stop, by index.
    StopPosition(usize),
}

impl ShapeField {
    fn all() -> Vec<ShapeField> {
        let mut fields = vec![
            ShapeField::FillOpacity,
            ShapeField::StrokeWidth,
            ShapeField::StrokeOpacity,
            ShapeField::CornerRadius,
            ShapeField::GradientAngle,
            ShapeField::CenterX,
            ShapeField::CenterY,
            ShapeField::RadiusX,
            ShapeField::RadiusY,
        ];
        fields.extend((0..MAX_STOPS).map(ShapeField::StopPosition));
        fields
    }

    /// The field's value in one shape; `None` when the shape does not have
    /// it. Percentages are 0 to 100.
    fn value(self, kind: &ElementKind) -> Option<f32> {
        let fill = kind.fill();
        let stroke = kind.stroke();
        match self {
            ShapeField::FillOpacity => match fill? {
                Fill::Solid(solid) => Some(solid.opacity * 100.),
                Fill::Image(image) => Some(image.opacity * 100.),
                _ => None,
            },
            ShapeField::StrokeWidth => Some(stroke?.width),
            ShapeField::StrokeOpacity => Some(stroke?.opacity * 100.),
            ShapeField::CornerRadius => match kind {
                ElementKind::Rectangle(shape) => Some(shape.corner_radius),
                _ => None,
            },
            ShapeField::GradientAngle => match fill? {
                Fill::LinearGradient(gradient) => Some(gradient.angle),
                _ => None,
            },
            ShapeField::CenterX
            | ShapeField::CenterY
            | ShapeField::RadiusX
            | ShapeField::RadiusY => {
                let Fill::RadialGradient(gradient) = fill? else {
                    return None;
                };
                Some(
                    100. * match self {
                        ShapeField::CenterX => gradient.center.x,
                        ShapeField::CenterY => gradient.center.y,
                        ShapeField::RadiusX => gradient.radius.x,
                        _ => gradient.radius.y,
                    },
                )
            }
            ShapeField::StopPosition(index) => Some(fill?.stops()?.get(index)?.position * 100.),
        }
    }

    fn show(self, value: f32) -> String {
        match self {
            ShapeField::StrokeWidth | ShapeField::CornerRadius => number(value),
            ShapeField::GradientAngle => format!("{}°", number(value)),
            _ => format!("{}%", number(value)),
        }
    }

    /// The patch that writes `value` into one shape; `None` when the shape
    /// does not have the field or the value does not fit it.
    fn patch(self, value: f32, kind: &ElementKind) -> Option<ShapeStylePatch> {
        let percent = (0. ..=100.).contains(&value).then_some(value / 100.);
        let stroke = |change: &dyn Fn(&mut Stroke)| {
            let mut stroke = *kind.stroke()?;
            change(&mut stroke);
            Some(ShapeStylePatch {
                stroke: Some(Some(stroke)),
                ..ShapeStylePatch::default()
            })
        };
        let fill = |change: &dyn Fn(&mut Fill) -> Option<()>| {
            let mut fill = kind.fill()?.clone();
            change(&mut fill)?;
            Some(ShapeStylePatch {
                fill: Some(fill),
                ..ShapeStylePatch::default()
            })
        };
        match self {
            ShapeField::FillOpacity => {
                let opacity = percent?;
                fill(&|fill| {
                    match fill {
                        Fill::Solid(solid) => solid.opacity = opacity,
                        Fill::Image(image) => image.opacity = opacity,
                        _ => return None,
                    }
                    Some(())
                })
            }
            ShapeField::StrokeWidth if value > 0. => stroke(&|stroke| stroke.width = value),
            ShapeField::StrokeOpacity => {
                let opacity = percent?;
                stroke(&|stroke| stroke.opacity = opacity)
            }
            ShapeField::CornerRadius if value >= 0. => matches!(kind, ElementKind::Rectangle(_))
                .then(|| ShapeStylePatch {
                    corner_radius: Some(value),
                    ..ShapeStylePatch::default()
                }),
            ShapeField::GradientAngle => fill(&|fill| {
                let Fill::LinearGradient(gradient) = fill else {
                    return None;
                };
                gradient.angle = crate::document::normalize_degrees(value);
                Some(())
            }),
            ShapeField::CenterX
            | ShapeField::CenterY
            | ShapeField::RadiusX
            | ShapeField::RadiusY => {
                let fraction = value / 100.;
                let radius = matches!(self, ShapeField::RadiusX | ShapeField::RadiusY);
                if radius && fraction <= 0. {
                    return None;
                }
                fill(&|fill| {
                    let Fill::RadialGradient(gradient) = fill else {
                        return None;
                    };
                    match self {
                        ShapeField::CenterX => gradient.center.x = fraction,
                        ShapeField::CenterY => gradient.center.y = fraction,
                        ShapeField::RadiusX => gradient.radius.x = fraction,
                        _ => gradient.radius.y = fraction,
                    }
                    Some(())
                })
            }
            ShapeField::StopPosition(index) => {
                let position = percent?;
                fill(&|fill| {
                    let stops = fill.stops_mut()?;
                    // Between its neighbors, so the stops stay in order.
                    let low = index
                        .checked_sub(1)
                        .map_or(0., |before| stops[before].position);
                    let high = stops.get(index + 1).map_or(1., |after| after.position);
                    stops.get_mut(index)?.position = position.clamp(low, high);
                    Some(())
                })
            }
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            ShapeField::FillOpacity => "Fill opacity",
            ShapeField::StrokeWidth => "Stroke width",
            ShapeField::StrokeOpacity => "Stroke opacity",
            ShapeField::CornerRadius => "Corner radius",
            ShapeField::GradientAngle => "Gradient angle",
            ShapeField::CenterX | ShapeField::CenterY => "Gradient center",
            ShapeField::RadiusX | ShapeField::RadiusY => "Gradient radius",
            ShapeField::StopPosition(_) => "Gradient stop",
        }
    }
}

/// The type of fill the inspector offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillType {
    None,
    Solid,
    Linear,
    Radial,
    Image,
}

impl FillType {
    pub fn of(fill: &Fill) -> FillType {
        match fill {
            Fill::None => FillType::None,
            Fill::Solid(_) => FillType::Solid,
            Fill::LinearGradient(_) => FillType::Linear,
            Fill::RadialGradient(_) => FillType::Radial,
            Fill::Image(_) => FillType::Image,
        }
    }
}

/// The first color of a fill, which a new fill type keeps.
fn main_color(fill: &Fill) -> Rgb {
    match fill {
        Fill::Solid(solid) => solid.color,
        Fill::LinearGradient(_) | Fill::RadialGradient(_) => {
            fill.stops().map_or(Rgb(0xD9D9D9), |stops| stops[0].color)
        }
        Fill::None | Fill::Image(_) => Rgb(0xD9D9D9),
    }
}

/// The stops a new gradient takes from the current fill.
fn stops_from(fill: &Fill) -> Vec<GradientStop> {
    match fill.stops() {
        Some(stops) => stops.to_vec(),
        None => vec![
            GradientStop::new(0., main_color(fill)),
            GradientStop::new(1., Rgb(0xFFFFFF)),
        ],
    }
}

/// The value the shapes share, or `None` when they differ or none has it.
pub fn common<T: PartialEq>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn to_rgb(color: Hsla) -> Rgb {
    let rgba = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;
    Rgb(channel(rgba.r) << 16 | channel(rgba.g) << 8 | channel(rgba.b))
}

/// Which color a color picker of the shape sections edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorTarget {
    Fill,
    Stroke,
    Stop(usize),
}

pub struct ShapeInspector {
    fields: Vec<(ShapeField, Entity<InputState>)>,
    pub fill_color: Entity<ColorPickerState>,
    pub stroke_color: Entity<ColorPickerState>,
    pub stop_colors: Vec<Entity<ColorPickerState>>,
    /// Shapes the fields show. A field left by clicking elsewhere commits
    /// to these, not to the new selection.
    shown: Vec<ElementId>,
    /// Fields the author typed into and has not committed yet.
    dirty: Vec<ShapeField>,
    _subscriptions: Vec<Subscription>,
}

impl ShapeInspector {
    pub fn new(window: &mut Window, cx: &mut Context<EditorView>) -> Self {
        let mut subscriptions = Vec::new();
        let fields = ShapeField::all()
            .into_iter()
            .map(|field| {
                let input = cx.new(|cx| InputState::new(window, cx));
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this: &mut EditorView, _, event: &InputEvent, window, cx| match event {
                        InputEvent::Change => {
                            if !this.shape_inspector.dirty.contains(&field) {
                                this.shape_inspector.dirty.push(field);
                            }
                        }
                        InputEvent::PressEnter { .. } | InputEvent::Blur => {
                            this.commit_shape_field(field, window, cx);
                        }
                        InputEvent::Focus => {}
                    },
                ));
                (field, input)
            })
            .collect();
        let mut picker = |target: ColorTarget| {
            let state = cx.new(|cx| ColorPickerState::new(window, cx));
            subscriptions.push(cx.subscribe_in(
                &state,
                window,
                move |this: &mut EditorView, _, event: &ColorPickerEvent, _, cx| {
                    let ColorPickerEvent::Change(Some(color)) = event else {
                        return;
                    };
                    this.set_shape_color(target, to_rgb(*color));
                    cx.notify();
                },
            ));
            state
        };
        let fill_color = picker(ColorTarget::Fill);
        let stroke_color = picker(ColorTarget::Stroke);
        let stop_colors = (0..MAX_STOPS)
            .map(|index| picker(ColorTarget::Stop(index)))
            .collect();
        Self {
            fields,
            fill_color,
            stroke_color,
            stop_colors,
            shown: Vec::new(),
            dirty: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn input(&self, field: ShapeField) -> &Entity<InputState> {
        &self
            .fields
            .iter()
            .find(|(candidate, _)| *candidate == field)
            .expect("every shape field has an input")
            .1
    }
}

impl EditorView {
    /// The selected elements when every one is a shape.
    pub fn selected_shapes(&self) -> Option<Vec<ElementId>> {
        if self.selection.is_empty() {
            return None;
        }
        self.selection
            .iter()
            .all(|id| {
                self.presentation
                    .element(*id)
                    .is_some_and(|element| element.kind.is_shape())
            })
            .then(|| self.selection.clone())
    }

    fn shape_kinds(&self, ids: &[ElementId]) -> Vec<ElementKind> {
        ids.iter()
            .filter_map(|id| self.presentation.element(*id))
            .map(|element| element.kind.clone())
            .collect()
    }

    /// What a field shows for shapes: their shared value, "Mixed", or
    /// nothing when none has the field.
    fn shape_field_text(&self, field: ShapeField, ids: &[ElementId]) -> String {
        let values: Vec<f32> = self
            .shape_kinds(ids)
            .iter()
            .filter_map(|kind| field.value(kind))
            .collect();
        match (values.first(), common(values.iter())) {
            (None, _) => String::new(),
            (Some(_), Some(value)) => field.show(*value),
            (Some(_), None) => "Mixed".into(),
        }
    }

    /// Updates the shape fields that are not being typed into to show the
    /// selected shapes. Runs before every render.
    pub fn sync_shape_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in self.shape_inspector.dirty.clone() {
            let focused = self
                .shape_inspector
                .input(field)
                .read(cx)
                .focus_handle(cx)
                .is_focused(window);
            if !focused {
                self.commit_shape_field(field, window, cx);
            }
        }
        let Some(ids) = self.selected_shapes() else {
            self.shape_inspector.shown.clear();
            return;
        };
        for (field, input) in &self.shape_inspector.fields {
            let shown = self.shape_field_text(*field, &ids);
            let state = input.read(cx);
            if !state.focus_handle(cx).is_focused(window) && state.value() != shown.as_str() {
                input.update(cx, |state, cx| state.set_value(shown, window, cx));
            }
        }
        let kinds = self.shape_kinds(&ids);
        let fill = common(kinds.iter().filter_map(|kind| match kind.fill()? {
            Fill::Solid(solid) => Some(solid.color),
            _ => None,
        }));
        let stroke = common(kinds.iter().filter_map(|kind| Some(kind.stroke()?.color)));
        let mut targets = vec![
            (self.shape_inspector.fill_color.clone(), fill),
            (self.shape_inspector.stroke_color.clone(), stroke),
        ];
        for (index, picker) in self.shape_inspector.stop_colors.iter().enumerate() {
            let color = common(
                kinds
                    .iter()
                    .filter_map(|kind| Some(kind.fill()?.stops()?.get(index)?.color)),
            );
            targets.push((picker.clone(), color));
        }
        for (picker, color) in targets {
            let Some(color) = color else {
                continue;
            };
            if picker
                .read(cx)
                .value()
                .is_none_or(|current| to_rgb(current) != color)
            {
                let hsla: Hsla = gpui_kit::rgb(color.0).into();
                picker.update(cx, |state, cx| state.set_value(hsla, window, cx));
            }
        }
        self.shape_inspector.shown = ids;
    }

    /// The fill type the selected shapes share.
    pub fn selected_fill_type(&self) -> Option<FillType> {
        let ids = self.selected_shapes()?;
        common(
            self.shape_kinds(&ids)
                .iter()
                .filter_map(|kind| Some(FillType::of(kind.fill()?))),
        )
    }

    /// Changes the shapes as one undo step. `patch` gives the change of
    /// each shape; shapes it gives none for stay as they are.
    pub fn edit_shapes(
        &mut self,
        label: &str,
        ids: &[ElementId],
        patch: impl Fn(&ElementKind) -> Option<ShapeStylePatch>,
    ) -> bool {
        let operations: Vec<Operation> = ids
            .iter()
            .filter_map(|id| {
                let element = self.presentation.element(*id)?;
                let patch = patch(&element.kind)?;
                Some(Operation::SetShapeStyle { id: *id, patch })
            })
            .collect();
        if operations.is_empty() {
            return false;
        }
        let selection = self.selection.clone();
        let done = self.commit(label, Operation::Batch(operations), selection.clone());
        self.selection = selection;
        done
    }

    /// Changes the selected shapes; see [`Self::edit_shapes`].
    pub fn edit_selected_shapes(
        &mut self,
        label: &str,
        patch: impl Fn(&ElementKind) -> Option<ShapeStylePatch>,
    ) -> bool {
        let Some(ids) = self.selected_shapes() else {
            return false;
        };
        self.edit_shapes(label, &ids, patch)
    }

    /// Commits what the author typed into a shape field. Unreadable or
    /// unchanged input is replaced by the current value.
    pub fn commit_shape_field(
        &mut self,
        field: ShapeField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shape_inspector.dirty.retain(|dirty| *dirty != field);
        let ids = self.shape_inspector.shown.clone();
        let input = self.shape_inspector.input(field).clone();
        let typed = input.read(cx).value().to_string();
        if typed != self.shape_field_text(field, &ids)
            && let Some(value) = crate::ui::inspector::parse(&typed)
        {
            self.edit_shapes(field.label(), &ids, |kind| {
                let patch = field.patch(value, kind)?;
                // Leaves out shapes the value does not change.
                let mut changed = kind.clone();
                patch.clone().apply_to(ElementId(0), &mut changed).ok()?;
                (changed != *kind).then_some(patch)
            });
        }
        let shown = self.shape_field_text(field, &ids);
        input.update(cx, |state, cx| state.set_value(shown, window, cx));
        cx.notify();
    }

    fn set_shape_color(&mut self, target: ColorTarget, color: Rgb) {
        let label = match target {
            ColorTarget::Fill => "Fill color",
            ColorTarget::Stroke => "Stroke color",
            ColorTarget::Stop(_) => "Gradient stop",
        };
        self.edit_selected_shapes(label, |kind| match target {
            ColorTarget::Fill => match kind.fill()? {
                Fill::Solid(solid) if solid.color != color => Some(ShapeStylePatch {
                    fill: Some(Fill::Solid(SolidFill { color, ..*solid })),
                    ..ShapeStylePatch::default()
                }),
                _ => None,
            },
            ColorTarget::Stroke => {
                let stroke = kind.stroke()?;
                (stroke.color != color).then(|| ShapeStylePatch {
                    stroke: Some(Some(Stroke { color, ..*stroke })),
                    ..ShapeStylePatch::default()
                })
            }
            ColorTarget::Stop(index) => {
                let mut fill = kind.fill()?.clone();
                let stop = fill.stops_mut()?.get_mut(index)?;
                if stop.color == color {
                    return None;
                }
                stop.color = color;
                Some(ShapeStylePatch {
                    fill: Some(fill),
                    ..ShapeStylePatch::default()
                })
            }
        });
    }

    /// Gives the selected rectangles and ellipses a fill of another type,
    /// keeping their colors where the new type has them.
    pub fn set_fill_type(&mut self, fill_type: FillType) {
        self.edit_selected_shapes("Fill", |kind| {
            let fill = kind.fill()?;
            if FillType::of(fill) == fill_type {
                return None;
            }
            let new = match fill_type {
                FillType::None => Fill::None,
                FillType::Solid => Fill::Solid(SolidFill {
                    color: main_color(fill),
                    opacity: 1.,
                }),
                FillType::Linear => Fill::LinearGradient(LinearGradient {
                    angle: 0.,
                    stops: stops_from(fill),
                }),
                FillType::Radial => Fill::RadialGradient(RadialGradient {
                    center: Vec2::new(0.5, 0.5),
                    radius: Vec2::new(0.5, 0.5),
                    stops: stops_from(fill),
                }),
                FillType::Image => return None,
            };
            Some(ShapeStylePatch {
                fill: Some(new),
                ..ShapeStylePatch::default()
            })
        });
    }

    /// Adds a stop to the gradients of the selected shapes, halfway between
    /// the last two, with the color of the last.
    pub fn add_gradient_stop(&mut self) {
        self.edit_selected_shapes("Add gradient stop", |kind| {
            let mut fill = kind.fill()?.clone();
            let stops = fill.stops_mut()?;
            if stops.len() >= MAX_STOPS {
                return None;
            }
            let last = *stops.last()?;
            let before = stops[stops.len() - 2];
            let stop = GradientStop {
                position: (before.position + last.position) / 2.,
                ..last
            };
            stops.insert(stops.len() - 1, stop);
            Some(ShapeStylePatch {
                fill: Some(fill),
                ..ShapeStylePatch::default()
            })
        });
    }

    /// Removes a stop from the gradients of the selected shapes; a gradient
    /// keeps at least two.
    pub fn remove_gradient_stop(&mut self, index: usize) {
        self.edit_selected_shapes("Remove gradient stop", |kind| {
            let mut fill = kind.fill()?.clone();
            let stops = fill.stops_mut()?;
            if stops.len() <= 2 || index >= stops.len() {
                return None;
            }
            stops.remove(index);
            Some(ShapeStylePatch {
                fill: Some(fill),
                ..ShapeStylePatch::default()
            })
        });
    }

    /// Adds the default stroke to the selected rectangles and ellipses that
    /// have none, or removes the stroke of all of them.
    pub fn set_stroke(&mut self, on: bool) {
        let label = if on { "Add stroke" } else { "Remove stroke" };
        self.edit_selected_shapes(label, |kind| {
            if matches!(kind, ElementKind::Line(_)) || kind.stroke().is_some() == on {
                return None;
            }
            Some(ShapeStylePatch {
                stroke: Some(on.then(Stroke::default)),
                ..ShapeStylePatch::default()
            })
        });
    }

    pub fn set_dash(&mut self, dash: Dash) {
        self.edit_selected_shapes("Dash", |kind| {
            let stroke = kind.stroke()?;
            (stroke.dash != dash).then(|| ShapeStylePatch {
                stroke: Some(Some(Stroke { dash, ..*stroke })),
                ..ShapeStylePatch::default()
            })
        });
    }

    /// Changes the head at the start (`start`) or the end of the selected
    /// lines: its kind, its size, or both.
    pub fn set_arrowhead(&mut self, start: bool, kind: Option<HeadKind>, size: Option<HeadSize>) {
        let label = if start { "Line start" } else { "Line end" };
        self.edit_selected_shapes(label, |element| {
            let ElementKind::Line(line) = element else {
                return None;
            };
            let current = if start { line.start } else { line.end };
            let head = Arrowhead::new(kind.unwrap_or(current.kind), size.unwrap_or(current.size));
            if head == current {
                return None;
            }
            Some(if start {
                ShapeStylePatch {
                    start: Some(head),
                    ..ShapeStylePatch::default()
                }
            } else {
                ShapeStylePatch {
                    end: Some(head),
                    ..ShapeStylePatch::default()
                }
            })
        });
    }

    /// Swaps the heads of the selected lines.
    pub fn swap_arrowheads(&mut self) {
        self.edit_selected_shapes("Swap line ends", |element| {
            let ElementKind::Line(line) = element else {
                return None;
            };
            (line.start != line.end).then(|| ShapeStylePatch {
                start: Some(line.end),
                end: Some(line.start),
                ..ShapeStylePatch::default()
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::RectangleElement;

    #[test]
    fn stop_positions_stay_between_their_neighbors() {
        let kind = ElementKind::Rectangle(RectangleElement {
            fill: Fill::LinearGradient(LinearGradient {
                angle: 0.,
                stops: vec![
                    GradientStop::new(0., Rgb(0)),
                    GradientStop::new(0.5, Rgb(0)),
                    GradientStop::new(1., Rgb(0)),
                ],
            }),
            ..RectangleElement::default()
        });
        let patch = ShapeField::StopPosition(0).patch(80., &kind).unwrap();
        let stops = patch.fill.unwrap().stops().unwrap().to_vec();
        assert_eq!(stops[0].position, 0.5);
    }

    #[test]
    fn fields_a_shape_lacks_make_no_patch() {
        let kind = ElementKind::Rectangle(RectangleElement::default());
        assert!(ShapeField::StrokeWidth.patch(3., &kind).is_none());
        assert!(ShapeField::GradientAngle.patch(3., &kind).is_none());
        assert!(ShapeField::CornerRadius.patch(-1., &kind).is_none());
        assert_eq!(ShapeField::FillOpacity.value(&kind), Some(100.));
    }
}
