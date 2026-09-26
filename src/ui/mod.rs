pub mod canvas;
pub mod inspector;
#[cfg(all(test, feature = "perf"))]
mod perf_tests;
pub mod properties_panel;
pub mod slides_panel;
#[cfg(test)]
pub mod test_support;
pub mod text_input;
#[cfg(test)]
mod text_tool_tests;
pub mod top_bar;
pub mod widgets;
