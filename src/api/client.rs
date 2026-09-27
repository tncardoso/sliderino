//! Finding and calling editor instances, for `sliderino mcp` and the CLI.
//!
//! Each editor writes `<pid>.sock` and `<pid>.json` in the runtime
//! directory. Clients list the `.json` files, drop the entries whose socket
//! no longer accepts connections, and pick one instance: the only one, or
//! the one the caller names.

use std::collections::HashMap;
use std::io::BufReader;
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::protocol::{
    self, ApiError, ClientKind, Request, RequestBody, Response, ToolOutput,
};
use crate::api::tools::{self, Target};

/// Environment variable that overrides the runtime directory, for tests.
pub const RUNTIME_DIR_ENV: &str = "SLIDERINO_RUNTIME_DIR";

/// How long `open_editor` waits for the new editor to accept calls.
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);

/// The directory of the instance files: `$XDG_RUNTIME_DIR/sliderino`, else
/// `$TMPDIR/sliderino-<uid>`.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(RUNTIME_DIR_ENV) {
        return PathBuf::from(dir);
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("sliderino"),
        None => {
            // SAFETY: getuid has no preconditions and cannot fail.
            let uid = unsafe { libc::getuid() };
            std::env::temp_dir().join(format!("sliderino-{uid}"))
        }
    }
}

/// Creates the runtime directory, readable by the user only.
pub fn create_runtime_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// What an editor writes about itself in `<pid>.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstanceInfo {
    pub pid: u32,
    pub title: String,
    pub api_version: u32,
    /// Seconds since the Unix epoch.
    pub started: u64,
}

impl InstanceInfo {
    pub fn socket(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.sock", self.pid))
    }
}

/// The instances that accept connections, oldest first. Entries of
/// instances that are gone are deleted.
pub fn instances(dir: &Path) -> Vec<InstanceInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let info: Option<InstanceInfo> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        match info {
            Some(info) if UnixStream::connect(info.socket(dir)).is_ok() => found.push(info),
            Some(info) => {
                std::fs::remove_file(info.socket(dir)).ok();
                std::fs::remove_file(&path).ok();
            }
            // Possibly being written right now: leave it.
            None => {}
        }
    }
    found.sort_by_key(|info| (info.started, info.pid));
    found
}

/// The instance to call: `pid` when given, else the only open one.
pub fn choose(found: Vec<InstanceInfo>, pid: Option<u32>) -> Result<InstanceInfo, ApiError> {
    let listed = || json!({"instances": found});
    match pid {
        Some(pid) => found
            .iter()
            .find(|info| info.pid == pid)
            .cloned()
            .ok_or_else(|| {
                ApiError::new("unknown_instance", format!("no Sliderino instance {pid}"))
                    .with_data(listed())
            }),
        None => match found.as_slice() {
            [] => Err(ApiError::new(
                "no_instance",
                "no Sliderino editor is open: call open_editor or start sliderino",
            )),
            [info] => Ok(info.clone()),
            _ => Err(ApiError::new(
                "several_instances",
                "several Sliderino editors are open: pass instance (see list_instances)",
            )
            .with_data(listed())),
        },
    }
}

/// A connection to one instance.
pub struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Connection {
    pub fn open(socket: &Path, client: &str, kind: ClientKind) -> std::io::Result<Self> {
        let writer = UnixStream::connect(socket)?;
        let reader = BufReader::new(writer.try_clone()?);
        let mut connection = Self {
            reader,
            writer,
            next_id: 1,
        };
        connection.request(RequestBody::Hello {
            client: client.into(),
            kind,
        })?;
        Ok(connection)
    }

    fn request(&mut self, body: RequestBody) -> std::io::Result<Response> {
        let id = self.next_id;
        self.next_id += 1;
        protocol::send(&mut self.writer, &Request { id, body })?;
        match protocol::receive::<Response>(&mut self.reader)? {
            Some(response) if response.id == id => Ok(response),
            Some(_) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response to another request",
            )),
            None => Err(std::io::ErrorKind::UnexpectedEof.into()),
        }
    }

    /// Calls a tool. The outer error is a broken connection.
    pub fn call(
        &mut self,
        tool: &str,
        args: Value,
    ) -> std::io::Result<Result<ToolOutput, ApiError>> {
        let response = self.request(RequestBody::Call {
            tool: tool.into(),
            args,
        })?;
        Ok(match (response.result, response.error) {
            (Some(result), _) => Ok(result),
            (None, Some(error)) => Err(error),
            (None, None) => Err(ApiError::new("protocol", "empty response")),
        })
    }
}

