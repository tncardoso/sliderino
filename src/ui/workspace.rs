//! The content of the Sliderino window: the Home screen or the editor.
//!
//! The workspace switches between them in the same window. Before it drops
//! a presentation with unsaved changes, it asks the person to save or
//! discard them. It also serves the agent API of the window: the tools that
//! change the screen run here, the others on the editor.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Root, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement, Render, Styled, Subscription, TestSupportExt as _, Window, div, px,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::protocol::{ApiError, ToolOutput};
use crate::api::server::{Agents, Event};
use crate::api::tools::{self, Target};
use crate::document::Presentation;
use crate::editor::{EditorEvent, EditorView};
use crate::history::History;
use crate::theme;
use crate::ui::home::{HomeEvent, HomeView};

gpui_kit::actions!(sliderino, [CloseWindow]);

pub enum Screen {
    Home(Entity<HomeView>),
    Editor(Entity<EditorView>),
}

/// A change of screen that drops the open presentation.
enum Leave {
    Home,
    New,
    /// Show a presentation read from `file`.
    Replace {
        presentation: Presentation,
        file: PathBuf,
    },
    Close,
}

pub struct Workspace {
    pub screen: Screen,
    /// The change of screen that waits for the answer about unsaved
    /// changes.
    pending: Option<Leave>,
    _screen_events: Subscription,
}

impl Workspace {
    /// A workspace that shows the Home screen.
    pub fn home(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (screen, subscription) = Self::home_screen(window, cx);
        Self {
            screen,
            pending: None,
            _screen_events: subscription,
        }
    }

    /// A workspace that shows the editor on `presentation`.
    pub fn editor(
        presentation: Presentation,
        history: History,
        file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (screen, subscription) = Self::editor_screen(presentation, history, file, window, cx);
        Self {
            screen,
            pending: None,
            _screen_events: subscription,
        }
    }

    fn home_screen(window: &mut Window, cx: &mut Context<Self>) -> (Screen, Subscription) {
        let home = cx.new(HomeView::new);
        let subscription =
            cx.subscribe_in(&home, window, |this, _, event, window, cx| match event {
                HomeEvent::New => this.leave(Leave::New, window, cx),
                HomeEvent::Open => this.choose_file(window, cx),
            });
        (Screen::Home(home), subscription)
    }

    fn editor_screen(
        presentation: Presentation,
        history: History,
        file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Screen, Subscription) {
        let editor = cx.new(|cx| {
            let mut editor = EditorView::with_document(presentation, history, window, cx);
            editor.file = file;
            editor
        });
        let subscription =
            cx.subscribe_in(&editor, window, |this, _, event, window, cx| match event {
                EditorEvent::GoHome => this.leave(Leave::Home, window, cx),
                EditorEvent::New => this.leave(Leave::New, window, cx),
                EditorEvent::Open => this.choose_file(window, cx),
            });
        (Screen::Editor(editor), subscription)
    }

    /// The editor, unless the Home screen shows.
    pub fn editor_view(&self) -> Option<&Entity<EditorView>> {
        match &self.screen {
            Screen::Editor(editor) => Some(editor),
            Screen::Home(_) => None,
        }
    }

