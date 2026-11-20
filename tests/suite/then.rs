use assert_cmd::Command;
use pretty_assertions::assert_eq;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;
use tempfile::tempdir;

const PATCH: &str = "*** Begin Patch\n*** Update File: f.txt\n@@\n-old\n+new\n*** Add File: sub/g.txt\n+added\n*** End Patch";

fn patchwork(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_patchwork"));
    cmd.current_dir(dir);
    cmd
}

#[test]
fn then_runs_once_the_patch_is_applied_and_not_when_it_does_not_fit() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "old\n")?;

    patchwork(tmp.path())
        .args(["--then", "cat f.txt"])
        .write_stdin(PATCH)
        .assert()
        .success()
        .stdout("M f.txt\nA sub/g.txt\nnew\n");

    patchwork(tmp.path())
        .args(["--then", "touch ran"])
        .write_stdin(PATCH)
        .assert()
        .code(1);
    assert!(!tmp.path().join("ran").exists());
    Ok(())
}

#[test]
fn failing_then_command_exits_3_and_keeps_the_edits() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "old\n")?;

    patchwork(tmp.path())
        .args(["--then", "exit 7"])
        .write_stdin(PATCH)
        .assert()
        .code(3)
        .stderr("patchwork: the --then command exited with status 7; the edits were kept\n");

    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "new\n");
    Ok(())
}

#[test]
fn revert_restores_the_files_whether_the_command_fails_or_succeeds() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "old\n")?;

    patchwork(tmp.path())
        .args(["--revert", "--then", "cat f.txt; exit 7"])
        .write_stdin(PATCH)
        .assert()
        .code(3)
        .stdout("M f.txt\nA sub/g.txt\nnew\n")
        .stderr("patchwork: the --then command exited with status 7; the files were restored\n");
    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "old\n");
    assert!(!tmp.path().join("sub").exists());

    patchwork(tmp.path())
        .args(["--revert", "--then", "true"])
        .write_stdin(PATCH)
        .assert()
        .success();
    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "old\n");
    Ok(())
}

#[test]
fn revert_leaves_a_file_alone_that_changed_after_the_patch() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "old\n")?;

    patchwork(tmp.path())
        .args(["--revert", "--then", "echo later >> f.txt"])
        .write_stdin(PATCH)
        .assert()
        .code(4)
        .stderr("could not restore f.txt: it changed after the patch was applied\n");

    assert_eq!(
        fs::read_to_string(tmp.path().join("f.txt"))?,
        "new\nlater\n"
    );
    assert!(!tmp.path().join("sub").exists());
    Ok(())
}

#[test]
fn revert_restores_the_files_when_terminated_while_the_command_runs() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = tmp.path().join("f.txt");
    fs::write(&file, "old\n")?;

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_patchwork"))
        .current_dir(tmp.path())
        .args(["--revert", "--then", "sleep 30"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(PATCH.as_bytes())?;

    let started = Instant::now();
    while fs::read_to_string(&file)? != "new\n" {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "patch was not applied"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()?;
    let status = child.wait()?;

    assert_eq!(status.code(), Some(128 + 15));
    assert_eq!(fs::read_to_string(&file)?, "old\n");
    assert!(started.elapsed() < Duration::from_secs(10));
    Ok(())
}

#[test]
fn revert_needs_then_and_then_excludes_check() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "old\n")?;

    patchwork(tmp.path())
        .arg("--revert")
        .write_stdin(PATCH)
        .assert()
        .code(2);
    patchwork(tmp.path())
        .args(["--check", "--then", "touch ran"])
        .write_stdin(PATCH)
        .assert()
        .code(2);

    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "old\n");
    assert!(!tmp.path().join("ran").exists());
    Ok(())
}