/// A client's connections, one per instance, kept open between calls so
/// the editor shows the client as connected.
pub struct Session {
    pub client: String,
    pub kind: ClientKind,
    dir: PathBuf,
    connections: HashMap<u32, Connection>,
}

impl Session {
    pub fn new(client: impl Into<String>, kind: ClientKind) -> Self {
        Self {
            client: client.into(),
            kind,
            dir: runtime_dir(),
            connections: HashMap::new(),
        }
    }

    /// Calls any tool. `instance` in `args` picks the instance; for
    /// `get_screenshot` and `render_shader_video`, `path` writes the image
    /// or the video to a file here instead of returning it. The `add_image`,
    /// `add_video` and `add_font` ops of `apply_operations` read their
    /// `path` here.
    pub fn call(&mut self, tool: &str, mut args: Value) -> Result<ToolOutput, ApiError> {
        let Some(spec) = tools::spec(tool) else {
            return Err(ApiError::new("unknown_tool", format!("no tool {tool:?}")));
        };
        if args.is_null() {
            args = json!({});
        }
        let Some(object) = args.as_object_mut() else {
            return Err(ApiError::invalid_args("arguments must be a JSON object"));
        };
        let instance = match object.remove("instance") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .ok_or_else(|| ApiError::invalid_args("instance must be a process id"))?,
            ),
        };
        let path = match (tool, object.remove("path")) {
            ("get_screenshot" | "render_shader_video", Some(Value::String(path))) => {
                Some(PathBuf::from(path))
            }
            ("render_shader_video", None) => {
                return Err(ApiError::invalid_args("render_shader_video needs path"));
            }
            ("save_presentation" | "open_presentation", Some(Value::String(path))) => {
                // The editor runs in another folder: send the path from here.
                let base = std::env::current_dir().unwrap_or_default();
                object.insert("path".into(), json!(base.join(path)));
                None
            }
            (_, None) => None,
            (_, Some(_)) => return Err(ApiError::invalid_args("path must be a file path")),
        };
        if tool == "apply_operations"
            && let Some(ops) = object.get_mut("ops")
        {
            // The editor reads no files: images and videos go by value.
            let base = std::env::current_dir().unwrap_or_default();
            crate::api::ops::inline_paths(ops, &base)
                .map_err(|message| ApiError::new("io", message))?;
        }
        if spec.target == Target::Local {
            return self.call_local(tool, args);
        }
        let info = choose(instances(&self.dir), instance)?;
        let output = self.call_instance(&info, tool, args)?;
        match path {
            Some(path) => save_image(output, &path),
            None => Ok(output),
        }
    }

    fn call_instance(
        &mut self,
        info: &InstanceInfo,
        tool: &str,
        args: Value,
    ) -> Result<ToolOutput, ApiError> {
        // A kept connection may have closed with an earlier editor of the
        // same pid: retry once on a new one.
        for _ in 0..2 {
            let connection = match self.connections.entry(info.pid) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let connection =
                        Connection::open(&info.socket(&self.dir), &self.client, self.kind)
                            .map_err(|error| {
                                ApiError::new("connection_failed", error.to_string())
                            })?;
                    entry.insert(connection)
                }
            };
            match connection.call(tool, args.clone()) {
                Ok(result) => return result,
                Err(_) => {
                    self.connections.remove(&info.pid);
                }
            }
        }
        Err(ApiError::new(
            "connection_failed",
            format!("instance {} closed the connection", info.pid),
        ))
    }

    fn call_local(&mut self, tool: &str, args: Value) -> Result<ToolOutput, ApiError> {
        if args.as_object().is_some_and(|args| !args.is_empty()) {
            return Err(ApiError::invalid_args(format!("{tool} takes no arguments")));
        }
        match tool {
            "list_instances" => Ok(json!({"instances": instances(&self.dir)}).into()),
            "open_editor" => {
                let info = open_editor(&self.dir)?;
                Ok(json!({"instance": info}).into())
            }
            _ => Err(ApiError::new("unknown_tool", format!("no tool {tool:?}"))),
        }
    }
}

