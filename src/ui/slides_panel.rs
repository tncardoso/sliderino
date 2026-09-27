//! Left panel: slide thumbnails, later the component library, and the
//! hierarchy of the current slide.
//!
//! Thumbnails select slides (Ctrl and Shift select more than one), open a
//! menu to add, duplicate or delete them, and drag to change their order.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ContentMask, Context, Div, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement, SharedString, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, WeakEntity, canvas as paint_canvas, div, px,
};

use crate::document::SlideId;
use crate::editor::EditorView;
use crate::theme;
use crate::ui::canvas::{PaintItem, paint_items};
use crate::ui::hierarchy_panel::{DragPreview, hierarchy_panel};

const THUMB_WIDTH: f32 = 184.;

/// Where dragged slides land relative to a thumbnail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlideDrop {
    Before,
    After,
}

/// The slides dragged in the slides panel, in presentation order.
#[derive(Clone, Debug)]
pub struct DraggedSlides {
    pub ids: Vec<SlideId>,
    label: SharedString,
}

/// How a thumbnail shows the selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    None,
    /// Selected, not on the canvas.
    Selected,
    /// On the canvas.
    Current,
}

impl EditorView {
    /// Selects what the slide menu acts on: the selected slides, or only
    /// `id` when it is not selected.
    pub(crate) fn right_click_slide(&mut self, id: SlideId) {
        if !self.slide_selection.contains(&id) {
            self.select_slide(id);
        }
    }

    /// Moves the dragged slides to the drop position found while dragging.
    fn drop_slides(&mut self, dragged: &DraggedSlides) {
        let Some((row, zone)) = self.slide_drop.take() else {
            return;
        };
        let Some(index) = self.presentation.index_of(row) else {
            return;
        };
        let to = match zone {
            SlideDrop::Before => index,
            SlideDrop::After => index + 1,
        };
        self.move_slides(dragged.ids.clone(), to);
    }
}

pub fn slides_panel(editor: &mut EditorView, cx: &mut Context<EditorView>) -> impl IntoElement {
    let _span = crate::perf::span("slides_panel");
    let hierarchy = editor.library_tab == 2;
    let slides: Vec<_> = editor
        .presentation
        .slides
        .iter()
        .map(|slide| slide.id)
        .collect();
    let drop = editor.slide_drop.filter(|_| cx.has_active_drag());
    let thumbnails: Vec<_> = slides
        .into_iter()
        .filter(|_| !hierarchy)
        .enumerate()
        .map(|(ix, id)| slide_row(editor, ix, id, drop, cx))
        .collect();
    let weak = cx.entity().downgrade();

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
                        .on_drag_move::<DraggedSlides>(cx.listener(
                            |this, event: &gpui_kit::DragMoveEvent<DraggedSlides>, _, cx| {
                                if !event.bounds.contains(&event.event.position)
                                    && this.slide_drop.take().is_some()
                                {
                                    cx.notify();
                                }
                            },
                        ))
                        .children(thumbnails)
                        .context_menu(move |menu, _, cx| slide_menu(menu, &weak, cx))
                        .into_any_element(),
                )
            }
        })
}

