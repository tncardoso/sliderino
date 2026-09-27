//! Hierarchy tab of the left panel: the layers of the current slide as a
//! tree, topmost first, like the layers panel of Figma. Rows select, show or
//! hide, lock, rename and reorder layers; dragging a row into a group moves
//! the layer into it.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled, Subscription,
    TestSupportExt as _, WeakEntity, Window, div, px,
};

use crate::document::{Element, ElementId, LayerPatch, Operation, Presentation};
use crate::editor::EditorView;
use crate::theme;
use crate::ui::properties_panel::{element_icon, element_name};

/// Indent of one level of the tree.
const INDENT: f32 = 16.;
const ROW_HEIGHT: f32 = 28.;

/// Where a dragged layer lands relative to a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    /// Above the row: over the layer in paint order.
    Before,
    /// Below the row: under the layer.
    After,
    /// Inside the group of the row, on top of its children.
    Into,
}

/// A rename in progress: the layer and the field typed into.
pub struct Renaming {
    pub id: ElementId,
    pub input: Entity<InputState>,
    _subscription: Subscription,
}

/// The layers dragged in the hierarchy.
#[derive(Clone, Debug)]
pub struct DraggedLayers {
    pub ids: Vec<ElementId>,
    label: SharedString,
}

/// The pill that follows the pointer while layers or slides are dragged.
pub struct DragPreview {
    pub label: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(theme::background())
            .border_1()
            .border_color(theme::accent())
            .text_size(px(12.))
            .text_color(theme::text())
            .child(self.label.clone())
    }
}

/// One row of the tree.
struct Row {
    id: ElementId,
    depth: usize,
    name: String,
    icon: IconName,
    group: bool,
    expanded: bool,
    /// The layer's own flags, shown by the eye and the lock.
    hidden: bool,
    locked: bool,
    /// Hidden by itself or an ancestor: the row is faded.
    faded: bool,
    selected: bool,
    /// An ancestor is selected.
    inside_selection: bool,
}

impl EditorView {
    /// Rows of the current slide, topmost first, without the children of
    /// collapsed groups.
    fn hierarchy_rows(&self) -> Vec<Row> {
        fn visit(
            editor: &EditorView,
            elements: &[Element],
            depth: usize,
            faded: bool,
            inside_selection: bool,
            rows: &mut Vec<Row>,
        ) {
            for element in elements.iter().rev() {
                let group = element.as_group().is_some();
                let expanded = group && !editor.collapsed.contains(&element.id);
                let selected = editor.selection.contains(&element.id);
                let faded = faded || element.hidden;
                rows.push(Row {
                    id: element.id,
                    depth,
                    name: element_name(element),
                    icon: element_icon(element),
                    group,
                    expanded,
                    hidden: element.hidden,
                    locked: element.locked,
                    faded,
                    selected,
                    inside_selection,
                });
                if expanded {
                    visit(
                        editor,
                        element.children(),
                        depth + 1,
                        faded,
                        inside_selection || selected,
                        rows,
                    );
                }
            }
        }
        let mut rows = Vec::new();
        visit(
            self,
            &self.current_slide().elements,
            0,
            false,
            false,
            &mut rows,
        );
        rows
    }

    /// Selects a layer from a click on its row: Shift selects the visible
    /// rows from the last clicked one, Ctrl adds or removes one row.
    pub fn click_layer(&mut self, id: ElementId, shift: bool, toggle: bool) {
        self.end_text_edit();
        if shift && let Some(anchor) = self.tree_anchor {
            let rows: Vec<ElementId> = self.hierarchy_rows().iter().map(|row| row.id).collect();
            if let (Some(a), Some(b)) = (
                rows.iter().position(|row| *row == anchor),
                rows.iter().position(|row| *row == id),
            ) {
                let range = a.min(b)..=a.max(b);
                self.selection = rows[range].to_vec();
                return;
            }
        }
        if toggle {
            if let Some(index) = self.selection.iter().position(|selected| *selected == id) {
                self.selection.remove(index);
            } else {
                self.selection.push(id);
            }
        } else {
            self.selection = vec![id];
        }
        self.tree_anchor = Some(id);
    }

    /// Shows or hides the selected layers, as one step. Hides them unless the
    /// first one is hidden.
    pub fn toggle_hidden(&mut self) {
        let ids = self.selection_roots();
        let hide = !ids
            .first()
            .and_then(|id| self.presentation.element(*id))
            .is_some_and(|element| element.hidden);
        self.set_layers(
            &ids,
            LayerPatch {
                hidden: Some(hide),
                ..LayerPatch::default()
            },
        );
    }

