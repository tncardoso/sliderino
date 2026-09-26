//! The editor side of the agent API: a Unix socket that clients call.
//!
//! A thread accepts connections and one thread per connection reads its
//! requests. Each request goes over a channel to a task on the UI thread,
//! which runs the tool on the [`EditorView`] and sends the result back. Edits
//! therefore apply between frames, and land in the shared undo history.

use std::io::BufReader;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::Context;
use serde_json::{Value, json};

use crate::api::API_VERSION;
use crate::api::client::{self, InstanceInfo};
use crate::api::ops::{Applied, Op, OpError};
use crate::api::protocol::{
    self, ApiError, ClientKind, Request, RequestBody, Response, ToolOutput,
};
use crate::api::tools::{self, Host, ViewState};
use crate::document::{ApplyError, Presentation};
use crate::editor::{Drag, EditorView};
use crate::history::History;

/// How long the agent status shows a CLI call after it ends.
const CLI_SHOWN_FOR: Duration = Duration::from_secs(3);

/// A client that said hello.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentClient {
    pub connection: u64,
    pub name: String,
    pub kind: ClientKind,
}

/// The editor's view of the API: who is connected, and whether the view
/// follows their edits.
#[derive(Default)]
pub struct Agents {
    pub clients: Vec<AgentClient>,
    /// Until when the status shows the last CLI call.
    pub cli_until: Option<Instant>,
    /// Show the slide and select the element of each agent edit.
    pub follow: bool,
    files: Option<InstanceFiles>,
}

impl Agents {
    /// The text of the agent status: the MCP clients, else a recent CLI
    /// call, else `None`.
    pub fn status(&self, now: Instant) -> Option<String> {
        let mcp: Vec<&str> = self
            .clients
            .iter()
            .filter(|client| client.kind == ClientKind::Mcp)
            .map(|client| client.name.as_str())
            .collect();
        match mcp.as_slice() {
            [] => {}
            [name] => return Some(format!("{name} · MCP")),
            names => return Some(format!("{} agents · MCP", names.len())),
        }
        let cli = self
            .clients
            .iter()
            .any(|client| client.kind == ClientKind::Cli)
            || self.cli_until.is_some_and(|until| now < until);
        cli.then(|| "CLI".to_string())
    }
}

/// The socket and info files of this instance, deleted on drop.
struct InstanceFiles {
    socket: PathBuf,
    info: PathBuf,
}

impl Drop for InstanceFiles {
    fn drop(&mut self) {
        std::fs::remove_file(&self.socket).ok();
        std::fs::remove_file(&self.info).ok();
    }
}

pub enum Event {
    Connected {
        connection: u64,
        name: String,
        kind: ClientKind,
    },
    Closed {
        connection: u64,
    },
    Call {
        tool: String,
        args: Value,
        reply: mpsc::Sender<Result<ToolOutput, ApiError>>,
    },
}

/// Opens this instance's socket and serves it on the editor.
pub fn start(editor: &mut EditorView, cx: &mut Context<EditorView>) -> std::io::Result<()> {
    let dir = client::runtime_dir();
    client::create_runtime_dir(&dir)?;
    let pid = std::process::id();
    let info = InstanceInfo {
        pid,
        title: "Untitled".into(),
        api_version: API_VERSION,
        started: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    };
    let socket = info.socket(&dir);
    std::fs::remove_file(&socket).ok();
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    // Write then rename, so clients never read half a file.
    let info_path = dir.join(format!("{pid}.json"));
    let partial = dir.join(format!("{pid}.json.partial"));
    std::fs::write(&partial, serde_json::to_vec(&info)?)?;
    std::fs::rename(&partial, &info_path)?;

    let (sender, receiver) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("api-accept".into())
        .spawn(move || accept(listener, sender))?;
    cx.spawn(async move |this, cx| {
        while let Ok(event) = receiver.recv().await {
            let handled = this.update(cx, |editor, cx| editor.on_api_event(event, cx));
            if handled.is_err() {
                break;
            }
        }
    })
    .detach();
    // The files go with the editor; clients also drop the entries of
    // instances that exit without deleting them.
    let files = InstanceFiles {
        socket,
        info: info_path,
    };
    cx.on_app_quit(|editor: &mut EditorView, _| {
        editor.agents.files.take();
        async {}
    })
    .detach();
    editor.agents.files = Some(files);
    Ok(())
}

fn accept(listener: UnixListener, events: async_channel::Sender<Event>) {
    for (connection, stream) in (1..).zip(listener.incoming()) {
        let Ok(stream) = stream else {
            continue;
        };
        let events = events.clone();
        std::thread::Builder::new()
            .name(format!("api-{connection}"))
            .spawn(move || serve(stream, connection, events))
            .ok();
    }
}

fn serve(stream: UnixStream, connection: u64, events: async_channel::Sender<Event>) {
    let Ok(read) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read);
    let mut writer = stream;
    let mut greeted = false;
    loop {
        let request = match protocol::receive::<Request>(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                let error = ApiError::new("invalid_request", error.to_string());
                if protocol::send(&mut writer, &Response::new(0, Err(error))).is_err() {
                    break;
                }
                continue;
            }
            Err(_) => break,
        };
        let outcome = match request.body {
            RequestBody::Hello { client, kind } => {
                greeted = true;
                let hello = Event::Connected {
                    connection,
                    name: client,
                    kind,
                };
                if events.send_blocking(hello).is_err() {
                    break;
                }
                Ok(json!({"api_version": API_VERSION, "pid": std::process::id()}).into())
            }
            RequestBody::Call { tool, args } => {
                let (reply, result) = mpsc::channel();
                if events
                    .send_blocking(Event::Call { tool, args, reply })
                    .is_err()
                {
                    break;
                }
                result
                    .recv()
                    .unwrap_or_else(|_| Err(ApiError::new("closed", "the editor closed")))
            }
        };
        if protocol::send(&mut writer, &Response::new(request.id, outcome)).is_err() {
            break;
        }
    }
    if greeted {
        events.send_blocking(Event::Closed { connection }).ok();
    }
}

