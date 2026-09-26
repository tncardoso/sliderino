//! Puts font files into the presentation from the editor: the upload button
//! next to the font family and files dropped on the canvas.
//!
//! Each way reads the files off the UI thread, then makes one undo step that
//! embeds their faces (see [`fonts::plan_upload`]). The upload button also
//! gives the new font to the selected texts.

use std::path::PathBuf;

use gpui_kit::{Context, PathPromptOptions, Window};

use crate::document::{ElementId, FontFace, Operation, TextStylePatch};
use crate::editor::EditorView;
use crate::fonts::{self, UploadedFace};
use crate::ui::{show_error, show_warning};

impl EditorView {
    /// Embeds the uploaded faces as one undo step. With `apply_to_selection`,
    /// the selected texts change to the uploaded face closest to their
    /// weight and slant. Returns the notes for the author.
    pub fn upload_fonts(
        &mut self,
        uploaded: Vec<UploadedFace>,
        apply_to_selection: bool,
    ) -> Vec<String> {
        let embedded: Vec<_> = self
            .presentation
            .fonts
            .iter()
            .map(|(face, data)| (face.clone(), data.clone()))
            .collect();
        let plan = fonts::plan_upload(&embedded, fonts::catalog(), uploaded);
        let mut operations = plan.operations;
        if apply_to_selection {
            for (id, current) in self.selected_texts() {
                let Some(face) = fonts::closest_face(&plan.faces, current.weight, current.italic)
                else {
                    continue;
                };
                if *face != current {
                    operations.push(Operation::SetTextStyle {
                        id,
                        patch: TextStylePatch {
                            font: Some(face.clone()),
                            ..Default::default()
                        },
                    });
                }
            }
        }
        if !operations.is_empty() {
            let selection = self.selection.clone();
            self.end_text_edit();
            self.commit_pruning("Add font", operations, selection.clone());
            self.selection = selection;
        }
        plan.warnings
    }

    /// The selected texts and their faces, texts in selected groups too.
    fn selected_texts(&self) -> Vec<(ElementId, FontFace)> {
        let mut texts = Vec::new();
        for id in &self.selection {
            let Some(element) = self.presentation.element(*id) else {
                continue;
            };
            let mut stack = vec![element];
            while let Some(element) = stack.pop() {
                if let Some(text) = element.as_text() {
                    texts.push((element.id, text.style.font.clone()));
                }
                if let Some(group) = element.as_group() {
                    stack.extend(group.children.iter());
                }
            }
        }
        texts
    }

    /// Asks for font files, then embeds them and gives them to the selected
    /// texts.
    pub fn choose_font_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Upload".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.load_font_files(paths, true, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Reads font files off the UI thread, then embeds all their faces as
    /// one undo step.
    pub fn load_font_files(
        &mut self,
        paths: Vec<PathBuf>,
        apply_to_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let files = gpui_kit::AppContext::background_spawn(cx, async move {
                // The upload compares names with the installed fonts.
                fonts::catalog();
                paths
                    .into_iter()
                    .map(|path| {
                        let bytes = std::fs::read(&path)
                            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
                        fonts::read_file(bytes)
                            .map_err(|error| format!("Cannot use {}: {error}", path.display()))
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                let mut uploaded = Vec::new();
                for file in files {
                    match file {
                        Ok(faces) => uploaded.extend(faces),
                        Err(message) => show_error(message, window, cx),
                    }
                }
                if !uploaded.is_empty() {
                    for warning in this.upload_fonts(uploaded, apply_to_selection) {
                        show_warning(warning, window, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Removes an embedded family as one undo step; its texts change to
    /// Inter.
    pub fn remove_font_family(&mut self, family: &str) -> Result<(), fonts::RemoveFamilyError> {
        let operations = fonts::remove_family(&self.presentation, family)?;
        let selection = self.selection.clone();
        self.commit_pruning("Remove font", operations, selection.clone());
        self.selection = selection;
        Ok(())
    }
}
