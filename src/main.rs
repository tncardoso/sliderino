//! `sliderino`: without a command, opens the editor. The commands are the
//! agent API: `mcp` serves it to an MCP client on stdio, the others call
//! the open editor and print JSON. See `docs/agent-api.md`.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use sliderino::api::client::Session;
use sliderino::api::protocol::{ApiError, ClientKind};
use sliderino::api::{mcp, server, tools};
use sliderino::app;
use sliderino::document::Presentation;
use sliderino::history::History;

#[derive(Parser)]
#[command(version, about = "Presentation editor for the agents age")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Open the editor without the agent API socket.
    #[arg(long)]
    no_api: bool,
    /// Process id of the editor to call, when several are open.
    #[arg(long, global = true, env = "SLIDERINO_INSTANCE")]
    instance: Option<u32>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the agent API to an MCP client on stdio.
    Mcp,
    /// Call a tool with JSON arguments ("-" reads them from stdin).
    Call { tool: String, args: Option<String> },
    /// List the tools with their descriptions and argument schemas.
    Tools,
    /// List the open editors (list_instances).
    Instances,
    /// Open an editor and wait until it accepts calls (open_editor).
    Open,
    /// Overview of the presentation (get_basic_info).
    Info,
    /// Render a slide to a PNG file (get_screenshot).
    Screenshot {
        /// PNG file to write.
        #[arg(short, long)]
        output: PathBuf,
        /// Slide id; the slide shown in the editor by default.
        #[arg(long)]
        slide: Option<u64>,
        /// Pixels per slide unit.
        #[arg(long)]
        scale: Option<f32>,
        /// Draw text frames, line boxes, baselines and overflow.
        #[arg(long)]
        overlay: bool,
    },
    /// Apply a JSON list of operations as one undo step (apply_operations).
    Apply {
        /// File of operations; "-" reads stdin.
        file: PathBuf,
        /// Name of the undo step.
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        base_revision: Option<u64>,
    },
    /// Undo the latest step of the shared history.
    Undo {
        #[arg(long)]
        base_revision: Option<u64>,
    },
    /// Redo the latest undone step.
    Redo {
        #[arg(long)]
        base_revision: Option<u64>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        open_editor(!cli.no_api);
        return ExitCode::SUCCESS;
    };
    let call = match command {
        Command::Mcp => {
            return match mcp::run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("error: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        Command::Tools => {
            let listed: Vec<Value> = tools::specs()
                .into_iter()
                .map(|spec| {
                    json!({
                        "name": spec.name,
                        "description": spec.description,
                        "input_schema": spec.input_schema(),
                    })
                })
                .collect();
            return print(Ok(json!(listed)));
        }
        Command::Call { tool, args } => read_args(args.as_deref()).map(|args| (tool, args)),
        Command::Instances => Ok(("list_instances".into(), json!({}))),
        Command::Open => Ok(("open_editor".into(), json!({}))),
        Command::Info => Ok(("get_basic_info".into(), json!({}))),
        Command::Screenshot {
            output,
            slide,
            scale,
            overlay,
        } => {
            let mut args = json!({"path": output, "overlay": overlay});
            if let Some(slide) = slide {
                args["slide"] = json!(slide);
            }
            if let Some(scale) = scale {
                args["scale"] = json!(scale);
            }
            Ok(("get_screenshot".into(), args))
        }
        Command::Apply {
            file,
            label,
            base_revision,
        } => read_text(Some(&file)).and_then(|text| {
            let ops: Value = serde_json::from_str(&text).map_err(ApiError::invalid_args)?;
            let mut args = json!({"ops": ops});
            if let Some(label) = label {
                args["label"] = json!(label);
            }
            if let Some(base) = base_revision {
                args["base_revision"] = json!(base);
            }
            Ok(("apply_operations".into(), args))
        }),
        Command::Undo { base_revision } => Ok(("undo".into(), step_args(base_revision))),
        Command::Redo { base_revision } => Ok(("redo".into(), step_args(base_revision))),
    };
    print(call.and_then(|(tool, mut args)| {
        if let (Some(pid), Some(object)) = (cli.instance, args.as_object_mut())
            && tools::spec(&tool).is_some_and(|spec| spec.target == tools::Target::Instance)
        {
            object.entry("instance").or_insert(json!(pid));
        }
        let output = Session::new("sliderino CLI", ClientKind::Cli).call(&tool, args)?;
        let mut value = output.value;
        if let Some(image) = output.image {
            value["image"] = json!(image);
        }
        Ok(value)
    }))
}

fn open_editor(api: bool) {
    app::application().run(move |cx| {
        app::init(cx);
        app::open_editor(
            cx,
            Presentation::new(),
            History::default(),
            move |editor, _, cx| {
                if api && let Err(error) = server::start(editor, cx) {
                    eprintln!("sliderino: the agent API is off: {error}");
                }
            },
        );
    });
}

fn step_args(base_revision: Option<u64>) -> Value {
    match base_revision {
        Some(base) => json!({"base_revision": base}),
        None => json!({}),
    }
}

/// Reads a file, or stdin for "-".
fn read_text(path: Option<&std::path::Path>) -> Result<String, ApiError> {
    let mut text = String::new();
    match path {
        Some(path) if path.as_os_str() != "-" => {
            text = std::fs::read_to_string(path).map_err(|error| {
                ApiError::new("io", format!("cannot read {}: {error}", path.display()))
            })?;
        }
        _ => {
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|error| ApiError::new("io", error.to_string()))?;
        }
    }
    Ok(text)
}

fn read_args(args: Option<&str>) -> Result<Value, ApiError> {
    let text = match args {
        None => return Ok(json!({})),
        Some("-") => read_text(None)?,
        Some(text) => text.to_string(),
    };
    serde_json::from_str(&text).map_err(ApiError::invalid_args)
}

/// Prints the result as JSON on stdout, or the error on stderr.
fn print(result: Result<Value, ApiError>) -> ExitCode {
    match result {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("JSON serializes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            let error = json!({"error": error});
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&error).expect("JSON serializes")
            );
            ExitCode::FAILURE
        }
    }
}
