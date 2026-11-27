//! Serves mend as a tool over the Model Context Protocol on stdin and stdout, so an agent
//! passes a patch as an argument instead of through a quoted heredoc.

use std::io::BufRead;
use std::io::Write;
use std::process::ExitCode;

use crate::ApplyPatchError;
use crate::Matching;
use crate::Plan;
use crate::parse_patch;
use serde_json::Value;
use serde_json::json;

/// The protocol versions this server can answer in, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const DESCRIPTION: &str =
    "Applies a patch in the Codex apply_patch format. Every chunk is checked \
before anything is written, and nothing is written when one does not fit.

*** Begin Patch
*** Update File: <path>
@@ <anchor line>
 context line
-removed line
+added line
*** Add File: <path>
+line
*** Delete File: <path>
*** End Patch

Each chunk matches exactly one place, and added lines go exactly where they stand among its \
lines. An `@@` anchor names one line of the file, whole or by its start. A chunk that repeats \
the anchor line is placed around it. Any other chunk goes below the anchor and above the next \
one, and a chunk of only `+` lines goes right after it. To insert before a line, give that line \
as context after the `+` lines. A failed call wrote nothing, and its message names the candidate \
places or the closest one.";

/// Answers one JSON-RPC message per line until stdin ends.
pub fn serve() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().split(b'\n') {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        if line.trim_ascii().is_empty() {
            continue;
        }
        let reply = match serde_json::from_slice::<Value>(&line) {
            Ok(message) => answer(&message),
            Err(error) => Some(json!({"jsonrpc": "2.0", "id": null,
                                      "error": {"code": -32700, "message": error.to_string()}})),
        };
        if let Some(reply) = reply
            && writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// The reply to `message`, or `None` for a notification, which has no id and gets no reply.
fn answer(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let params = &message["params"];
    let outcome = match message["method"].as_str().unwrap_or_default() {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"]
                .as_str()
                .filter(|asked| PROTOCOL_VERSIONS.contains(asked))
                .unwrap_or(PROTOCOL_VERSIONS[0]),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mend", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": [tool()]})),
        "tools/call" if params["name"] == "patch" => Ok(call(&params["arguments"])),
        "tools/call" => Err((-32602, format!("Unknown tool: {}", params["name"]))),
        method => Err((-32601, format!("Unknown method: {method}"))),
    };
    Some(match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    })
}

fn tool() -> Value {
    let flag = |description: &str| json!({"type": "boolean", "description": description});
    json!({
        "name": "patch",
        "title": "Apply patch",
        "description": DESCRIPTION,
        // Codex weighs these when it decides whether a call needs the user's approval.
        "annotations": {"readOnlyHint": false, "destructiveHint": true, "openWorldHint": false},
        "inputSchema": {
            "type": "object",
            "properties": {
                "patch": {"type": "string",
                          "description": "The patch, from `*** Begin Patch` to `*** End Patch`"},
                "cwd": {"type": "string",
                        "description": "The directory that relative paths in the patch are \
                                        resolved against. Defaults to the server's directory."},
                "check": flag("Report what the patch would change without writing anything"),
                "diff": flag("Return a diff in git's format in place of the status lines"),
                "fragment": flag("Let a chunk of one `-` and one `+` line replace part of a line \
                                  when no whole line matches"),
                "all": flag("Let a chunk that matches several places change all of them"),
            },
            "required": ["patch"],
        },
    })
}

fn call(arguments: &Value) -> Value {
    let (text, failed) = match apply(arguments) {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({"content": [{"type": "text", "text": text}], "isError": failed})
}

/// Applies the patch the arguments give, returning what the command line would print.
fn apply(arguments: &Value) -> Result<String, String> {
    let patch = arguments["patch"]
        .as_str()
        .ok_or("The `patch` argument is required\n")?;
    let flag = |name: &str| arguments[name].as_bool().unwrap_or(false);
    let cwd = std::env::current_dir()
        .map_err(|error| format!("Failed to determine the current directory: {error}\n"))?;
    let base = match arguments["cwd"].as_str() {
        Some(dir) => cwd.join(dir),
        None => cwd,
    };
    if !base.is_dir() {
        return Err(format!("cwd: {} is not a directory\n", base.display()));
    }
    let fail = |error: ApplyPatchError| format!("{}\n", crate::describe(&error));
    let hunks = parse_patch(patch).map_err(|error| fail(error.into()))?;
    let matching = Matching {
        fragments: flag("fragment"),
        all: flag("all"),
    };
    let mut plan = Plan::compute(&hunks, &base, matching).map_err(fail)?;
    if !flag("check") {
        crate::commit_uninterrupted(&mut plan).map_err(fail)?;
    }
    let mut text = String::new();
    for change in plan.changes() {
        match flag("diff") {
            true => text.push_str(&change.unified_diff()),
            false => text.push_str(&format!(
                "{} {}\n",
                change.status(),
                change.label().display()
            )),
        }
    }
    for note in plan.notes() {
        text.push_str(&format!("{note}\n"));
    }
    Ok(text)
}
