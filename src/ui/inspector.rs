//! State of the inspector's editable fields and the edits they make.
//!
//! The fields show the selected text box. While a field has focus it keeps
//! what the author types; Enter or leaving the field commits it as one undo
//! step. Otherwise the fields follow the document, including changes made by
//! undo or by other editors.

use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, Hsla, SharedString, Subscription, Window,
};

use crate::document::{
    ElementId, FontFace, Frame, LineHeight, Operation, Rgb, TextSizing, TextStyle, TextStylePatch,
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
    Size,
    LineHeight,
    LetterSpacing,
    ParagraphSpacing,
    Opacity,
}

impl Field {
    const ALL: [Field; 9] = [
        Field::X,
        Field::Y,
        Field::Width,
        Field::Height,
        Field::Size,
        Field::LineHeight,
        Field::LetterSpacing,
        Field::ParagraphSpacing,
        Field::Opacity,
    ];

    /// The field's text for a frame and style.
    fn show(self, frame: &Frame, style: &TextStyle) -> String {
        match self {
            Field::X => number(frame.x),
            Field::Y => number(frame.y),
            Field::Width => number(frame.width),
            Field::Height => number(frame.height),
            Field::Size => number(style.size),
            Field::LineHeight => match style.line_height {
                LineHeight::Auto => "Auto".into(),
                LineHeight::Percent(percent) => format!("{}%", number(percent)),
            },
            Field::LetterSpacing => format!("{}%", number(style.letter_spacing)),
            Field::ParagraphSpacing => number(style.paragraph_spacing),
            Field::Opacity => format!("{}%", number(style.opacity * 100.)),
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

/// Parses what the author typed: a number, optionally followed by "%".
fn parse(text: &str) -> Option<f32> {
    let value: f32 = text.trim().trim_end_matches('%').trim().parse().ok()?;
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

pub type FamilyList = SearchableVec<SharedString>;

pub struct Inspector {
    fields: Vec<(Field, Entity<InputState>)>,
    pub color: Entity<ColorPickerState>,
    pub family: Entity<SelectState<FamilyList>>,
    pub face: Entity<SelectState<Vec<FaceItem>>>,
    families_loaded: bool,
    /// Family whose faces the face picker lists.
    faces_of: Option<String>,
    /// Element the fields show. A field left by clicking another element
    /// commits to this one, not to the new selection.
    shown: Option<ElementId>,
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
                FamilyList::new(Vec::<SharedString>::new()),
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
            families_loaded: false,
            faces_of: None,
            shown: None,
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
        self.text_parts(self.selection?)
    }

    fn text_parts(&self, id: ElementId) -> Option<(ElementId, Frame, TextStyle, TextSizing)> {
        let element = self.presentation.element(id)?;
        let text = element.as_text()?;
        Some((id, element.frame, text.style.clone(), text.sizing))
    }

    /// Updates the fields that are not being typed into to show the selected
    /// text. Runs before every render.
    pub fn sync_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            return;
        };
        self.inspector.shown = Some(id);
        for (field, input) in &self.inspector.fields {
            let shown = field.show(&frame, &style);
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

        let catalog = fonts::catalog();
        if !self.inspector.families_loaded {
            self.inspector.families_loaded = true;
            let names: Vec<SharedString> = catalog
                .families()
                .iter()
                .map(|family| SharedString::from(family.name.clone()))
                .collect();
            self.inspector.family.update(cx, |state, cx| {
                state.set_items(FamilyList::new(names), window, cx);
            });
        }
        let family = SharedString::from(style.font.family.clone());
        if self.inspector.family.read(cx).selected_value() != Some(&family) {
            self.inspector.family.update(cx, |state, cx| {
                state.set_selected_value(&family, window, cx)
            });
        }

        if self.inspector.faces_of.as_deref() != Some(style.font.family.as_str()) {
            self.inspector.faces_of = Some(style.font.family.clone());
            let mut items: Vec<FaceItem> = catalog
                .family(&style.font.family)
                .map(|family| {
                    family
                        .faces
                        .iter()
                        .map(|face| FaceItem::new(face.clone(), catalog.is_restricted(face)))
                        .collect()
                })
                .unwrap_or_default();
            // A face embedded in the file may not be installed here.
            if !items.iter().any(|item| item.face == style.font) {
                items.push(FaceItem::new(style.font.clone(), false));
            }
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
            return;
        };
        let input = self.inspector.input(field).clone();
        let typed = input.read(cx).value().to_string();
        if typed == field.show(&frame, &style) {
            return;
        }
        if let Some((label, operation)) = field_edit(field, &typed, id, &frame, &style, sizing) {
            let selection = self.selection;
            self.commit(label, operation, Some(id));
            self.selection = selection;
        }
        // Show the value the document ended up with.
        if let Some((_, frame, style, _)) = self.selected_text() {
            let shown = field.show(&frame, &style);
            input.update(cx, |state, cx| state.set_value(shown, window, cx));
        }
        cx.notify();
    }

    /// Changes style fields of the selected text as one undo step.
    pub fn set_style(&mut self, patch: TextStylePatch) {
        if let Some((id, ..)) = self.selected_text() {
            let label = patch.label();
            self.commit(label, Operation::SetTextStyle { id, patch }, Some(id));
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
                Some(id),
            );
        }
    }

    /// Moves the selection to an edge or the center of the slide.
    pub fn align_to_slide(&mut self, horizontal: Option<f32>, vertical: Option<f32>) {
        let Some((id, frame, ..)) = self.selected_text() else {
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
        if aligned != frame {
            self.commit(
                "Align",
                Operation::SetFrame { id, frame: aligned },
                Some(id),
            );
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
        self.commit("Font", Operation::Batch(operations), Some(id));
    }

    /// Uses another family for the selected text, keeping the face closest
    /// to the current weight and slant.
    pub fn set_family(&mut self, name: &str) {
        let Some((_, _, style, _)) = self.selected_text() else {
            return;
        };
        let catalog = fonts::catalog();
        let Some(family) = catalog.family(name) else {
            return;
        };
        let allowed: Vec<FontFace> = family
            .faces
            .iter()
            .filter(|face| !catalog.is_restricted(face))
            .cloned()
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
            "Text opacity",
            patch(TextStylePatch {
                opacity: Some(value / 100.),
                ..Default::default()
            }),
        ),
        _ => return None,
    };
    Some(edit)
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
