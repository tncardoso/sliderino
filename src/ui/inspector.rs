//! State of the inspector's editable fields and the edits they make.
//!
//! The fields show the selected text box. While a field has focus it keeps
//! what the author types; Enter or leaving the field commits it as one undo
//! step. Otherwise the fields follow the document, including changes made by
//! undo or by other editors.

use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::searchable_list::{SearchableGroup, SearchableListItem, SearchableVec};
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, Hsla, SharedString, Subscription, Window,
};

use crate::document::{
    ElementId, FontFace, Frame, LayerPatch, LineHeight, Operation, Presentation, Rgb, TextSizing,
    TextStyle, TextStylePatch,
};
use crate::editor::EditorView;
use crate::fonts;

/// A numeric field of the inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    X,
    Y,
    Width,
    Height,
    Rotation,
    Size,
    LineHeight,
    LetterSpacing,
    ParagraphSpacing,
    Opacity,
}

impl Field {
    const ALL: [Field; 10] = [
        Field::X,
        Field::Y,
        Field::Width,
        Field::Height,
        Field::Rotation,
        Field::Size,
        Field::LineHeight,
        Field::LetterSpacing,
        Field::ParagraphSpacing,
        Field::Opacity,
    ];

    /// X, Y, width, height or rotation: the fields a group or several
    /// elements show.
    pub fn is_position(self) -> bool {
        matches!(
            self,
            Field::X | Field::Y | Field::Width | Field::Height | Field::Rotation
        )
    }

    /// The field's text for a frame and style.
    fn show(self, frame: &Frame, style: &TextStyle, opacity: f32) -> String {
        match self {
            Field::X => number(frame.x),
            Field::Y => number(frame.y),
            Field::Width => number(frame.width),
            Field::Height => number(frame.height),
            Field::Rotation => format!("{}°", number(frame.rotation)),
            Field::Size => number(style.size),
            Field::LineHeight => match style.line_height {
                LineHeight::Auto => "Auto".into(),
                LineHeight::Percent(percent) => format!("{}%", number(percent)),
            },
            Field::LetterSpacing => format!("{}%", number(style.letter_spacing)),
            Field::ParagraphSpacing => number(style.paragraph_spacing),
            Field::Opacity => format!("{}%", number(opacity * 100.)),
        }
    }
}

