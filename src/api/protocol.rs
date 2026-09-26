//! Messages between the editor and its clients (`sliderino mcp`,
//! `sliderino call`) on the instance socket: one JSON object per line.
//!
//! A client first sends `hello`, then any number of `call` requests. Each
//! request gets one response with the same `id`.

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Mcp,
    Cli,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub body: RequestBody,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum RequestBody {
    /// Names the client, shown in the editor's agent status.
    Hello { client: String, kind: ClientKind },
    Call {
        tool: String,
        #[serde(default)]
        args: Value,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ToolOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl Response {
    pub fn new(id: u64, outcome: Result<ToolOutput, ApiError>) -> Self {
        match outcome {
            Ok(result) => Self {
                id,
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                id,
                result: None,
                error: Some(error),
            },
        }
    }
}

/// What a tool returns: JSON, plus an image for `get_screenshot`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
}

impl From<Value> for ToolOutput {
    fn from(value: Value) -> Self {
        Self { value, image: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub mime: String,
    /// Base64 of the image bytes.
    pub data: String,
}

/// A failed tool call. `code` is stable and machine-readable; `data` holds
/// details such as the current revision of a `stale_revision` error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: Value::Null,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn invalid_args(message: impl std::fmt::Display) -> Self {
        Self::new("invalid_arguments", message.to_string())
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// Writes one message and its line end, then flushes.
pub fn send<T: Serialize>(writer: &mut impl Write, message: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// Reads one message; `None` at the end of the stream.
pub fn receive<T: for<'de> Deserialize<'de>>(
    reader: &mut impl BufRead,
) -> std::io::Result<Option<T>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_round_trip_as_one_line() {
        let request = Request {
            id: 3,
            body: RequestBody::Call {
                tool: "get_basic_info".into(),
                args: json!({}),
            },
        };
        let mut buffer = Vec::new();
        send(&mut buffer, &request).unwrap();
        assert_eq!(buffer.iter().filter(|byte| **byte == b'\n').count(), 1);
        let read: Request = receive(&mut buffer.as_slice()).unwrap().unwrap();
        assert_eq!(read, request);
        let text = String::from_utf8(buffer).unwrap();
        assert!(text.contains(r#""method":"call""#), "{text}");
    }
}
