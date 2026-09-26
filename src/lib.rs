//! Sliderino: presentation editor for the agents age. The library holds the
//! document model, text layout and rendering, and the GPUI editor; the
//! binaries are the editor (`sliderino`) and the debug tooling
//! (`sliderino-debug`).

pub mod app;
pub mod assets;
pub mod camera;
pub mod document;
pub mod editor;
pub mod fonts;
pub mod history;
pub mod mock;
pub mod operation;
pub mod perf;
pub mod render;
pub mod script;
pub mod shortcuts;
pub mod snap;
pub mod text_layout;
pub mod theme;
pub mod ui;