/// Writes the image (or the video) of a tool output to `path`, replacing it
/// by the path in the output.
fn save_image(mut output: ToolOutput, path: &Path) -> Result<ToolOutput, ApiError> {
    let Some(image) = output.image.take() else {
        return Ok(output);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image.data)
        .map_err(|error| ApiError::new("protocol", error.to_string()))?;
    std::fs::write(path, bytes).map_err(|error| {
        ApiError::new("io", format!("cannot write {}: {error}", path.display()))
    })?;
    output.value["path"] = json!(path);
    Ok(output)
}

/// Starts an editor detached from this process and waits until it accepts
/// calls.
pub fn open_editor(dir: &Path) -> Result<InstanceInfo, ApiError> {
    use std::os::unix::process::CommandExt as _;
    let exe =
        std::env::current_exe().map_err(|error| ApiError::new("open_failed", error.to_string()))?;
    let mut child = std::process::Command::new(exe)
        .arg("--new")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Its own process group: signals sent to the client's terminal do
        // not close the editor.
        .process_group(0)
        .spawn()
        .map_err(|error| ApiError::new("open_failed", error.to_string()))?;
    let pid = child.id();
    let start = Instant::now();
    while start.elapsed() < OPEN_TIMEOUT {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(ApiError::new(
                "open_failed",
                format!("the editor exited with {status}"),
            ));
        }
        if let Some(info) = instances(dir).into_iter().find(|info| info.pid == pid) {
            // Reap the editor when it exits; the thread ends with it.
            std::thread::spawn(move || child.wait());
            return Ok(info);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(ApiError::new(
        "open_failed",
        "the editor did not accept calls in time",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn info(pid: u32, started: u64) -> InstanceInfo {
        InstanceInfo {
            pid,
            title: "Untitled".into(),
            api_version: 1,
            started,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sliderino-test-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        create_runtime_dir(&dir).unwrap();
        dir
    }

    fn write_info(dir: &Path, info: &InstanceInfo) {
        std::fs::write(
            dir.join(format!("{}.json", info.pid)),
            serde_json::to_string(info).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn dead_instances_are_removed() {
        let dir = temp_dir("dead");
        let live = info(10, 2);
        let _listener = UnixListener::bind(live.socket(&dir)).unwrap();
        write_info(&dir, &live);
        let dead = info(11, 1);
        drop(UnixListener::bind(dead.socket(&dir)).unwrap());
        write_info(&dir, &dead);

        assert_eq!(instances(&dir), [live]);
        assert!(!dir.join("11.json").exists());
        assert!(!dead.socket(&dir).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_saved_image_leaves_the_output() {
        let dir = temp_dir("image");
        let path = dir.join("shot.png");
        let output = ToolOutput {
            value: json!({"slide": 1}),
            image: Some(crate::api::protocol::Image {
                mime: "image/png".into(),
                data: "UE5H".into(),
            }),
        };
        let saved = save_image(output, &path).unwrap();
        assert!(saved.image.is_none());
        assert_eq!(saved.value["path"], json!(path));
        assert_eq!(std::fs::read(&path).unwrap(), b"PNG");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn one_instance_is_chosen_without_a_pid() {
        assert_eq!(choose(vec![], None).unwrap_err().code, "no_instance");
        assert_eq!(choose(vec![info(1, 0)], None).unwrap().pid, 1);
        let two = vec![info(1, 0), info(2, 0)];
        let error = choose(two.clone(), None).unwrap_err();
        assert_eq!(error.code, "several_instances");
        assert_eq!(error.data["instances"][1]["pid"], 2);
        assert_eq!(choose(two.clone(), Some(2)).unwrap().pid, 2);
        assert_eq!(choose(two, Some(3)).unwrap_err().code, "unknown_instance");
    }
}