/// A slide number and its thumbnail.
fn slide_row(
    editor: &mut EditorView,
    ix: usize,
    id: SlideId,
    drop: Option<(SlideId, SlideDrop)>,
    cx: &mut Context<EditorView>,
) -> gpui_kit::AnyElement {
    let items = editor.paint_items(id, false, cx);
    let mark = if id == editor.current_slide {
        Mark::Current
    } else if editor.slide_selection.contains(&id) {
        Mark::Selected
    } else {
        Mark::None
    };
    let dragged = if mark == Mark::None {
        vec![id]
    } else {
        editor.selected_slides()
    };
    let label: SharedString = if dragged.len() > 1 {
        format!("{} slides", dragged.len()).into()
    } else {
        format!("Slide {}", ix + 1).into()
    };
    let payload = DraggedSlides {
        ids: dragged,
        label,
    };
    let zone = drop.filter(|(row, _)| *row == id).map(|(_, zone)| zone);
    h_flex()
        .id(("slide", ix))
        .test_support()
        .relative()
        .items_start()
        .gap(px(8.))
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, _, cx| {
                this.right_click_slide(id);
                cx.notify();
            }),
        )
        .on_click(
            cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                let modifiers = event.modifiers();
                this.click_slide(id, modifiers.shift, modifiers.secondary());
                cx.notify();
            }),
        )
        .on_drag(payload, |payload, _, _, cx| {
            let label = payload.label.clone();
            cx.new(|_| DragPreview { label })
        })
        .on_drag_move::<DraggedSlides>(cx.listener(
            move |this, event: &gpui_kit::DragMoveEvent<DraggedSlides>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                let middle = event.bounds.top() + event.bounds.size.height / 2.;
                let zone = if event.event.position.y < middle {
                    SlideDrop::Before
                } else {
                    SlideDrop::After
                };
                if this.slide_drop != Some((id, zone)) {
                    this.slide_drop = Some((id, zone));
                    cx.notify();
                }
            },
        ))
        .on_drop(cx.listener(|this, dragged: &DraggedSlides, _, cx| {
            this.drop_slides(dragged);
            cx.notify();
        }))
        .child(slide_number(ix + 1, mark))
        .child(thumbnail(editor, items, mark))
        .when_some(zone, |this, zone| this.child(drop_indicator(zone)))
        .into_any_element()
}

/// The line in the gap between thumbnails where dragged slides land.
fn drop_indicator(zone: SlideDrop) -> impl IntoElement {
    let line = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(2.))
        .bg(theme::accent());
    match zone {
        SlideDrop::Before => line.top(px(-8.)),
        SlideDrop::After => line.bottom(px(-8.)),
    }
}

/// The menu of the selected slides.
fn slide_menu(
    menu: PopupMenu,
    editor: &WeakEntity<EditorView>,
    cx: &mut gpui_kit::App,
) -> PopupMenu {
    let Some(entity) = editor.upgrade() else {
        return menu;
    };
    let count = entity.read(cx).slide_selection.len();
    let (duplicate, delete): (SharedString, SharedString) = if count > 1 {
        (
            format!("Duplicate {count} slides").into(),
            format!("Delete {count} slides").into(),
        )
    } else {
        ("Duplicate slide".into(), "Delete slide".into())
    };
    let item = |label: SharedString, action: fn(&mut EditorView)| {
        let editor = editor.clone();
        PopupMenuItem::new(label).on_click(move |_, _, cx| {
            editor
                .update(cx, |editor, cx| {
                    action(editor);
                    cx.notify();
                })
                .ok();
        })
    };
    menu.item(item("New slide".into(), EditorView::add_slide))
        .item(item(duplicate, EditorView::duplicate_slides))
        .separator()
        .item(item(delete, EditorView::delete_slides))
}

fn slide_number(number: usize, mark: Mark) -> impl IntoElement {
    let current = mark == Mark::Current;
    div()
        .w(px(16.))
        .flex_shrink_0()
        .pt(px(2.))
        .flex()
        .justify_end()
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(if mark == Mark::None {
            theme::text_muted()
        } else {
            theme::accent()
        })
        .when(current, |this| this.font_weight(FontWeight::SEMIBOLD))
        .child(number.to_string())
}

/// The slide drawn small, clipped to its edges. The slide on the canvas gets
/// a 2px accent ring, the other selected slides a lighter one.
fn thumbnail(editor: &EditorView, items: Vec<PaintItem>, mark: Mark) -> Div {
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
                        paint_items(&items, bounds.origin, zoom, true, window);
                    });
                },
            )
            .absolute()
            .size_full(),
        );
    match mark {
        Mark::Current => frame.border_2().border_color(theme::accent()),
        Mark::Selected => frame.border_2().border_color(theme::accent().opacity(0.45)),
        Mark::None => frame.border_1().border_color(theme::border()),
    }
}
