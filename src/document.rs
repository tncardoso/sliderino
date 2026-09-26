//! The presentation document: what gets saved to disk and what agents edit.
//!
//! Types here hold plain data only (no GPUI types) so they can be serialized
//! later without dragging the UI along.

/// Size of every slide in the presentation, in slide units.
///
/// At 100% zoom one slide unit is one logical pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideSize {
    pub width: u32,
    pub height: u32,
}

impl Default for SlideSize {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
        }
    }
}

/// Stable identity of a slide inside its presentation.
///
/// Ids follow creation order. Reordering keeps them, and the id of a removed
/// slide is never handed out again, so an external agent holding an id never
/// ends up pointing at a different slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlideId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct Slide {
    pub id: SlideId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Presentation {
    pub size: SlideSize,
    /// Slides in presentation order.
    pub slides: Vec<Slide>,
    /// Id given to the next created slide; only ever grows.
    next_slide_id: u64,
}

#[allow(dead_code, reason = "slide editing is not wired to the UI yet")]
impl Presentation {
    /// A presentation with the default size and one empty slide.
    pub fn new() -> Self {
        let mut presentation = Self {
            size: SlideSize::default(),
            slides: Vec::new(),
            next_slide_id: 1,
        };
        presentation.add_slide();
        presentation
    }

    /// Appends an empty slide and returns its id.
    pub fn add_slide(&mut self) -> SlideId {
        let id = SlideId(self.next_slide_id);
        self.next_slide_id += 1;
        self.slides.push(Slide { id });
        id
    }

    /// Removes the slide; its id is not reused. Returns the removed slide.
    pub fn remove_slide(&mut self, id: SlideId) -> Option<Slide> {
        let index = self.index_of(id)?;
        Some(self.slides.remove(index))
    }

    /// Moves the slide to `index` (clamped to the last position).
    /// Returns false when no slide has that id.
    pub fn move_slide(&mut self, id: SlideId, index: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let slide = self.slides.remove(from);
        let index = index.min(self.slides.len());
        self.slides.insert(index, slide);
        true
    }

    pub fn index_of(&self, id: SlideId) -> Option<usize> {
        self.slides.iter().position(|slide| slide.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(presentation: &Presentation) -> Vec<u64> {
        presentation.slides.iter().map(|slide| slide.id.0).collect()
    }

    #[test]
    fn new_presentation_is_1600_by_900_with_one_slide() {
        let presentation = Presentation::new();
        assert_eq!(
            presentation.size,
            SlideSize {
                width: 1600,
                height: 900
            }
        );
        assert_eq!(ids(&presentation), [1]);
    }

    #[test]
    fn ids_follow_creation_order_and_survive_reordering() {
        let mut presentation = Presentation::new();
        let second = presentation.add_slide();
        presentation.add_slide();
        assert!(presentation.move_slide(second, 0));
        assert_eq!(ids(&presentation), [2, 1, 3]);
        assert!(presentation.move_slide(second, 99));
        assert_eq!(ids(&presentation), [1, 3, 2]);
    }

    #[test]
    fn removed_ids_are_never_reused() {
        let mut presentation = Presentation::new();
        let second = presentation.add_slide();
        let third = presentation.add_slide();
        presentation.remove_slide(third);
        presentation.remove_slide(second);
        assert_eq!(presentation.add_slide(), SlideId(4));
        assert_eq!(presentation.remove_slide(third), None);
        assert!(!presentation.move_slide(third, 0));
    }
}