    /// Locks or unlocks the selected layers, as one step.
    pub fn toggle_locked(&mut self) {
        let ids = self.selection_roots();
        let lock = !ids
            .first()
            .and_then(|id| self.presentation.element(*id))
            .is_some_and(|element| element.locked);
        self.set_layers(
            &ids,
            LayerPatch {
                locked: Some(lock),
                ..LayerPatch::default()
            },
        );
    }

    /// Changes layer fields of `ids` as one undo step.
    pub fn set_layers(&mut self, ids: &[ElementId], patch: LayerPatch) {
        if ids.is_empty() {
            return;
        }
        let label = patch.label();
        let operations = ids
            .iter()
            .map(|id| Operation::SetLayer {
                id: *id,
                patch: patch.clone(),
            })
            .collect();
        let selection = self.selection.clone();
        self.commit_pruning(label, operations, selection);
    }

    /// Opens the name field on the row of a layer, in the Hierarchy tab.
    pub fn start_rename(&mut self, id: ElementId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(element) = self.presentation.element(id) else {
            return;
        };
        let name = element_name(element);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.finish_rename(true, window, cx);
                }
            },
        );
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.library_tab = 2;
        self.renaming = Some(Renaming {
            id,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Closes the name field, renaming the layer when `commit` is set. An
    /// empty name brings back the name made from the content.
    pub fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        if commit && let Some(element) = self.presentation.element(renaming.id) {
            let typed = renaming.input.read(cx).value().trim().to_string();
            let unchanged = match &element.name {
                Some(name) => *name == typed,
                None => typed == element_name(element) || typed.is_empty(),
            };
            if !unchanged {
                self.set_layers(
                    &[renaming.id],
                    LayerPatch {
                        name: Some(Some(typed)),
                        ..LayerPatch::default()
                    },
                );
            }
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Moves the dragged layers to the drop place recorded while dragging.
    fn drop_layers(&mut self, dragged: &DraggedLayers) {
        let Some((row, zone)) = self.layer_drop.take() else {
            return;
        };
        if dragged
            .ids
            .iter()
            .any(|id| self.presentation.is_locked(*id))
        {
            return;
        }
        let zone = self.effective_zone(row, zone);
        if let Some(operations) = layer_moves(&self.presentation, &dragged.ids, row, zone) {
            self.commit_pruning("Move layer", operations, dragged.ids.clone());
        }
    }

    /// Below an expanded group with children, the next row is its topmost
    /// child: a drop there lands inside the group.
    fn effective_zone(&self, row: ElementId, zone: DropZone) -> DropZone {
        let expanded_group = self
            .presentation
            .element(row)
            .and_then(Element::as_group)
            .is_some_and(|group| !group.children.is_empty())
            && !self.collapsed.contains(&row);
        if zone == DropZone::After && expanded_group {
            DropZone::Into
        } else {
            zone
        }
    }
}

/// The operations that move `dragged` to `zone` of the layer `row`, keeping
/// their paint order, or `None` when the place is inside a dragged layer.
pub fn layer_moves(
    presentation: &Presentation,
    dragged: &[ElementId],
    row: ElementId,
    zone: DropZone,
) -> Option<Vec<Operation>> {
    let target = presentation.locate(row)?;
    let (parent, siblings): (Option<ElementId>, Vec<ElementId>) = match zone {
        DropZone::Into => {
            let group = presentation.element(row)?.as_group()?;
            (Some(row), group.children.iter().map(|e| e.id).collect())
        }
        DropZone::Before | DropZone::After => {
            let slide = presentation.slide(target.slide)?;
            let siblings = match target.parent {
                None => &slide.elements,
                Some(parent) => &presentation.element(parent)?.as_group()?.children,
            };
            (target.parent, siblings.iter().map(|e| e.id).collect())
        }
    };
    if let Some(parent) = parent
        && (dragged.contains(&parent)
            || presentation
                .ancestors(parent)
                .iter()
                .any(|ancestor| dragged.contains(ancestor)))
    {
        return None;
    }
    if zone != DropZone::Into && dragged.contains(&row) {
        return None;
    }
    // Siblings that stay under the dropped layers.
    let below = match zone {
        DropZone::Into => siblings.len(),
        DropZone::Before => target.index + 1,
        DropZone::After => target.index,
    };
    let under = siblings[..below]
        .iter()
        .filter(|id| !dragged.contains(id))
        .count();
    // Layers in paint order, bottom first, all on the slide of the target.
    let slide = presentation.slide(target.slide)?;
    let order: Vec<ElementId> = slide
        .walk()
        .iter()
        .map(|node| node.element.id)
        .filter(|id| dragged.contains(id))
        .collect();
    if order.len() != dragged.len() {
        return None;
    }
    // Replays the moves on the list of the destination's children.
    let mut children = siblings;
    let mut operations = Vec::with_capacity(order.len());
    let mut last: Option<ElementId> = None;
    for id in order {
        if let Some(index) = children.iter().position(|child| *child == id) {
            children.remove(index);
        }
        let index = match last {
            Some(last) => children.iter().position(|child| *child == last)? + 1,
            None => {
                let mut seen = 0;
                let mut index = 0;
                for (position, child) in children.iter().enumerate() {
                    if seen == under {
                        break;
                    }
                    if !dragged.contains(child) {
                        seen += 1;
                    }
                    index = position + 1;
                }
                if seen < under { children.len() } else { index }
            }
        };
        children.insert(index, id);
        operations.push(Operation::MoveElement { id, parent, index });
        last = Some(id);
    }
    Some(operations)
}

/// The Hierarchy tab: the tree of the current slide.
pub fn hierarchy_panel(editor: &mut EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    let _span = crate::perf::span("hierarchy_panel");
    let rows = editor.hierarchy_rows();
    let dragging = cx.has_active_drag();
    let drop = editor.layer_drop.filter(|_| dragging);
    let weak = cx.entity().downgrade();
    let children: Vec<_> = rows
        .into_iter()
        .map(|row| layer_row(editor, row, drop, cx))
        .collect();
    let empty = children.is_empty();
    v_flex()
        .id("hierarchy")
        .test_support()
        .flex_1()
        .min_h_0()
        .py(px(6.))
        .overflow_y_scrollbar()
        .on_drag_move::<DraggedLayers>(cx.listener(
            |this, event: &gpui_kit::DragMoveEvent<DraggedLayers>, _, cx| {
                if !event.bounds.contains(&event.event.position) && this.layer_drop.take().is_some()
                {
                    cx.notify();
                }
            },
        ))
        .children(children)
        .when(empty, |this| {
            this.child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("No layers on this slide"),
            )
        })
        .context_menu(move |menu, _, cx| layer_menu(menu, &weak, cx))
}

fn layer_row(
    editor: &EditorView,
    row: Row,
    drop: Option<(ElementId, DropZone)>,
    cx: &mut Context<EditorView>,
) -> gpui_kit::AnyElement {
    let id = row.id;
    let dragged = if row.selected {
        editor.selection_roots()
    } else {
        vec![id]
    };
    let label: SharedString = if dragged.len() > 1 {
        format!("{} layers", dragged.len()).into()
    } else {
        row.name.clone().into()
    };
    let payload = DraggedLayers {
        ids: dragged,
        label,
    };
    let renaming = editor
        .renaming
        .as_ref()
        .filter(|renaming| renaming.id == id)
        .map(|renaming| renaming.input.clone());
    let zone = drop.filter(|(row, _)| *row == id).map(|(_, zone)| zone);
    let group_name = SharedString::from(format!("layer-row-{}", id.0));
    let background = if row.selected {
        theme::accent().opacity(0.16)
    } else if row.inside_selection {
        theme::accent().opacity(0.06)
    } else {
        gpui_kit::transparent_black()
    };

    let chevron = div()
        .id(("layer-chevron", id.0 as usize))
        .test_support()
        .w(px(16.))
        .flex_shrink_0()
        .flex()
        .justify_center()
        .when(row.group, |this| {
            this.child(
                Icon::new(if row.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(12.))
                .text_color(theme::text_muted()),
            )
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.collapsed.remove(&id) {
                    this.collapsed.insert(id);
                }
                cx.stop_propagation();
                cx.notify();
            }))
        });

    let toggle = |name: &'static str, icon: IconName, active: bool| {
        div()
            .id((name, id.0 as usize))
            .test_support()
            .w(px(22.))
            .h(px(22.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|style| style.bg(theme::field()))
            .when(!active, |this| {
                this.invisible()
                    .group_hover(group_name.clone(), |style| style.visible())
            })
            .child(
                Icon::new(icon)
                    .size(px(13.))
                    .text_color(theme::text_muted()),
            )
    };
    let eye = toggle(
        "layer-eye",
        if row.hidden {
            IconName::EyeOff
        } else {
            IconName::Eye
        },
        row.hidden,
    )
    .on_click(cx.listener(move |this, _, _, cx| {
        let hidden = this.presentation.element(id).is_some_and(|e| e.hidden);
        this.set_layers(
            &[id],
            LayerPatch {
                hidden: Some(!hidden),
                ..LayerPatch::default()
            },
        );
        cx.stop_propagation();
        cx.notify();
    }));
    let lock = toggle(
        "layer-lock",
        if row.locked {
            IconName::Lock
        } else {
            IconName::LockOpen
        },
        row.locked,
    )
    .on_click(cx.listener(move |this, _, _, cx| {
        let locked = this.presentation.element(id).is_some_and(|e| e.locked);
        this.set_layers(
            &[id],
            LayerPatch {
                locked: Some(!locked),
                ..LayerPatch::default()
            },
        );
        cx.stop_propagation();
        cx.notify();
    }));

    let name = match renaming {
        Some(input) => div()
            .flex_1()
            .min_w_0()
            .capture_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.finish_rename(false, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .child(Input::new(&input).xsmall())
            .into_any_element(),
        None => div()
            .flex_1()
            .min_w_0()
            .truncate()
            .child(row.name.clone())
            .into_any_element(),
    };

    h_flex()
        .id(("layer", id.0 as usize))
        .test_support()
        .group(group_name.clone())
        .relative()
        .h(px(ROW_HEIGHT))
        .flex_shrink_0()
        .pl(px(6. + row.depth as f32 * INDENT))
        .pr(px(6.))
        .gap(px(4.))
        .items_center()
        .bg(background)
        .when(!row.selected, |this| {
            this.hover(|style| style.bg(theme::field()))
        })
        .when(row.faded, |this| this.opacity(0.45))
        .text_size(px(12.))
        .text_color(theme::text())
        .child(chevron)
        .child(
            Icon::new(row.icon)
                .size(px(13.))
                .text_color(if row.selected {
                    theme::accent()
                } else {
                    theme::text_muted()
                }),
        )
        .child(name)
        .child(eye)
        .child(lock)
        .when_some(zone, |this, zone| this.child(drop_indicator(zone)))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            if *hovered {
                this.hovered_layer = Some(id);
            } else if this.hovered_layer == Some(id) {
                this.hovered_layer = None;
            }
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, _, cx| {
                if !this.selection.contains(&id) {
                    this.click_layer(id, false, false);
                    cx.notify();
                }
            }),
        )
        .on_click(
            cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                let modifiers = event.modifiers();
                this.click_layer(id, modifiers.shift, modifiers.secondary());
                window.focus(&this.focus, cx);
                cx.notify();
            }),
        )
        .on_drag(payload, |payload, _, _, cx| {
            let label = payload.label.clone();
            cx.new(|_| DragPreview { label })
        })
        .on_drag_move::<DraggedLayers>(cx.listener(
            move |this, event: &gpui_kit::DragMoveEvent<DraggedLayers>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                let group = this
                    .presentation
                    .element(id)
                    .is_some_and(|element| element.as_group().is_some());
                let top = f32::from(event.bounds.top());
                let height = f32::from(event.bounds.size.height).max(1.);
                let t = (f32::from(event.event.position.y) - top) / height;
                let zone = match (group, t) {
                    (true, t) if t < 0.25 => DropZone::Before,
                    (true, t) if t > 0.75 => DropZone::After,
                    (true, _) => DropZone::Into,
                    (false, t) if t < 0.5 => DropZone::Before,
                    (false, _) => DropZone::After,
                };
                if this.layer_drop != Some((id, zone)) {
                    this.layer_drop = Some((id, zone));
                    cx.notify();
                }
            },
        ))
        .on_drop(cx.listener(|this, dragged: &DraggedLayers, _, cx| {
            this.drop_layers(dragged);
            cx.notify();
        }))
        .into_any_element()
}

