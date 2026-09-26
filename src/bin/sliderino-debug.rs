//! Debug tooling for agents: subcommands that exercise parts of the app in
//! isolation (e.g. build a scene and render a screenshot) so outcomes can be
//! validated without driving the GUI.
//!
//! A scene is a JSON list of operations (see `sliderino::script`). `render`
//! draws it on the CPU; `scene` opens it in the real editor and can capture
//! the slide from the screen (X11), so the two paths can be compared.

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use gpui_kit::{App, Bounds, Pixels, Size, WeakEntity, Window};
use sliderino::document::{ElementId, Presentation, SlideId};
use sliderino::editor::EditorView;
use sliderino::history::History;
use sliderino::{app, render, script};

/// Longest wait for the editor to paint the scene before capturing.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Parser)]
#[command(
    name = "sliderino-debug",
    about = "Debug subcommands for testing sliderino internals"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Confirms the debug binary runs.
    Ping,
    /// Applies a scene and renders a slide to PNG on the CPU.
    Render {
        /// Scene file: a JSON list of operations.
        #[arg(long)]
        ops: PathBuf,
        /// PNG to write.
        #[arg(short, long)]
        output: PathBuf,
        /// Slide id to render.
        #[arg(long, default_value_t = 1)]
        slide: u64,
        /// Pixels per slide unit.
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// Draw text frames, line boxes, baselines and overflow.
        #[arg(long)]
        overlay: bool,
    },
    /// Opens a scene in the editor. With --output, captures the slide from
    /// the screen (needs X11 with xdotool and ImageMagick's import) and exits.
    Scene {
        /// Scene file: a JSON list of operations.
        #[arg(long)]
        ops: PathBuf,
        /// PNG to write; without it the editor stays open.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Slide id to show.
        #[arg(long)]
        slide: Option<u64>,
        /// Element to select, showing its outline, handles and size.
        #[arg(long)]
        select: Option<u64>,
        /// Capture the whole window instead of the slide.
        #[arg(long)]
        full: bool,
    },
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Ping => {
            println!("pong");
            Ok(())
        }
        Command::Render {
            ops,
            output,
            slide,
            scale,
            overlay,
        } => render(&ops, &output, SlideId(slide), scale, overlay),
        Command::Scene {
            ops,
            output,
            slide,
            select,
            full,
        } => scene(
            &ops,
            output,
            slide.map(SlideId),
            select.map(ElementId),
            full,
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A new presentation with the scene applied, and the applied steps.
fn load_scene(ops: &Path) -> Result<(Presentation, Vec<sliderino::api::ops::Step>)> {
    let mut presentation = Presentation::new();
    let applied = script::apply(&mut presentation, script::load(ops)?)?;
    Ok((presentation, applied))
}

fn print_report(presentation: &Presentation, slide: SlideId) {
    for text in render::report(presentation, slide) {
        println!("{text}");
    }
}

fn render(ops: &Path, output: &Path, slide: SlideId, scale: f32, overlay: bool) -> Result<()> {
    let (presentation, _) = load_scene(ops)?;
    let pixmap = render::render_slide(&presentation, slide, scale, overlay)?;
    pixmap.save_png(output)?;
    println!("{}", output.display());
    print_report(&presentation, slide);
    Ok(())
}

fn scene(
    ops: &Path,
    output: Option<PathBuf>,
    slide: Option<SlideId>,
    select: Option<ElementId>,
    full: bool,
) -> Result<()> {
    let (presentation, applied) = load_scene(ops)?;
    if let Some(slide) = slide
        && presentation.slide(slide).is_none()
    {
        return Err(format!("no slide {}", slide.0).into());
    }
    if let Some(id) = select
        && presentation.element(id).is_none()
    {
        return Err(format!("no element {}", id.0).into());
    }
    if output.is_some() {
        check_capture_tools()?;
    }
    let shown = select
        .and_then(|id| presentation.locate(id).map(|(slide, _)| slide))
        .or(slide)
        .unwrap_or(presentation.slides[0].id);
    print_report(&presentation, shown);

    let mut history = History::default();
    for step in applied {
        history.record(format!("Scene: {}", step.label), step.inverse, None, None);
    }

    app::application().run(move |cx| {
        app::init(cx);
        app::open_editor(cx, presentation, history, move |editor, window, cx| {
            editor.select_slide(shown);
            editor.selection = select;
            if let Some(output) = output {
                let view = cx.weak_entity();
                let capture = Capture {
                    view,
                    output,
                    full,
                    started: Instant::now(),
                    fitted_to: None,
                };
                window.on_next_frame(move |window, cx| capture.wait(2, window, cx));
            }
        });
    });
    Ok(())
}

fn check_capture_tools() -> Result<()> {
    if std::env::var_os("DISPLAY").is_none() {
        return Err("capturing needs an X11 display (DISPLAY is not set)".into());
    }
    for tool in ["xdotool", "import"] {
        let found = Process::new("which")
            .arg(tool)
            .output()
            .is_ok_and(|output| output.status.success());
        if !found {
            return Err(format!("capturing needs `{tool}` on the PATH").into());
        }
    }
    Ok(())
}

/// A pending screenshot of the editor window.
struct Capture {
    view: WeakEntity<EditorView>,
    output: PathBuf,
    full: bool,
    started: Instant,
    /// Canvas size the slide was last fitted to. The window manager may
    /// resize the window after it opens, so the slide is fitted again until
    /// the size holds.
    fitted_to: Option<Size<Pixels>>,
}

impl Capture {
    /// Waits until the slide is fitted to a canvas size that held for
    /// `frames` frames, then captures and quits.
    fn wait(mut self, frames: u32, window: &mut Window, cx: &mut App) {
        if self.started.elapsed() > CAPTURE_TIMEOUT {
            fail("the editor did not render the scene in time");
        }
        let viewport = self
            .view
            .update(cx, |editor, cx| {
                let size = editor.viewport.size;
                if editor.camera.is_some() && self.fitted_to != Some(size) {
                    editor.zoom_to_fit();
                    cx.notify();
                }
                editor.camera.map(|_| size)
            })
            .ok()
            .flatten();
        let frames = match viewport {
            Some(size) if self.fitted_to == Some(size) => frames.saturating_sub(1),
            Some(size) => {
                self.fitted_to = Some(size);
                2
            }
            None => 2,
        };
        if frames == 0 {
            match self.take(window, cx) {
                Ok(()) => {
                    println!("{}", self.output.display());
                    cx.quit();
                }
                Err(error) => fail(&error.to_string()),
            }
            return;
        }
        window.on_next_frame(move |window, cx| self.wait(frames, window, cx));
        window.refresh();
    }

    fn take(&self, window: &mut Window, cx: &mut App) -> Result<()> {
        let search = Process::new("xdotool")
            .args(["search", "--onlyvisible", "--pid"])
            .arg(std::process::id().to_string())
            .output()?;
        let window_id = String::from_utf8(search.stdout)?
            .lines()
            .last()
            .map(str::to_string)
            .ok_or("the editor window was not found with xdotool")?;
        let shot = std::env::temp_dir().join(format!("sliderino-scene-{}.png", std::process::id()));
        let status = Process::new("import")
            .args(["-window", &window_id])
            .arg(&shot)
            .status()?;
        if !status.success() {
            return Err("import could not capture the window".into());
        }
        let mut pixmap = tiny_skia::Pixmap::load_png(&shot)?;
        let _ = std::fs::remove_file(&shot);
        if !self.full {
            let slide = self
                .view
                .read_with(cx, |editor, _| editor.slide_bounds())
                .ok()
                .flatten()
                .ok_or("the slide is not on screen")?;
            pixmap = crop(&pixmap, slide, window.scale_factor())?;
        }
        pixmap.save_png(&self.output)?;
        Ok(())
    }
}

/// Cuts window-relative logical `bounds` out of a window capture.
fn crop(
    pixmap: &tiny_skia::Pixmap,
    bounds: Bounds<Pixels>,
    scale: f32,
) -> Result<tiny_skia::Pixmap> {
    let device = |value: Pixels| (f32::from(value) * scale).round() as i32;
    let left = device(bounds.left()).clamp(0, pixmap.width() as i32);
    let top = device(bounds.top()).clamp(0, pixmap.height() as i32);
    let right = device(bounds.right()).clamp(0, pixmap.width() as i32);
    let bottom = device(bounds.bottom()).clamp(0, pixmap.height() as i32);
    tiny_skia::IntRect::from_ltrb(left, top, right, bottom)
        .and_then(|rect| pixmap.clone_rect(rect))
        .ok_or_else(|| "the slide is outside the captured window".into())
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    std::process::exit(1);
}
