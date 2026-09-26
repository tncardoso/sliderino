//! Undo and redo of the presentation.
//!
//! Each step holds the operation that reverts it, as returned by
//! [`Presentation::apply`]. Typing is grouped into bursts: consecutive edits
//! of the same text join one step until the author pauses, moves the caret or
//! leaves the text.

use std::time::{Duration, Instant};

use crate::document::{ApplyError, ElementId, Operation, Presentation};

/// A pause longer than this closes a typing burst.
pub const BURST_PAUSE: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub label: String,
    /// Applying it reverts the step (on the undo stack) or repeats it (on
    /// the redo stack).
    operation: Operation,
    /// Selection before and after the step, restored by undo and redo.
    selection_before: Option<ElementId>,
    selection_after: Option<ElementId>,
    burst: Option<Burst>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Burst {
    element: ElementId,
    last: Instant,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl History {
    /// Records an applied change. `inverse` is what `apply` returned.
    pub fn record(
        &mut self,
        label: impl Into<String>,
        inverse: Operation,
        selection_before: Option<ElementId>,
        selection_after: Option<ElementId>,
    ) {
        self.close_burst();
        self.redo.clear();
        self.undo.push(Step {
            label: label.into(),
            operation: inverse,
            selection_before,
            selection_after,
            burst: None,
        });
    }

    /// Records a typing edit of `element`, joining the open burst of the same
    /// element when the last keystroke was less than [`BURST_PAUSE`] ago.
    pub fn record_typing(&mut self, element: ElementId, inverse: Operation, now: Instant) {
        self.redo.clear();
        if let Some(step) = self.undo.last_mut()
            && let Some(burst) = &mut step.burst
            && burst.element == element
            && now.duration_since(burst.last) < BURST_PAUSE
        {
            // The newest change is reverted first.
            let previous = std::mem::replace(&mut step.operation, Operation::Batch(vec![]));
            step.operation = Operation::Batch(vec![inverse, previous]);
            burst.last = now;
            return;
        }
        self.close_burst();
        self.undo.push(Step {
            label: "Edit text".into(),
            operation: inverse,
            selection_before: Some(element),
            selection_after: Some(element),
            burst: Some(Burst { element, last: now }),
        });
    }

    /// Ends the typing burst in progress, if any: the next edit starts a new
    /// step.
    pub fn close_burst(&mut self) {
        if let Some(step) = self.undo.last_mut() {
            step.burst = None;
        }
    }

    /// Reverts the latest step. Returns the selection to restore, or `None`
    /// when there is nothing to undo.
    pub fn undo(
        &mut self,
        presentation: &mut Presentation,
    ) -> Option<Result<Option<ElementId>, ApplyError>> {
        let mut step = self.undo.pop()?;
        step.burst = None;
        match presentation.apply(step.operation.clone()) {
            Ok(redo) => {
                let selection = step.selection_before;
                step.operation = redo;
                self.redo.push(step);
                Some(Ok(selection))
            }
            Err(error) => {
                self.undo.push(step);
                Some(Err(error))
            }
        }
    }

    /// Repeats the latest undone step. Returns the selection to restore, or
    /// `None` when there is nothing to redo.
    pub fn redo(
        &mut self,
        presentation: &mut Presentation,
    ) -> Option<Result<Option<ElementId>, ApplyError>> {
        let mut step = self.redo.pop()?;
        match presentation.apply(step.operation.clone()) {
            Ok(undo) => {
                let selection = step.selection_after;
                step.operation = undo;
                self.close_burst();
                self.undo.push(step);
                Some(Ok(selection))
            }
            Err(error) => {
                self.redo.push(step);
                Some(Err(error))
            }
        }
    }

    /// Labels of the steps that can be undone, oldest first; the last one is
    /// the current state.
    pub fn done(&self) -> impl Iterator<Item = &str> {
        self.undo.iter().map(|step| step.label.as_str())
    }

    /// Labels of the steps that can be redone, next first.
    pub fn undone(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|step| step.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::tests::{add_text, with_inter};
    use crate::document::{Frame, TextSizing};

    fn content(presentation: &Presentation, id: ElementId) -> String {
        presentation
            .element(id)
            .unwrap()
            .as_text()
            .unwrap()
            .content
            .clone()
    }

    fn type_text(
        history: &mut History,
        presentation: &mut Presentation,
        id: ElementId,
        text: &str,
        at: Instant,
    ) {
        let end = content(presentation, id).len();
        let inverse = presentation
            .apply(Operation::ReplaceText {
                id,
                range: end..end,
                text: text.into(),
            })
            .unwrap();
        history.record_typing(id, inverse, at);
    }

    #[test]
    fn a_typing_burst_is_one_step() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let mut history = History::default();
        let start = Instant::now();
        for (ix, ch) in ["H", "e", "y"].into_iter().enumerate() {
            let at = start + Duration::from_millis(200 * ix as u64);
            type_text(&mut history, &mut presentation, id, ch, at);
        }
        assert_eq!(history.done().count(), 1);
        history.undo(&mut presentation).unwrap().unwrap();
        assert_eq!(content(&presentation, id), "");
        history.redo(&mut presentation).unwrap().unwrap();
        assert_eq!(content(&presentation, id), "Hey");
    }

    #[test]
    fn a_pause_or_a_closed_burst_starts_a_new_step() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let mut history = History::default();
        let start = Instant::now();
        type_text(&mut history, &mut presentation, id, "One", start);
        type_text(
            &mut history,
            &mut presentation,
            id,
            " two",
            start + BURST_PAUSE * 2,
        );
        history.close_burst();
        type_text(
            &mut history,
            &mut presentation,
            id,
            " three",
            start + BURST_PAUSE * 2,
        );
        assert_eq!(history.done().count(), 3);
        history.undo(&mut presentation).unwrap().unwrap();
        assert_eq!(content(&presentation, id), "One two");
    }

    #[test]
    fn undo_and_redo_restore_the_selection() {
        let mut presentation = with_inter();
        let mut history = History::default();
        let id = presentation.new_element_id();
        let slide = presentation.slides[0].id;
        let element = crate::document::Element::new(
            id,
            Frame::default(),
            crate::document::tests::text("Hi", TextSizing::AutoWidth),
        );
        let inverse = presentation
            .apply(Operation::AddElement {
                slide,
                parent: None,
                index: 0,
                element,
            })
            .unwrap();
        history.record("Create text", inverse, None, Some(id));

        assert_eq!(history.undo(&mut presentation), Some(Ok(None)));
        assert!(presentation.element(id).is_none());
        assert_eq!(history.undone().collect::<Vec<_>>(), ["Create text"]);
        assert_eq!(history.redo(&mut presentation), Some(Ok(Some(id))));
        assert!(presentation.element(id).is_some());
        assert_eq!(
            history.undo(&mut presentation).map(|r| r.is_ok()),
            Some(true)
        );
        assert_eq!(history.undo(&mut presentation), None);
    }

    #[test]
    fn a_new_step_clears_redo() {
        let mut presentation = with_inter();
        let id = add_text(
            &mut presentation,
            "",
            TextSizing::AutoWidth,
            Frame::default(),
        );
        let mut history = History::default();
        type_text(&mut history, &mut presentation, id, "a", Instant::now());
        history.undo(&mut presentation).unwrap().unwrap();
        type_text(&mut history, &mut presentation, id, "b", Instant::now());
        assert_eq!(history.undone().count(), 0);
    }
}
