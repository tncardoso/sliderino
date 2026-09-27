#[cfg(test)]
mod agent_tests;
pub mod brand;
pub mod canvas;
pub mod font_upload;
#[cfg(test)]
mod font_upload_tests;
#[cfg(test)]
mod group_tests;
pub mod hierarchy_panel;
#[cfg(test)]
mod hierarchy_tests;
pub mod home;
pub mod image_insert;
pub mod inspector;
#[cfg(all(test, feature = "perf"))]
mod perf_tests;
pub mod playback;
pub mod presenter;
pub mod properties_panel;
#[cfg(test)]
mod rotation_tests;
pub mod shape_inspector;
pub mod shape_paint;
#[cfg(test)]
mod shape_tests;
pub mod slides_panel;
#[cfg(test)]
mod slides_panel_tests;
#[cfg(test)]
pub mod test_support;
pub mod text_input;
#[cfg(test)]
mod text_tool_tests;
pub mod top_bar;
pub mod widgets;
pub mod workspace;
#[cfg(test)]
mod workspace_tests;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Window};

/// Tells the author that something they gave could not be used.
pub fn show_error(message: String, window: &mut Window, cx: &mut App) {
    notify(Notification::error(message.clone()), &message, window, cx);
}

/// Tells the author something they should know about their change.
pub fn show_warning(message: String, window: &mut Window, cx: &mut App) {
    notify(Notification::warning(message.clone()), &message, window, cx);
}

fn notify(notification: Notification, message: &str, window: &mut Window, cx: &mut App) {
    if window
        .root::<gpui_kit::component::Root>()
        .flatten()
        .is_some()
    {
        window.push_notification(notification, cx);
    } else {
        eprintln!("sliderino: {message}");
    }
}