/// A number as the inspector shows it: at most two decimals, no trailing
/// zeros.
pub fn number(value: f32) -> String {
    let rounded = (value * 100.).round() / 100.;
    if rounded.fract() == 0. {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

/// Parses what the author typed: a number, optionally followed by "%" or
/// "°".
pub fn parse(text: &str) -> Option<f32> {
    let value: f32 = text
        .trim()
        .trim_end_matches(['%', '°'])
        .trim()
        .parse()
        .ok()?;
    value.is_finite().then_some(value)
}

/// A face in the face picker. Faces whose license forbids embedding are
/// listed but cannot be picked.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceItem {
    face: FontFace,
    title: SharedString,
    restricted: bool,
}

impl FaceItem {
    fn new(face: FontFace, restricted: bool) -> Self {
        let title = if restricted {
            format!("{} — {}", face.style_name(), fonts::RESTRICTED_LICENSE)
        } else {
            face.style_name()
        };
        Self {
            face,
            title: title.into(),
            restricted,
        }
    }
}

impl SearchableListItem for FaceItem {
    type Value = FontFace;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &FontFace {
        &self.face
    }

    fn disabled(&self) -> bool {
        self.restricted
    }
}

/// The families of the family picker: those embedded in the presentation
/// first, then the others.
pub type FamilyList = SearchableVec<SearchableGroup<SharedString>>;

pub struct Inspector {
    fields: Vec<(Field, Entity<InputState>)>,
    pub color: Entity<ColorPickerState>,
    pub family: Entity<SelectState<FamilyList>>,
    pub face: Entity<SelectState<Vec<FaceItem>>>,
    /// Embedded families the family picker lists; `None` until the fonts
    /// load.
    pub(crate) families_of: Option<Vec<String>>,
    /// Family whose faces the face picker lists, and its embedded faces.
    faces_of: Option<(String, Vec<FontFace>)>,
    /// Text element the fields show. A field left by clicking another
    /// element commits to this one, not to the new selection.
    shown: Option<ElementId>,
    /// Elements whose union the position fields show when the selection is
    /// a group or several elements.
    shown_group: Vec<ElementId>,
    /// Fields the author typed into and has not committed yet.
    dirty: Vec<Field>,
    _subscriptions: Vec<Subscription>,
}

impl Inspector {
    pub fn new(window: &mut Window, cx: &mut Context<EditorView>) -> Self {
        let mut subscriptions = Vec::new();
        let fields = Field::ALL
            .into_iter()
            .map(|field| {
                let input = cx.new(|cx| InputState::new(window, cx));
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this: &mut EditorView, _, event: &InputEvent, window, cx| match event {
                        // Only typing emits changes; values set by sync do not.
                        InputEvent::Change => {
                            if !this.inspector.dirty.contains(&field) {
                                this.inspector.dirty.push(field);
                            }
                        }
                        InputEvent::PressEnter { .. } | InputEvent::Blur => {
                            this.commit_field(field, window, cx);
                        }
                        InputEvent::Focus => {}
                    },
                ));
                (field, input)
            })
            .collect();

        let color = cx.new(|cx| ColorPickerState::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &color,
            window,
            |this: &mut EditorView, _, event: &ColorPickerEvent, _, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                this.set_style(TextStylePatch {
                    color: Some(to_rgb(*color)),
                    ..Default::default()
                });
                cx.notify();
            },
        ));

        let family = cx.new(|cx| {
            SelectState::new(
                FamilyList::new(Vec::<SearchableGroup<SharedString>>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        subscriptions.push(cx.subscribe_in(
            &family,
            window,
            |this: &mut EditorView, _, event: &SelectEvent<FamilyList>, _, cx| {
                let SelectEvent::Confirm(Some(name)) = event else {
                    return;
                };
                this.set_family(name);
                cx.notify();
            },
        ));

        let face = cx.new(|cx| SelectState::new(Vec::<FaceItem>::new(), None, window, cx));
        subscriptions.push(cx.subscribe_in(
            &face,
            window,
            |this: &mut EditorView, _, event: &SelectEvent<Vec<FaceItem>>, _, cx| {
                let SelectEvent::Confirm(Some(face)) = event else {
                    return;
                };
                this.set_font(face.clone());
                cx.notify();
            },
        ));

        Self {
            fields,
            color,
            family,
            face,
            families_of: None,
            faces_of: None,
            shown: None,
            shown_group: Vec::new(),
            dirty: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn input(&self, field: Field) -> &Entity<InputState> {
        &self
            .fields
            .iter()
            .find(|(candidate, _)| *candidate == field)
            .expect("every field has an input")
            .1
    }
}

fn to_rgb(color: Hsla) -> Rgb {
    let rgba = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;
    Rgb(channel(rgba.r) << 16 | channel(rgba.g) << 8 | channel(rgba.b))
}

impl EditorView {
    /// The selected text element's id, frame, style and sizing.
    pub fn selected_text(&self) -> Option<(ElementId, Frame, TextStyle, TextSizing)> {
        self.text_parts(self.single_selection()?)
    }

    /// The opacity of the element itself, 1 for an unknown one.
    fn opacity_of(&self, id: ElementId) -> f32 {
        self.presentation
            .element(id)
            .map_or(1., |element| element.opacity)
    }

    fn text_parts(&self, id: ElementId) -> Option<(ElementId, Frame, TextStyle, TextSizing)> {
        let element = self.presentation.element(id)?;
        let text = element.as_text()?;
        Some((id, element.frame, text.style.clone(), text.sizing))
    }

    /// Updates the fields that are not being typed into to show the selected
    /// text. Runs before every render.
    pub fn sync_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _span = crate::perf::span("sync_inspector");
        // A field left for the canvas renders before it reports the blur:
        // commit what was typed before showing the new selection over it.
        for field in self.inspector.dirty.clone() {
            let focused = self
                .inspector
                .input(field)
                .read(cx)
                .focus_handle(cx)
                .is_focused(window);
            if !focused {
                self.commit_field(field, window, cx);
            }
        }
        let Some((id, frame, style, _)) = self.selected_text() else {
            self.inspector.shown = None;
            self.inspector.shown_group = self.selection_roots();
            if let Some(frame) = self.selection_box() {
                let style = TextStyle::default();
                let rotation = self.group_rotation_label();
                let opacity = self.group_opacity_label();
                for (field, input) in &self.inspector.fields {
                    if !field.is_position() && *field != Field::Opacity {
                        continue;
                    }
                    let shown = match field {
                        Field::Rotation => rotation.clone(),
                        Field::Opacity => opacity.clone(),
                        _ => field.show(&frame, &style, 1.),
                    };
                    let state = input.read(cx);
                    if !state.focus_handle(cx).is_focused(window) && state.value() != shown.as_str()
                    {
                        input.update(cx, |state, cx| state.set_value(shown, window, cx));
                    }
                }
            }
            return;
        };
        self.inspector.shown = Some(id);
        self.inspector.shown_group.clear();
        // While the element is dragged, the fields follow the preview.
        let frame = self.shown_frame(id).unwrap_or(frame);
        let opacity = self.opacity_of(id);
        for (field, input) in &self.inspector.fields {
            let shown = field.show(&frame, &style, opacity);
            let state = input.read(cx);
            if !state.focus_handle(cx).is_focused(window) && state.value() != shown.as_str() {
                input.update(cx, |state, cx| state.set_value(shown, window, cx));
            }
        }

        let color: Hsla = gpui_kit::rgb(style.color.0).into();
        if self
            .inspector
            .color
            .read(cx)
            .value()
            .is_none_or(|current| to_rgb(current) != style.color)
        {
            self.inspector
                .color
                .update(cx, |state, cx| state.set_value(color, window, cx));
        }

        // The fonts load in the background; the pickers fill in when they
        // are ready, so the editor never waits for them.
        let Some(catalog) = fonts::catalog_ready() else {
            return;
        };
        let embedded = embedded_families(&self.presentation);
        if self.inspector.families_of.as_ref() != Some(&embedded) {
            let others = catalog
                .families()
                .iter()
                .filter(|family| !embedded.contains(&family.name))
                .map(|family| SharedString::from(family.name.clone()));
            let groups = vec![
                SearchableGroup::new("In this presentation")
                    .items(embedded.iter().cloned().map(SharedString::from)),
                SearchableGroup::new("All fonts").items(others),
            ];
            self.inspector.families_of = Some(embedded);
            self.inspector.family.update(cx, |state, cx| {
                state.set_items(FamilyList::new(groups), window, cx);
            });
        }
        let family = SharedString::from(style.font.family.clone());
        if self.inspector.family.read(cx).selected_value() != Some(&family) {
            self.inspector.family.update(cx, |state, cx| {
                state.set_selected_value(&family, window, cx)
            });
        }

        let embedded_faces = embedded_faces(&self.presentation, &style.font.family);
        let faces_of = (style.font.family.clone(), embedded_faces);
        if self.inspector.faces_of.as_ref() != Some(&faces_of) {
            let mut items: Vec<FaceItem> = catalog
                .family(&style.font.family)
                .map(|family| {
                    family
                        .faces
                        .iter()
                        .filter(|face| !faces_of.1.contains(face))
                        .map(|face| FaceItem::new(face.clone(), catalog.is_restricted(face)))
                        .collect()
                })
                .unwrap_or_default();
            // Faces embedded in the file, uploaded or not installed here.
            items.extend(
                faces_of
                    .1
                    .iter()
                    .map(|face| FaceItem::new(face.clone(), false)),
            );
            items.sort_by_key(|item| (item.face.italic, item.face.weight));
            self.inspector.faces_of = Some(faces_of);
            self.inspector
                .face
                .update(cx, |state, cx| state.set_items(items, window, cx));
        }
        if self.inspector.face.read(cx).selected_value() != Some(&style.font) {
            self.inspector.face.update(cx, |state, cx| {
                state.set_selected_value(&style.font, window, cx)
            });
        }
    }

    /// Commits what the author typed into a field. Unreadable or unchanged
    /// input is replaced by the current value.
    pub fn commit_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        self.inspector.dirty.retain(|dirty| *dirty != field);
        let Some((id, frame, style, sizing)) =
            self.inspector.shown.and_then(|id| self.text_parts(id))
        else {
            self.commit_group_field(field, window, cx);
            return;
        };
        let input = self.inspector.input(field).clone();
        let typed = input.read(cx).value().to_string();
        if typed == field.show(&frame, &style, self.opacity_of(id)) {
            return;
        }
        if let Some((label, operation)) = field_edit(field, &typed, id, &frame, &style, sizing) {
            let selection = self.selection.clone();
            self.commit(label, operation, vec![id]);
            self.selection = selection;
        }
        // Show the value the document ended up with.
        if let Some((id, frame, style, _)) = self.selected_text() {
            let shown = field.show(&frame, &style, self.opacity_of(id));
            input.update(cx, |state, cx| state.set_value(shown, window, cx));
        }
        cx.notify();
    }

    /// What the rotation field shows for a group or several elements: the
    /// angle of the group, the angle the elements share, or "Mixed".
    fn group_rotation_label(&self) -> String {
        let mut angles = self
            .selection_roots()
            .into_iter()
            .filter_map(|id| self.shown_frame(id))
            .map(|frame| frame.rotation);
        let Some(first) = angles.next() else {
            return String::new();
        };
        if angles.all(|angle| angle == first) {
            format!("{}°", number(first))
        } else {
            "Mixed".into()
        }
    }

    /// What the opacity field shows for a group or several elements: the
    /// opacity they share, or "Mixed".
    fn group_opacity_label(&self) -> String {
        let opacities = self
            .selection_roots()
            .into_iter()
            .map(|id| self.opacity_of(id));
        match crate::ui::shape_inspector::common(opacities) {
            Some(opacity) => format!("{}%", number(opacity * 100.)),
            None if self.selection.is_empty() => String::new(),
            None => "Mixed".into(),
        }
    }

    /// Commits a position field typed for a group or several elements: they
    /// move, scale to the typed size, or each turn to the typed angle around
    /// its own center. The opacity field sets the opacity of each.
    fn commit_group_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.inspector.shown_group.clone();
        if field == Field::Opacity {
            let input = self.inspector.input(field).clone();
            let typed = input.read(cx).value().to_string();
            if typed != self.group_opacity_label()
                && let Some(value) = parse(&typed).filter(|value| (0. ..=100.).contains(value))
            {
                let opacity = value / 100.;
                let operations: Vec<Operation> = ids
                    .iter()
                    .filter(|id| self.opacity_of(**id) != opacity)
                    .map(|id| Operation::SetLayer {
                        id: *id,
                        patch: LayerPatch {
                            opacity: Some(opacity),
                            ..Default::default()
                        },
                    })
                    .collect();
                if !operations.is_empty() {
                    let selection = self.selection.clone();
                    self.commit_pruning("Opacity", operations, selection);
                }
            }
            let shown = self.group_opacity_label();
            input.update(cx, |state, cx| state.set_value(shown, window, cx));
            cx.notify();
            return;
        }
        let frames: Vec<Frame> = ids.iter().filter_map(|id| self.frame_of(*id)).collect();
        // The box the fields show: the frame of one element, the union of
        // several.
        let from = match frames.as_slice() {
            [frame] => *frame,
            frames => match crate::document::union(frames) {
                Some(frame) => frame,
                None => return,
            },
        };
        let input = self.inspector.input(field).clone();
        let typed = input.read(cx).value().to_string();
        let style = TextStyle::default();
        if field == Field::Rotation {
            if let Some(value) = parse(&typed) {
                let rotation = crate::document::normalize_degrees(value);
                let operations: Vec<Operation> = ids
                    .iter()
                    .zip(&frames)
                    .filter(|(_, frame)| frame.rotation != rotation)
                    .map(|(id, frame)| Operation::SetFrame {
                        id: *id,
                        frame: Frame { rotation, ..*frame },
                    })
                    .collect();
                if !operations.is_empty() {
                    let selection = self.selection.clone();
                    self.commit_pruning("Rotate", operations, selection);
                }
            }
            let shown = self.group_rotation_label();
            input.update(cx, |state, cx| state.set_value(shown, window, cx));
            cx.notify();
            return;
        }
        if let Some(value) = parse(&typed).filter(|_| field.is_position()) {
            let mut to = from;
            match field {
                Field::X => to.x = value,
                Field::Y => to.y = value,
                Field::Width => to.width = value.max(crate::snap::MIN_SIZE),
                _ => to.height = value.max(crate::snap::MIN_SIZE),
            }
            let operations = self.transform_operations(&ids, from, to);
            if !operations.is_empty() {
                let label = if matches!(field, Field::X | Field::Y) {
                    "Move"
                } else {
                    "Resize"
                };
                let selection = self.selection.clone();
                self.commit_pruning(label, operations, selection);
            }
        }
        if let Some(frame) = self.selection_box() {
            let shown = field.show(&frame, &style, 1.);
            input.update(cx, |state, cx| state.set_value(shown, window, cx));
        }
        cx.notify();
    }

    /// Changes style fields of the selected text as one undo step.
    pub fn set_style(&mut self, patch: TextStylePatch) {
        if let Some((id, ..)) = self.selected_text() {
            let label = patch.label();
            self.commit(label, Operation::SetTextStyle { id, patch }, vec![id]);
        }
    }

    /// Switches the selected text to a sizing mode.
    pub fn set_sizing(&mut self, sizing: TextSizing) {
        if let Some((id, _, _, current)) = self.selected_text()
            && current != sizing
        {
            self.commit(
                "Resizing",
                Operation::SetTextSizing { id, sizing },
                vec![id],
            );
        }
    }

    /// Moves the selection to an edge or the center of the slide.
    pub fn align_to_slide(&mut self, horizontal: Option<f32>, vertical: Option<f32>) {
        let ids = self.selection_roots();
        let Some(frame) = self.selection_frame() else {
            return;
        };
        let size = self.presentation.size;
        let mut aligned = frame;
        if let Some(t) = horizontal {
            aligned.x = (size.width as f32 - frame.width) * t;
        }
        if let Some(t) = vertical {
            aligned.y = (size.height as f32 - frame.height) * t;
        }
        let operations = self.transform_operations(&ids, frame, aligned);
        if !operations.is_empty() {
            let selection = self.selection.clone();
            self.commit_pruning("Align", operations, selection);
        }
    }

    /// Uses a face for the selected text, embedding it first when needed.
    pub fn set_font(&mut self, face: FontFace) {
        let Some((id, _, style, _)) = self.selected_text() else {
            return;
        };
        if style.font == face {
            return;
        }
        let mut operations = Vec::new();
        if !self.presentation.fonts.contains(&face) {
            if fonts::catalog().is_restricted(&face) {
                return;
            }
            let Some(data) = fonts::data(&face) else {
                eprintln!("sliderino: font {face:?} is not available");
                return;
            };
            operations.push(Operation::AddFont {
                face: face.clone(),
                data,
            });
        }
        operations.push(Operation::SetTextStyle {
            id,
            patch: TextStylePatch {
                font: Some(face),
                ..Default::default()
            },
        });
        self.commit("Font", Operation::Batch(operations), vec![id]);
    }

    /// Uses another family for the selected text, keeping the face closest
    /// to the current weight and slant.
    pub fn set_family(&mut self, name: &str) {
        let Some((_, _, style, _)) = self.selected_text() else {
            return;
        };
        let catalog = fonts::catalog();
        let embedded = embedded_faces(&self.presentation, name);
        let installed = catalog.family(name).map_or(&[][..], |family| &family.faces);
        let allowed: Vec<FontFace> = installed
            .iter()
            .filter(|face| !embedded.contains(face) && !catalog.is_restricted(face))
            .cloned()
            .chain(embedded.iter().cloned())
            .collect();
        if let Some(face) = fonts::closest_face(&allowed, style.font.weight, style.font.italic) {
            self.set_font(face.clone());
        }
    }
}

/// The edit a field makes, labeled for the history; None when the input is
/// not a valid value.
fn field_edit(
    field: Field,
    typed: &str,
    id: ElementId,
    frame: &Frame,
    style: &TextStyle,
    sizing: TextSizing,
) -> Option<(&'static str, Operation)> {
    let patch = |patch: TextStylePatch| Operation::SetTextStyle { id, patch };
    if field == Field::LineHeight
        && (typed.trim().is_empty() || typed.trim().eq_ignore_ascii_case("auto"))
    {
        let edit = patch(TextStylePatch {
            line_height: Some(LineHeight::Auto),
            ..Default::default()
        });
        return (style.line_height != LineHeight::Auto).then_some(("Line height", edit));
    }
    let value = parse(typed)?;
    let edit = match field {
        Field::X => (
            "Move",
            Operation::SetFrame {
                id,
                frame: Frame { x: value, ..*frame },
            },
        ),
        Field::Y => (
            "Move",
            Operation::SetFrame {
                id,
                frame: Frame { y: value, ..*frame },
            },
        ),
        Field::Width => {
            let mut operations = Vec::new();
            if sizing == TextSizing::AutoWidth {
                operations.push(Operation::SetTextSizing {
                    id,
                    sizing: TextSizing::AutoHeight,
                });
            }
            operations.push(Operation::SetFrame {
                id,
                frame: Frame {
                    width: value.max(crate::snap::MIN_SIZE),
                    ..*frame
                },
            });
            ("Resize", Operation::Batch(operations))
        }
        Field::Rotation => (
            "Rotate",
            Operation::SetFrame {
                id,
                frame: Frame {
                    rotation: crate::document::normalize_degrees(value),
                    ..*frame
                },
            },
        ),
        Field::Height => {
            let mut operations = Vec::new();
            if sizing != TextSizing::Fixed {
                operations.push(Operation::SetTextSizing {
                    id,
                    sizing: TextSizing::Fixed,
                });
            }
            operations.push(Operation::SetFrame {
                id,
                frame: Frame {
                    height: value.max(crate::snap::MIN_SIZE),
                    ..*frame
                },
            });
            ("Resize", Operation::Batch(operations))
        }
        Field::Size if value > 0. => (
            "Font size",
            patch(TextStylePatch {
                size: Some(value),
                ..Default::default()
            }),
        ),
        Field::LineHeight if value > 0. => (
            "Line height",
            patch(TextStylePatch {
                line_height: Some(LineHeight::Percent(value)),
                ..Default::default()
            }),
        ),
        Field::LetterSpacing => (
            "Letter spacing",
            patch(TextStylePatch {
                letter_spacing: Some(value),
                ..Default::default()
            }),
        ),
        Field::ParagraphSpacing if value >= 0. => (
            "Paragraph spacing",
            patch(TextStylePatch {
                paragraph_spacing: Some(value),
                ..Default::default()
            }),
        ),
        Field::Opacity if (0. ..=100.).contains(&value) => (
            "Opacity",
            Operation::SetLayer {
                id,
                patch: LayerPatch {
                    opacity: Some(value / 100.),
                    ..Default::default()
                },
            },
        ),
        _ => return None,
    };
    Some(edit)
}

/// The families embedded in the presentation, sorted.
pub fn embedded_families(presentation: &Presentation) -> Vec<String> {
    let mut families: Vec<String> = presentation
        .fonts
        .faces()
        .map(|face| face.family.clone())
        .collect();
    families.dedup();
    families
}

/// The embedded faces of `family`, sorted.
fn embedded_faces(presentation: &Presentation, family: &str) -> Vec<FontFace> {
    presentation
        .fonts
        .faces()
        .filter(|face| face.family == family)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_show_at_most_two_decimals() {
        assert_eq!(number(56.), "56");
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(1. / 3.), "0.33");
        assert_eq!(number(-0.001), "-0");
        assert_eq!(parse(" 120 % "), Some(120.));
        assert_eq!(parse("abc"), None);
        assert_eq!(parse("-45°"), Some(-45.));
    }

    #[test]
    fn typing_a_width_makes_auto_width_text_wrap() {
        let frame = Frame {
            width: 100.,
            height: 40.,
            ..Frame::default()
        };
        let style = TextStyle::default();
        let (_, edit) = field_edit(
            Field::Width,
            "300",
            ElementId(1),
            &frame,
            &style,
            TextSizing::AutoWidth,
        )
        .unwrap();
        let Operation::Batch(operations) = edit else {
            panic!("a batch");
        };
        assert_eq!(
            operations[0],
            Operation::SetTextSizing {
                id: ElementId(1),
                sizing: TextSizing::AutoHeight
            }
        );
        assert!(
            field_edit(
                Field::Size,
                "-4",
                ElementId(1),
                &frame,
                &style,
                TextSizing::Fixed
            )
            .is_none()
        );
        assert!(
            field_edit(
                Field::LineHeight,
                "auto",
                ElementId(1),
                &frame,
                &style,
                TextSizing::Fixed
            )
            .is_none(),
            "already Auto"
        );
    }
}
