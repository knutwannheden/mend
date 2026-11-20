use assert_cmd::Command;
use std::fs;
use tempfile::tempdir;

fn apply_patch_command() -> anyhow::Result<Command> {
    Ok(Command::new(env!("CARGO_BIN_EXE_patchwork")))
}

#[test]
fn test_apply_patch_cli_add_and_update() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_test.txt";
    let absolute_path = tmp.path().join(file);

    // 1) Add a file
    let add_patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );
    apply_patch_command()?
        .arg(add_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(format!("A {file}\n"));
    assert_eq!(fs::read_to_string(&absolute_path)?, "hello\n");

    // 2) Update the file
    let update_patch = format!(
        r#"*** Begin Patch
*** Update File: {file}
@@
-hello
+world
*** End Patch"#
    );
    apply_patch_command()?
        .arg(update_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(format!("M {file}\n"));
    assert_eq!(fs::read_to_string(&absolute_path)?, "world\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_stdin_add_and_update() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_test_stdin.txt";
    let absolute_path = tmp.path().join(file);

    // 1) Add a file via stdin
    let add_patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );
    apply_patch_command()?
        .current_dir(tmp.path())
        .write_stdin(add_patch)
        .assert()
        .success()
        .stdout(format!("A {file}\n"));
    assert_eq!(fs::read_to_string(&absolute_path)?, "hello\n");

    // 2) Update the file via stdin
    let update_patch = format!(
        r#"*** Begin Patch
*** Update File: {file}
@@
-hello
+world
*** End Patch"#
    );
    apply_patch_command()?
        .current_dir(tmp.path())
        .write_stdin(update_patch)
        .assert()
        .success()
        .stdout(format!("M {file}\n"));
    assert_eq!(fs::read_to_string(&absolute_path)?, "world\n");

    Ok(())
}

#[test]
fn test_exit_status_distinguishes_a_patch_that_does_not_fit_from_a_malformed_one()
-> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "one\n")?;

    apply_patch_command()?
        .current_dir(tmp.path())
        .write_stdin("*** Begin Patch\n*** Update File: f.txt\n@@\n-two\n+2\n*** End Patch")
        .assert()
        .code(1);

    apply_patch_command()?
        .current_dir(tmp.path())
        .write_stdin("*** Begin Patch\n*** Update File: f.txt\n@@\n-one\n+1\n")
        .assert()
        .code(2)
        .stderr("Invalid patch: The last line of the patch must be '*** End Patch'\n");

    apply_patch_command()?
        .current_dir(tmp.path())
        .arg("--frobnicate")
        .assert()
        .code(2)
        .stderr(
            "Unknown option: --frobnicate\n\
             Usage: patchwork [-C <dir>] [--check] [--diff] [--fragment] [--all] [--then <command> [--revert]] [PATCH]\n\
             Run `patchwork --help` for the patch format and examples.\n",
        );

    let empty = apply_patch_command()?
        .current_dir(tmp.path())
        .write_stdin("")
        .output()?;
    assert_eq!(empty.status.code(), Some(2));
    assert!(String::from_utf8(empty.stderr)?.starts_with("No patch was given\n"));

    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "one\n");
    Ok(())
}

#[test]
fn test_check_reports_the_changes_without_writing_them() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "one\n")?;

    apply_patch_command()?
        .current_dir(tmp.path())
        .arg("--check")
        .write_stdin(
            "*** Begin Patch\n*** Update File: f.txt\n@@\n-one\n+1\n*** Add File: g.txt\n+new\n*** End Patch",
        )
        .assert()
        .success()
        .stdout("M f.txt\nA g.txt\n");

    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "one\n");
    assert!(!tmp.path().join("g.txt").exists());
    Ok(())
}