impl EditorView {
    fn on_api_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Connected {
                connection,
                name,
                kind,
            } => self.agents.clients.push(AgentClient {
                connection,
                name,
                kind,
            }),
            Event::Closed { connection } => {
                let closed = self
                    .agents
                    .clients
                    .iter()
                    .position(|client| client.connection == connection)
                    .map(|index| self.agents.clients.remove(index));
                if closed.is_some_and(|client| client.kind == ClientKind::Cli) {
                    self.agents.cli_until = Some(Instant::now() + CLI_SHOWN_FOR);
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(CLI_SHOWN_FOR).await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                    .detach();
                }
            }
            Event::Call { tool, args, reply } => {
                let _span = crate::perf::span("api_call");
                reply.send(tools::handle(self, &tool, args)).ok();
            }
        }
        cx.notify();
    }

    /// Prepares the view for a change an agent makes, and returns what
    /// [`Self::after_agent_change`] needs.
    fn before_agent_change(&mut self) -> AgentChange {
        self.history.close_burst();
        AgentChange {
            slide_index: self.presentation.index_of(self.current_slide),
            edited: self.edit_content().map(str::to_string),
        }
    }

    /// Keeps the person's view consistent with a change an agent made:
    /// cancels a drag whose element changed and moves the caret of a text
    /// the agent edited. With follow on, shows what the agent changed.
    fn after_agent_change(&mut self, change: AgentChange, applied: Option<&Applied>) {
        let stale_drag = match &self.drag {
            Some(Drag::Move { id, origin, .. }) => self
                .presentation
                .element(*id)
                .is_none_or(|element| element.frame != *origin),
            Some(Drag::Resize {
                id, origin, sizing, ..
            }) => self.presentation.element(*id).is_none_or(|element| {
                element.frame != *origin
                    || element.as_text().map(|text| text.sizing) != Some(*sizing)
            }),
            Some(Drag::Create { .. }) => self.presentation.slide(self.current_slide).is_none(),
            Some(Drag::SelectText { id }) => self.presentation.element(*id).is_none(),
            None => false,
        };
        if stale_drag {
            self.cancel_drag();
        }
        if self.edit_content() != change.edited.as_deref()
            && let Some(edit) = &mut self.text_edit
        {
            // The text moved under the caret: keep it valid.
            edit.marked = None;
            edit.goal_x = None;
        }
        self.repair_view(change.slide_index);
        if self.agents.follow
            && let Some(applied) = applied
        {
            self.follow(applied);
        }
    }

    fn follow(&mut self, applied: &Applied) {
        let element = applied
            .last_element
            .and_then(|id| Some((id, self.presentation.locate(id)?.slide)));
        let slide = element
            .map(|(_, slide)| slide)
            .or(applied.last_slide)
            .filter(|slide| self.presentation.slide(*slide).is_some());
        if let Some(slide) = slide
            && slide != self.current_slide
        {
            self.select_slide(slide);
        }
        if let Some((id, _)) = element
            && self.selection != Some(id)
            && self.text_edit.as_ref().is_none_or(|edit| edit.id != id)
        {
            self.end_text_edit();
            self.selection = Some(id);
        }
    }
}

struct AgentChange {
    slide_index: Option<usize>,
    edited: Option<String>,
}

impl Host for EditorView {
    fn presentation(&self) -> &Presentation {
        &self.presentation
    }

    fn history(&self) -> &History {
        &self.history
    }

    fn view(&self) -> ViewState {
        ViewState {
            current_slide: self.current_slide,
            selection: self.selection,
            text_edit: self
                .text_edit
                .as_ref()
                .map(|edit| (edit.id, edit.selection())),
        }
    }

    fn apply_ops(&mut self, label: &str, ops: Vec<Op>) -> Result<Applied, OpError> {
        let change = self.before_agent_change();
        let applied = tools::apply_and_record(
            &mut self.presentation,
            &mut self.history,
            self.selection,
            label,
            ops,
        )?;
        self.after_agent_change(change, Some(&applied));
        Ok(applied)
    }

    fn undo(&mut self) -> Option<Result<(), ApplyError>> {
        let change = self.before_agent_change();
        let result = EditorView::undo(self);
        self.after_agent_change(change, None);
        result
    }

    fn redo(&mut self) -> Option<Result<(), ApplyError>> {
        let change = self.before_agent_change();
        let result = EditorView::redo(self);
        self.after_agent_change(change, None);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(connection: u64, name: &str, kind: ClientKind) -> AgentClient {
        AgentClient {
            connection,
            name: name.into(),
            kind,
        }
    }

    #[test]
    fn the_status_names_mcp_clients_before_the_cli() {
        let now = Instant::now();
        let mut agents = Agents::default();
        assert_eq!(agents.status(now), None);
        agents.cli_until = Some(now + CLI_SHOWN_FOR);
        assert_eq!(agents.status(now).as_deref(), Some("CLI"));
        assert_eq!(agents.status(now + CLI_SHOWN_FOR), None);
        agents
            .clients
            .push(client(1, "claude-code", ClientKind::Mcp));
        assert_eq!(agents.status(now).as_deref(), Some("claude-code · MCP"));
        agents.clients.push(client(2, "codex", ClientKind::Mcp));
        assert_eq!(agents.status(now).as_deref(), Some("2 agents · MCP"));
    }
}
