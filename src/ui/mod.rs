#[cfg(test)]
mod agent_tests;
pub mod canvas;
#[cfg(test)]
mod group_tests;
pub mod hierarchy_panel;
#[cfg(test)]
mod hierarchy_tests;
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
