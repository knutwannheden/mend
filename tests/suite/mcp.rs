use assert_cmd::Command;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

/// Sends `requests` to `patchwork mcp`, one per line, and returns its replies by request id.
fn replies(requests: &[Value]) -> anyhow::Result<Vec<Value>> {
    let input: String = requests
        .iter()
        .map(|request| format!("{request}\n"))
        .collect();
    let output = Command::new(env!("CARGO_BIN_EXE_patchwork"))
        .arg("mcp")
        .write_stdin(input)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn call(id: u64, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
           "params": {"name": "patch", "arguments": arguments}})
}

#[test]
fn test_mcp_lists_the_patch_tool_and_applies_a_patch_in_the_given_directory() -> anyhow::Result<()>
{
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "one\n")?;
    let patch = "*** Begin Patch\n*** Update File: f.txt\n@@\n-one\n+1\n*** End Patch";

    let replies = replies(&[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                          "clientInfo": {"name": "test", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        call(3, json!({"patch": patch, "cwd": tmp.path()})),
    ])?;

    assert_eq!(replies.len(), 3, "{replies:?}");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "patchwork");
    assert_eq!(replies[1]["result"]["tools"][0]["name"], "patch");
    assert_eq!(
        replies[2],
        json!({"jsonrpc": "2.0", "id": 3,
               "result": {"content": [{"type": "text", "text": "M f.txt\n"}], "isError": false}})
    );
    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "1\n");
    Ok(())
}

#[test]
fn test_mcp_call_that_does_not_apply_is_an_error_result_and_writes_nothing() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "one\n")?;
    let patch = "*** Begin Patch\n*** Update File: f.txt\n@@\n-two\n+2\n*** End Patch";

    let replies = replies(&[call(1, json!({"patch": patch, "cwd": tmp.path()}))])?;

    assert_eq!(replies[0]["result"]["isError"], true);
    assert_eq!(
        replies[0]["result"]["content"][0]["text"],
        "f.txt: chunk 1 of 1 matches nothing, and no similar lines were found\n"
    );
    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "one\n");
    Ok(())
}