    /// The keyboard focus of the screen.
    pub fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        match &self.screen {
            Screen::Home(home) => home.read(cx).focus.clone(),
            Screen::Editor(editor) => editor.read(cx).focus.clone(),
        }
    }

    /// Whether the open presentation has changes that are not saved.
    pub fn is_dirty(&self, cx: &gpui_kit::App) -> bool {
        self.editor_view()
            .is_some_and(|editor| editor.read(cx).is_dirty())
    }

    fn show(
        &mut self,
        screen: (Screen, Subscription),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        (self.screen, self._screen_events) = screen;
        window.focus(&self.focus_handle(cx), cx);
        cx.notify();
    }

    fn show_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let screen = Self::home_screen(window, cx);
        self.show(screen, window, cx);
    }

    fn show_editor(
        &mut self,
        presentation: Presentation,
        file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let screen = Self::editor_screen(presentation, History::default(), file, window, cx);
        self.show(screen, window, cx);
    }

    /// Does `leave` now, or first asks what to do with the unsaved changes.
    fn leave(&mut self, leave: Leave, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dirty(cx) {
            self.pending = Some(leave);
            self.ask_about_changes(window, cx);
        } else {
            self.go(leave, window, cx);
        }
    }

    fn go(&mut self, leave: Leave, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        match leave {
            Leave::Home => self.show_home(window, cx),
            Leave::New => self.show_editor(Presentation::new(), None, window, cx),
            Leave::Replace { presentation, file } => {
                self.show_editor(presentation, Some(file), window, cx)
            }
            Leave::Close => window.remove_window(),
        }
    }

    /// The Save, Discard and Cancel dialog.
    fn ask_about_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_view().cloned() else {
            return;
        };
        let title = editor.read(cx).title();
        let this = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let (save, discard, cancel, closed) =
                (this.clone(), this.clone(), this.clone(), this.clone());
            let editor = editor.clone();
            dialog
                .w(px(420.))
                .title("Save changes?")
                .child(
                    div()
                        .id("unsaved-message")
                        .test_support()
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .text_color(theme::text_muted())
                        .child(format!(
                            "{title} has changes that are not saved. If you do not save them, they are lost."
                        )),
                )
                .on_close(move |_, _, cx| closed.update(cx, |this, _| this.pending = None))
                .footer(
                    h_flex()
                        .gap(px(8.))
                        .justify_end()
                        .child(
                            Button::new("unsaved-cancel")
                                .small()
                                .outline()
                                .label("Cancel")
                                .on_click(move |_, window, cx| {
                                    cancel.update(cx, |this, _| this.pending = None);
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("unsaved-discard")
                                .small()
                                .outline()
                                .label("Discard")
                                .on_click(move |_, window, cx| {
                                    let leave = discard.update(cx, |this, _| this.pending.take());
                                    window.close_dialog(cx);
                                    if let Some(leave) = leave {
                                        discard.update(cx, |this, cx| this.go(leave, window, cx));
                                    }
                                }),
                        )
                        .child({
                            let editor = editor.clone();
                            Button::new("unsaved-save")
                                .small()
                                .primary()
                                .label("Save")
                                .on_click(move |_, window, cx| {
                                    let leave = save.update(cx, |this, _| this.pending.take());
                                    window.close_dialog(cx);
                                    let Some(leave) = leave else {
                                        return;
                                    };
                                    let saved = editor
                                        .update(cx, |editor, cx| editor.save(false, window, cx));
                                    save.update(cx, |_, cx| {
                                        cx.spawn_in(window, async move |this, cx| {
                                            if saved.await {
                                                this.update_in(cx, |this, window, cx| {
                                                    this.go(leave, window, cx)
                                                })
                                                .ok();
                                            }
                                        })
                                        .detach();
                                    });
                                })
                        }),
                )
        });
    }

    /// Asks for a `.sldr` file, then opens it.
    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(error)) => {
                    this.update_in(cx, |_, window, cx| {
                        crate::ui::show_error(format!("Cannot open: {error}"), window, cx);
                    })
                    .ok();
                    None
                }
            };
            if let Some(path) = path {
                this.update_in(cx, |this, window, cx| this.open_path(path, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Reads `path` in the background, then shows it. A file that cannot be
    /// read shows as an error; the screen stays.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let target = path.clone();
        let loading = cx.background_spawn(async move { crate::file::load(&target) });
        cx.spawn_in(window, async move |this, cx| {
            let loaded = loading.await;
            this.update_in(cx, |this, window, cx| match loaded {
                Ok(presentation) => this.leave(
                    Leave::Replace {
                        presentation,
                        file: path,
                    },
                    window,
                    cx,
                ),
                Err(error) => {
                    let message = format!("Cannot open {}: {error}", path.display());
                    crate::ui::show_error(message, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the window may close now. With unsaved changes, asks first
    /// and closes the window after the answer.
    pub fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.is_dirty(cx) {
            return true;
        }
        if self.pending.is_none() {
            self.leave(Leave::Close, window, cx);
        }
        false
    }

    fn on_close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(window, cx) {
            window.remove_window();
        }
    }

    /// Runs an event of the agent API: clients that connect or close, the
    /// tools that change the screen, and the tools of the editor.
    pub fn on_api_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        if Agents::on_event(&event, cx) {
            return;
        }
        match event {
            Event::Call { tool, args, reply }
                if tools::spec(&tool).is_some_and(|spec| spec.target == Target::Window) =>
            {
                let result = self.screen_tool(&tool, args, window, cx);
                reply.send(result).ok();
            }
            event => match &self.screen {
                Screen::Editor(editor) => {
                    editor.update(cx, |editor, cx| editor.on_api_event(event, cx));
                }
                Screen::Home(_) => match event {
                    Event::Call { reply, .. } => {
                        reply.send(Err(no_presentation())).ok();
                    }
                    Event::ShaderVideoJob { reply, .. } => {
                        reply.send(Err(no_presentation())).ok();
                    }
                    Event::ExportJob { reply, .. } => {
                        reply.send(Err(no_presentation())).ok();
                    }
                    Event::Connected { .. } | Event::Closed { .. } => {}
                },
            },
        }
    }

    fn screen_tool(
        &mut self,
        tool: &str,
        args: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<ToolOutput, ApiError> {
        let args = if args.is_null() { json!({}) } else { args };
        let (presentation, file) = if tool == "open_presentation" {
            let args: OpenArgs = serde_json::from_value(args).map_err(ApiError::invalid_args)?;
            self.check_unsaved(args.discard, cx)?;
            let presentation = crate::file::load(&args.path).map_err(|error| {
                ApiError::new(
                    "cannot_open",
                    format!("cannot open {}: {error}", args.path.display()),
                )
            })?;
            (presentation, Some(args.path))
        } else {
            let args: NewArgs = serde_json::from_value(args).map_err(ApiError::invalid_args)?;
            self.check_unsaved(args.discard, cx)?;
            (Presentation::new(), None)
        };
        // The person may be answering the dialog: the agent decided.
        window.close_all_dialogs(cx);
        let slides = presentation.slides.len();
        self.show_editor(presentation, file.clone(), window, cx);
        self.pending = None;
        Ok(json!({"file": file, "revision": 0, "slides": slides}).into())
    }

    fn check_unsaved(&self, discard: bool, cx: &gpui_kit::App) -> Result<(), ApiError> {
        if self.is_dirty(cx) && !discard {
            return Err(ApiError::new(
                "unsaved_changes",
                "the open presentation has unsaved changes: save them with save_presentation, or pass discard: true",
            ));
        }
        Ok(())
    }
}

fn no_presentation() -> ApiError {
    ApiError::new(
        "no_presentation",
        "the editor shows its Home screen: call new_presentation or open_presentation",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewArgs {
    #[serde(default)]
    discard: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenArgs {
    path: PathBuf,
    #[serde(default)]
    discard: bool,
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = match &self.screen {
            Screen::Home(home) => home.clone().into_any_element(),
            Screen::Editor(editor) => editor.clone().into_any_element(),
        };
        // The kit draws its dialogs and notifications only where the view
        // of the window puts their layers.
        let dialogs = Root::render_dialog_layer(window, cx);
        let notifications = Root::render_notification_layer(window, cx);
        div()
            .id("workspace")
            .size_full()
            .on_action(cx.listener(Self::on_close_window))
            .child(screen)
            .children(dialogs)
            .children(notifications)
    }
}