#[test]
fn test_diff_prints_a_unified_diff_in_place_of_status_lines() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "one\ntwo\n")?;
    fs::write(tmp.path().join("old.txt"), "bye\n")?;

    apply_patch_command()?
        .current_dir(tmp.path())
        .arg("--diff")
        .write_stdin(
            "*** Begin Patch\n*** Update File: f.txt\n@@\n-one\n+1\n*** Add File: g.txt\n+new\n*** Delete File: old.txt\n*** End Patch",
        )
        .assert()
        .success()
        .stdout(
            "--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\n-one\n+1\n two\n\
             --- /dev/null\n+++ b/g.txt\n@@ -0,0 +1 @@\n+new\n\
             --- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n",
        );

    assert_eq!(fs::read_to_string(tmp.path().join("f.txt"))?, "1\ntwo\n");

    let absolute = tmp.path().canonicalize()?.join("f.txt");
    apply_patch_command()?
        .current_dir(tmp.path())
        .arg("--diff")
        .write_stdin(format!(
            "*** Begin Patch\n*** Update File: {}\n@@\n-two\n+2\n*** End Patch",
            absolute.display()
        ))
        .assert()
        .success()
        .stdout(format!(
            "--- {0}\n+++ {0}\n@@ -1,2 +1,2 @@\n 1\n-two\n+2\n",
            absolute.display()
        ));
    Ok(())
}

#[test]
fn test_help_shows_the_patch_format_and_how_to_chain_a_command() -> anyhow::Result<()> {
    let help = apply_patch_command()?.arg("--help").output()?;
    let stdout = String::from_utf8(help.stdout)?;

    assert_eq!(help.status.code(), Some(0));
    assert!(stdout.contains("*** Update File: <path>"), "{stdout}");
    assert!(stdout.contains("patchwork <<'EOF' && "), "{stdout}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_argument_that_is_not_utf8_is_a_usage_error() -> anyhow::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    apply_patch_command()?
        .arg(std::ffi::OsStr::from_bytes(b"\xff"))
        .assert()
        .code(2);
    Ok(())
}

#[test]
fn test_dash_c_resolves_patch_paths_against_a_directory() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::create_dir_all(tmp.path().join("deep/dir"))?;
    fs::write(tmp.path().join("deep/dir/f.txt"), "one\n")?;

    apply_patch_command()?
        .current_dir(tmp.path())
        .args(["-C", "deep/dir"])
        .write_stdin("*** Begin Patch\n*** Update File: f.txt\n@@\n-one\n+1\n*** End Patch")
        .assert()
        .success()
        .stdout("M f.txt\n");
    assert_eq!(
        fs::read_to_string(tmp.path().join("deep/dir/f.txt"))?,
        "1\n"
    );

    apply_patch_command()?
        .current_dir(tmp.path())
        .args(["-C", "missing"])
        .write_stdin("*** Begin Patch\n*** Add File: g.txt\n+new\n*** End Patch")
        .assert()
        .code(2);
    assert!(!tmp.path().join("missing").exists());
    Ok(())
}

#[test]
fn test_fragment_option_lets_a_chunk_replace_part_of_a_line() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "alpha beta gamma\n")?;
    let patch = "*** Begin Patch\n*** Update File: f.txt\n@@\n-beta\n+BETA\n*** End Patch";

    apply_patch_command()?
        .current_dir(tmp.path())
        .write_stdin(patch)
        .assert()
        .code(1);
    assert_eq!(
        fs::read_to_string(tmp.path().join("f.txt"))?,
        "alpha beta gamma\n"
    );

    apply_patch_command()?
        .current_dir(tmp.path())
        .arg("--fragment")
        .write_stdin(patch)
        .assert()
        .success()
        .stdout("M f.txt\n");
    assert_eq!(
        fs::read_to_string(tmp.path().join("f.txt"))?,
        "alpha BETA gamma\n"
    );
    Ok(())
}

#[test]
fn test_all_option_changes_every_match_and_names_the_lines_on_stderr() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::write(tmp.path().join("f.txt"), "x = 1\ny\nx = 1\n")?;

    apply_patch_command()?
        .current_dir(tmp.path())
        .arg("--all")
        .write_stdin("*** Begin Patch\n*** Update File: f.txt\n@@\n-x = 1\n+x = 2\n*** End Patch")
        .assert()
        .success()
        .stdout("M f.txt\n")
        .stderr("f.txt: chunk 1 of 1 changed 2 places, at lines 1, 3\n");
    assert_eq!(
        fs::read_to_string(tmp.path().join("f.txt"))?,
        "x = 2\ny\nx = 2\n"
    );
    Ok(())
}
