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
use sliderino::{app, pptx, render, script};

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
    /// Applies a scene and exports it to PPTX, without the editor.
    ExportPptx {
        /// Scene file: a JSON list of operations.
        #[arg(long)]
        ops: PathBuf,
        /// PPTX to write.
        #[arg(short, long)]
        output: PathBuf,
        /// Draw thin lines on the text frames and baselines of the layout.
        #[arg(long)]
        guides: bool,
    },
    /// Applies a scene, exports it to PPTX, reads the deck back and
    /// compares each shape with its element. Fails on a difference.
    PptxRoundtrip {
        /// Scene file: a JSON list of operations.
        #[arg(long)]
        ops: PathBuf,
        /// Also keep the PPTX in this file.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Renders each slide of a PPTX file to PNG with LibreOffice.
    PptxRender {
        /// PPTX file to render.
        file: PathBuf,
        /// Folder for the PNG files.
        #[arg(short, long)]
        output: PathBuf,
        /// Pixels per slide unit.
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
    },
    /// Applies a scene, then renders one slide on the CPU and its PPTX
    /// export with LibreOffice, and compares the two images. Writes
    /// reference.png, pptx.png and diff.png (differences in red).
    PptxCompare {
        /// Scene file: a JSON list of operations.
        #[arg(long)]
        ops: PathBuf,
        /// Folder for the images and the deck.
        #[arg(short, long)]
        output: PathBuf,
        /// Slide id to compare.
        #[arg(long, default_value_t = 1)]
        slide: u64,
        /// Pixels per slide unit.
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// Fail when more than this fraction of the pixels differ.
        #[arg(long)]
        threshold: Option<f32>,
    },
    /// Shows the parts of a PPTX file, or one part as indented XML.
    PptxDump {
        /// PPTX file to read.
        file: PathBuf,
        /// Part to show, such as ppt/slides/slide1.xml.
        #[arg(long)]
        part: Option<PathBuf>,
    },
    /// Checks the package of a PPTX file: content types, relationships,
    /// XML and shape ids. With --schema, also validates the XML against the
    /// Open XML schema (needs tools/pptx-validate and dotnet).
    PptxCheck {
        /// PPTX file to check.
        file: PathBuf,
        #[arg(long)]
        schema: bool,
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
        Command::ExportPptx {
            ops,
            output,
            guides,
        } => export_pptx(&ops, &output, guides),
        Command::PptxRoundtrip { ops, output } => pptx_roundtrip(&ops, output.as_deref()),
        Command::PptxRender {
            file,
            output,
            scale,
        } => pptx_render(&file, &output, scale),
        Command::PptxCompare {
            ops,
            output,
            slide,
            scale,
            threshold,
        } => pptx_compare(&ops, &output, SlideId(slide), scale, threshold),
        Command::PptxDump { file, part } => pptx_dump(&file, part.as_deref()),
        Command::PptxCheck { file, schema } => pptx_check(&file, schema),
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

fn export_pptx(ops: &Path, output: &Path, guides: bool) -> Result<()> {
    let (presentation, _) = load_scene(ops)?;
    let export = pptx::export(&presentation, &pptx::Options { guides })?;
    pptx::save(&export, output)?;
    println!("{}", output.display());
    for warning in &export.warnings {
        println!("warning: {warning}");
    }
    Ok(())
}

fn pptx_roundtrip(ops: &Path, output: Option<&Path>) -> Result<()> {
    let (presentation, _) = load_scene(ops)?;
    let export = pptx::export(&presentation, &pptx::Options::default())?;
    if let Some(output) = output {
        pptx::save(&export, output)?;
    }
    let deck = pptx::inspect::Deck::read(&export.bytes)?;
    let mut problems = deck.check();
    problems.extend(pptx::parity::compare(&presentation, &deck)?);
    for warning in &export.warnings {
        println!("warning: {warning}");
    }
    for problem in &problems {
        println!("{problem}");
    }
    if problems.is_empty() {
        println!("ok");
        Ok(())
    } else {
        Err(format!("{} difference(s)", problems.len()).into())
    }
}

fn pptx_render(file: &Path, output: &Path, scale: f32) -> Result<()> {
    let pages = pptx::visual::render(file, &output.join("work"), scale)?;
    for (index, page) in pages.iter().enumerate() {
        let path = output.join(format!("slide{}.png", index + 1));
        page.save_png(&path)?;
        println!("{}", path.display());
    }
    Ok(())
}

fn pptx_compare(
    ops: &Path,
    output: &Path,
    slide: SlideId,
    scale: f32,
    threshold: Option<f32>,
) -> Result<()> {
    let (presentation, _) = load_scene(ops)?;
    let index = presentation
        .index_of(slide)
        .ok_or_else(|| format!("no slide {}", slide.0))?;
    std::fs::create_dir_all(output)?;
    let reference = render::render_slide(&presentation, slide, scale, false)?;
    reference.save_png(output.join("reference.png"))?;
    let export = pptx::export(&presentation, &pptx::Options::default())?;
    let deck = output.join("deck.pptx");
    pptx::save(&export, &deck)?;
    for warning in &export.warnings {
        println!("warning: {warning}");
    }
    let work = output.join("work");
    std::fs::remove_dir_all(&work).ok();
    let pages = pptx::visual::render(&deck, &work, scale)?;
    let page = pages
        .get(index)
        .ok_or("LibreOffice rendered fewer slides than the deck has")?;
    page.save_png(output.join("pptx.png"))?;
    let difference = pptx::visual::difference(&reference, page);
    difference.image.save_png(output.join("diff.png"))?;
    println!(
        "size {}x{} and {}x{}, mean difference {:.3}, differing pixels {:.4}%",
        reference.width(),
        reference.height(),
        page.width(),
        page.height(),
        difference.mean,
        difference.differing * 100.
    );
    // Where the ink of each element lands in the two images.
    let slide = presentation.slide(slide).ok_or("no slide")?;
    for node in slide.visible_leaves() {
        let bounds = node.element.frame.bounds();
        let margin = 24.;
        let area = [
            ((bounds.x - margin) * scale).floor() as i32,
            ((bounds.y - margin) * scale).floor() as i32,
            ((bounds.x + bounds.width + margin) * scale).ceil() as i32,
            ((bounds.y + bounds.height + margin) * scale).ceil() as i32,
        ];
        let expected = pptx::visual::ink_box(&reference, area);
        let got = pptx::visual::ink_box(page, area);
        match (expected, got) {
            (Some(e), Some(g)) => println!(
                "element {}: ink moves {:+} {:+}, size {:+} {:+} px",
                node.element.id.0,
                g[0] as i64 - e[0] as i64,
                g[1] as i64 - e[1] as i64,
                (g[2] - g[0]) as i64 - (e[2] - e[0]) as i64,
                (g[3] - g[1]) as i64 - (e[3] - e[1]) as i64,
            ),
            (e, g) => println!(
                "element {}: ink {:?} in the reference, {:?} in the deck",
                node.element.id.0, e, g
            ),
        }
    }
    match threshold {
        Some(limit) if difference.differing > limit => Err(format!(
            "{:.4}% of the pixels differ, more than {:.4}%",
            difference.differing * 100.,
            limit * 100.
        )
        .into()),
        _ => Ok(()),
    }
}

fn pptx_dump(file: &Path, part: Option<&Path>) -> Result<()> {
    let deck = pptx::inspect::Deck::load(file)?;
    match part {
        Some(part) => {
            let name = part.to_string_lossy();
            let text = deck.text(&name)?;
            print!("{}", pptx::inspect::pretty(text)?);
        }
        None => {
            for name in deck.names() {
                let size = deck.part(name).map_or(0, <[u8]>::len);
                let kind = deck.content_type(name)?.unwrap_or_default();
                println!("{name}  {size} bytes  {kind}");
            }
        }
    }
    Ok(())
}

fn pptx_check(file: &Path, schema: bool) -> Result<()> {
    let deck = pptx::inspect::Deck::load(file)?;
    let mut problems = deck.check();
    if schema {
        problems.extend(pptx_validate(file)?);
    }
    for problem in &problems {
        println!("{problem}");
    }
    if problems.is_empty() {
        println!("ok");
        Ok(())
    } else {
        Err(format!("{} problem(s)", problems.len()).into())
    }
}

/// Validates `file` against the Open XML schema with the .NET tool in
/// `tools/pptx-validate`. Gives one line for each error.
fn pptx_validate(file: &Path) -> Result<Vec<String>> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/pptx-validate");
    let output = Process::new("dotnet")
        .args(["run", "--project"])
        .arg(&project)
        .args(["--configuration", "Release", "--"])
        .arg(file)
        .output()
        .map_err(|error| format!("cannot run dotnet: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    match output.status.code() {
        Some(0) => Ok(Vec::new()),
        Some(1) => Ok(stdout
            .lines()
            .map(|line| format!("schema: {line}"))
            .collect()),
        _ => Err(format!(
            "pptx-validate failed: {}{}",
            stdout,
            String::from_utf8_lossy(&output.stderr)
        )
        .into()),
    }
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
        .and_then(|id| presentation.locate(id).map(|location| location.slide))
        .or(slide)
        .unwrap_or(presentation.slides[0].id);
    print_report(&presentation, shown);

    let mut history = History::default();
    for step in applied {
        history.record(
            format!("Scene: {}", step.label),
            step.inverse,
            vec![],
            vec![],
        );
    }

    app::application().run(move |cx| {
        app::init(cx);
        app::open_editor(cx, presentation, history, move |editor, window, cx| {
            editor.select_slide(shown);
            editor.selection = select.into_iter().collect();
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
                // A capture waits for the images to be decoded.
                editor
                    .camera
                    .filter(|_| !editor.pictures_loading())
                    .map(|_| size)
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
