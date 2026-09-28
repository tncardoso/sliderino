//! Validates a deck against the Open XML schema, with the .NET tool in
//! `tools/pptx-validate` (the Open XML SDK). The first run builds the tool
//! and needs the NuGet packages.

use std::path::Path;
use std::process::Command;

/// True when `dotnet` is on the PATH.
pub fn available() -> bool {
    Command::new("dotnet")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The schema errors of `file`, one line each; empty when it is valid.
pub fn validate(file: &Path) -> Result<Vec<String>, String> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/pptx-validate");
    let output = Command::new("dotnet")
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
        )),
    }
}