/// The line or outline showing where a dragged layer lands.
fn drop_indicator(zone: DropZone) -> impl IntoElement {
    let line = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(2.))
        .bg(theme::accent());
    match zone {
        DropZone::Before => line.top_0(),
        DropZone::After => line.bottom_0(),
        DropZone::Into => div()
            .absolute()
            .inset_0()
            .border_2()
            .border_color(theme::accent())
            .rounded(px(4.)),
    }
}

/// The menu of the selected layers, on the hierarchy and on the canvas.
pub fn layer_menu(
    menu: PopupMenu,
    editor: &WeakEntity<EditorView>,
    cx: &mut gpui_kit::App,
) -> PopupMenu {
    let Some(entity) = editor.upgrade() else {
        return menu;
    };
    let view = entity.read(cx);
    if view.selection.is_empty() {
        return menu;
    }
    let single = view.single_selection();
    let roots = view.selection_roots();
    let editable = roots.iter().any(|id| !view.presentation.is_locked(*id));
    let has_group = roots.iter().any(|id| {
        !view.presentation.is_locked(*id)
            && view
                .presentation
                .element(*id)
                .is_some_and(|element| element.as_group().is_some())
    });
    let first = roots.first().and_then(|id| view.presentation.element(*id));
    let hidden = first.is_some_and(|element| element.hidden);
    let locked = first.is_some_and(|element| element.locked);

    let item = |label: &'static str,
                enabled: bool,
                action: fn(&mut EditorView, &mut Window, &mut Context<EditorView>)| {
        let editor = editor.clone();
        PopupMenuItem::new(label)
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| {
                        action(editor, window, cx);
                        cx.notify();
                    })
                    .ok();
            })
    };
    menu.item(item("Rename", single.is_some(), |editor, window, cx| {
        if let Some(id) = editor.single_selection() {
            editor.start_rename(id, window, cx);
        }
    }))
    .separator()
    .item(item("Group  (Ctrl+G)", editable, |editor, _, _| {
        editor.group_selection()
    }))
    .item(item(
        "Ungroup  (Ctrl+Shift+G)",
        has_group,
        |editor, _, _| editor.ungroup_selection(),
    ))
    .separator()
    .item(item(
        if hidden { "Show" } else { "Hide" },
        true,
        |editor, _, _| editor.toggle_hidden(),
    ))
    .item(item(
        if locked { "Unlock" } else { "Lock" },
        true,
        |editor, _, _| editor.toggle_locked(),
    ))
    .separator()
    .item(item("Delete", editable, |editor, _, _| {
        editor.delete_selection()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::{add_text, with_inter};
    use crate::document::{Frame, TextSizing};

    /// A slide with A, B, C, D (bottom to top) and a group G = [E, F] on top.
    fn layers() -> (Presentation, Vec<ElementId>, ElementId) {
        let mut presentation = with_inter();
        let ids: Vec<ElementId> = ["A", "B", "C", "D", "E", "F"]
            .into_iter()
            .map(|name| {
                add_text(
                    &mut presentation,
                    name,
                    TextSizing::AutoWidth,
                    Frame::default(),
                )
            })
            .collect();
        let group = presentation.new_element_id();
        let operations = presentation.group_operations(group, &ids[4..]).unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
        (presentation, ids, group)
    }

    fn apply(
        presentation: &mut Presentation,
        dragged: &[ElementId],
        row: ElementId,
        zone: DropZone,
    ) {
        let operations = layer_moves(presentation, dragged, row, zone).unwrap();
        presentation.apply(Operation::Batch(operations)).unwrap();
    }

    fn top(presentation: &Presentation) -> Vec<ElementId> {
        presentation.slides[0]
            .elements
            .iter()
            .map(|e| e.id)
            .collect()
    }

    #[test]
    fn dropping_above_a_row_puts_layers_over_it() {
        let (mut presentation, ids, group) = layers();
        let [a, b, c, d] = [ids[0], ids[1], ids[2], ids[3]];
        apply(&mut presentation, &[a, b], d, DropZone::Before);
        assert_eq!(top(&presentation), [c, d, a, b, group]);
    }

    #[test]
    fn dropping_below_a_row_puts_layers_under_it() {
        let (mut presentation, ids, group) = layers();
        let [a, b, c, d] = [ids[0], ids[1], ids[2], ids[3]];
        apply(&mut presentation, &[d], b, DropZone::After);
        assert_eq!(top(&presentation), [a, d, b, c, group]);
    }

    #[test]
    fn dropping_into_a_group_puts_layers_on_top_of_its_children() {
        let (mut presentation, ids, group) = layers();
        apply(&mut presentation, &[ids[0]], group, DropZone::Into);
        let children: Vec<_> = presentation
            .element(group)
            .unwrap()
            .children()
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(children, [ids[4], ids[5], ids[0]]);
        // And out again, below the group.
        apply(&mut presentation, &[ids[4]], group, DropZone::After);
        assert_eq!(top(&presentation), [ids[1], ids[2], ids[3], ids[4], group]);
    }

    #[test]
    fn a_group_cannot_drop_into_itself() {
        let (presentation, ids, group) = layers();
        assert!(layer_moves(&presentation, &[group], group, DropZone::Into).is_none());
        assert!(layer_moves(&presentation, &[group], ids[4], DropZone::Before).is_none());
    }
}
