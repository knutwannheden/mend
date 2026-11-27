use assert_cmd::Command;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn run_apply_patch_in_dir(dir: &Path, patch: &str) -> anyhow::Result<assert_cmd::assert::Assert> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mend"));
    cmd.current_dir(dir);
    Ok(cmd.arg(patch).assert())
}

fn assert_apply_patch_updates_file(
    file_name: &str,
    original: &[u8],
    patch: &str,
    expected: &[u8],
) -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let target_path = tmp.path().join(file_name);
    fs::write(&target_path, original)?;

    run_apply_patch_in_dir(tmp.path(), patch)?
        .success()
        .stdout(format!("M {file_name}\n"));

    assert_eq!(fs::read(target_path)?, expected);
    Ok(())
}

fn apply_patch_command(dir: &Path) -> anyhow::Result<Command> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mend"));
    cmd.current_dir(dir);
    Ok(cmd)
}

#[test]
fn test_apply_patch_cli_applies_multiple_operations() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let add_path = tmp.path().join("nested/new.txt");
    let modify_path = tmp.path().join("modify.txt");
    let delete_path = tmp.path().join("delete.txt");

    fs::write(&modify_path, "line1\nline2\n")?;
    fs::write(&delete_path, "obsolete\n")?;

    let patch = "*** Begin Patch\n*** Add File: nested/new.txt\n+created\n*** Delete File: delete.txt\n*** Update File: modify.txt\n@@\n-line2\n+changed\n*** End Patch";

    run_apply_patch_in_dir(tmp.path(), patch)?
        .success()
        .stdout("A nested/new.txt\nD delete.txt\nM modify.txt\n");

    assert_eq!(fs::read_to_string(add_path)?, "created\n");
    assert_eq!(fs::read_to_string(&modify_path)?, "line1\nchanged\n");
    assert!(!delete_path.exists());

    Ok(())
}

#[test]
fn test_apply_patch_cli_applies_multiple_chunks() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let target_path = tmp.path().join("multi.txt");
    fs::write(&target_path, "line1\nline2\nline3\nline4\n")?;

    let patch = "*** Begin Patch\n*** Update File: multi.txt\n@@\n-line2\n+changed2\n@@\n-line4\n+changed4\n*** End Patch";

    run_apply_patch_in_dir(tmp.path(), patch)?
        .success()
        .stdout("M multi.txt\n");

    assert_eq!(
        fs::read_to_string(&target_path)?,
        "line1\nchanged2\nline3\nchanged4\n"
    );

    Ok(())
}

#[test]
fn test_apply_patch_cli_rejects_overlapping_end_of_file_chunks() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let target_path = tmp.path().join("overlapping.txt");
    fs::write(&target_path, "one\n")?;

    let patch = "*** Begin Patch\n*** Update File: overlapping.txt\n@@\n-one\n+first\n@@\n-one\n+second\n*** End of File\n*** End Patch";

    run_apply_patch_in_dir(tmp.path(), patch)?
        .failure()
        .stderr(
            "overlapping.txt: chunk 2 of 2 matches only line 1, which an earlier chunk already matched\n",
        );

    assert_eq!(fs::read_to_string(target_path)?, "one\n");
    Ok(())
}

#[test]
fn test_apply_patch_cli_preserves_crlf_from_target_file() -> anyhow::Result<()> {
    let patch = "*** Begin Patch\n*** Update File: crlf.txt\n@@\n-one\n+uno\n@@\n two\n+\n+between\n three\n*** End Patch";

    assert_apply_patch_updates_file(
        "crlf.txt",
        b"one\r\ntwo\r\nthree\r\n",
        patch,
        b"uno\r\ntwo\r\n\r\nbetween\r\nthree\r\n",
    )
}

#[test]
fn test_apply_patch_cli_appends_after_trailing_blank_crlf_line() -> anyhow::Result<()> {
    let patch = "*** Begin Patch\n*** Update File: trailing_blank.txt\n@@\n+new\n*** End Patch";

    assert_apply_patch_updates_file(
        "trailing_blank.txt",
        b"a\r\n\r\n",
        patch,
        b"a\r\n\r\nnew\r\n",
    )
}

#[test]
fn test_apply_patch_cli_preserves_cr_from_target_file() -> anyhow::Result<()> {
    let patch = "*** Begin Patch\n*** Update File: cr.txt\n@@\n-one\n+uno\n@@\n two\n+\n+between\n three\n*** End Patch";

    assert_apply_patch_updates_file(
        "cr.txt",
        b"one\rtwo\rthree\r",
        patch,
        b"uno\rtwo\r\rbetween\rthree\r",
    )
}

#[test]
fn test_apply_patch_cli_preserves_change_order_with_repeated_lines() -> anyhow::Result<()> {
    let patch =
        "*** Begin Patch\n*** Update File: repeated.txt\n@@\n-a\n-b\n+b\n+b\n+a\n*** End Patch";

    assert_apply_patch_updates_file("repeated.txt", b"a\nb\n", patch, b"b\nb\na\n")
}

#[test]
fn test_apply_patch_cli_preserves_repeated_context_line_ending() -> anyhow::Result<()> {
    let patch =
        "*** Begin Patch\n*** Update File: repeated_context.txt\n@@\n-same\n same\n*** End Patch";

    assert_apply_patch_updates_file("repeated_context.txt", b"same\r\nsame\n", patch, b"same\n")
}

#[test]
fn test_apply_patch_cli_preserves_untouched_mixed_line_endings() -> anyhow::Result<()> {
    let patch = "*** Begin Patch\n*** Update File: mixed.txt\n@@\n one\n two\n-three\n+THREE\n four\n*** End Patch";

    assert_apply_patch_updates_file(
        "mixed.txt",
        b"one\r\ntwo\rthree\nfour\r\n",
        patch,
        b"one\r\ntwo\rTHREE\r\nfour\r\n",
    )
}

#[test]
fn test_apply_patch_cli_uses_crlf_for_new_trailing_newline() -> anyhow::Result<()> {
    let patch =
        "*** Begin Patch\n*** Update File: no_trailing_newline.txt\n@@\n-one\n+ONE\n*** End Patch";

    assert_apply_patch_updates_file(
        "no_trailing_newline.txt",
        b"one\r\ntwo",
        patch,
        b"ONE\r\ntwo\r\n",
    )
}

#[test]
fn test_apply_patch_cli_moves_file_to_new_directory_with_its_permissions() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempdir()?;
    let original_path = tmp.path().join("old/name.txt");
    let new_path = tmp.path().join("renamed/dir/name.txt");
    fs::create_dir_all(original_path.parent().expect("parent should exist"))?;
    fs::write(&original_path, "old content\n")?;
    fs::set_permissions(&original_path, fs::Permissions::from_mode(0o755))?;

    let patch = "*** Begin Patch\n*** Update File: old/name.txt\n*** Move to: renamed/dir/name.txt\n@@\n-old content\n+new content\n*** End Patch";

    run_apply_patch_in_dir(tmp.path(), patch)?
        .success()
        .stdout("D old/name.txt\nA renamed/dir/name.txt\n");

    assert!(!original_path.exists());
    assert_eq!(fs::read_to_string(&new_path)?, "new content\n");
    assert_eq!(fs::metadata(&new_path)?.permissions().mode() & 0o777, 0o755);

    Ok(())
}

#[test]
fn test_apply_patch_cli_rejects_empty_patch() -> anyhow::Result<()> {
    let tmp = tempdir()?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** End Patch")
        .assert()
        .code(2)
        .stderr("Invalid patch: The patch must contain at least one hunk\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_reports_missing_context() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let target_path = tmp.path().join("modify.txt");
    fs::write(&target_path, "line1\nline2\n")?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** Update File: modify.txt\n@@\n-missing\n+changed\n*** End Patch")
        .assert()
        .failure()
        .stderr("modify.txt: chunk 1 of 1 matches nothing, and no similar lines were found\n");
    assert_eq!(fs::read_to_string(&target_path)?, "line1\nline2\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_rejects_missing_file_delete() -> anyhow::Result<()> {
    let tmp = tempdir()?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** Delete File: missing.txt\n*** End Patch")
        .assert()
        .failure()
        .stderr("missing.txt: cannot delete, no such file\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_rejects_empty_update_hunk() -> anyhow::Result<()> {
    let tmp = tempdir()?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** Update File: foo.txt\n*** End Patch")
        .assert()
        .failure()
        .stderr("Invalid patch hunk on line 2: Update file hunk for path 'foo.txt' is empty\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_requires_existing_file_for_update() -> anyhow::Result<()> {
    let tmp = tempdir()?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** Update File: missing.txt\n@@\n-old\n+new\n*** End Patch")
        .assert()
        .failure()
        .stderr("missing.txt: cannot update, no such file\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_move_overwrites_existing_destination() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let original_path = tmp.path().join("old/name.txt");
    let destination = tmp.path().join("renamed/dir/name.txt");
    fs::create_dir_all(original_path.parent().expect("parent should exist"))?;
    fs::create_dir_all(destination.parent().expect("parent should exist"))?;
    fs::write(&original_path, "from\n")?;
    fs::write(&destination, "existing\n")?;

    run_apply_patch_in_dir(
        tmp.path(),
        "*** Begin Patch\n*** Update File: old/name.txt\n*** Move to: renamed/dir/name.txt\n@@\n-from\n+new\n*** End Patch",
    )?
    .success()
    .stdout("D old/name.txt\nM renamed/dir/name.txt\n");

    assert!(!original_path.exists());
    assert_eq!(fs::read_to_string(&destination)?, "new\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_add_overwrites_existing_file() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let path = tmp.path().join("duplicate.txt");
    fs::write(&path, "old content\n")?;

    run_apply_patch_in_dir(
        tmp.path(),
        "*** Begin Patch\n*** Add File: duplicate.txt\n+new content\n*** End Patch",
    )?
    .success()
    .stdout("M duplicate.txt\n");

    assert_eq!(fs::read_to_string(&path)?, "new content\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_delete_directory_fails() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let dir = tmp.path().join("dir");
    fs::create_dir(&dir)?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** Delete File: dir\n*** End Patch")
        .assert()
        .failure()
        .stderr("dir: cannot delete, is a directory\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_rejects_invalid_hunk_header() -> anyhow::Result<()> {
    let tmp = tempdir()?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** Frobnicate File: foo\n*** End Patch")
        .assert()
        .failure()
        .stderr("Invalid patch hunk on line 2: '*** Frobnicate File: foo' is not a valid hunk header. Valid hunk headers: '*** Add File: {path}', '*** Delete File: {path}', '*** Update File: {path}'\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_updates_file_appends_trailing_newline() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let target_path = tmp.path().join("no_newline.txt");
    fs::write(&target_path, "no newline at end")?;

    run_apply_patch_in_dir(
        tmp.path(),
        "*** Begin Patch\n*** Update File: no_newline.txt\n@@\n-no newline at end\n+first line\n+second line\n*** End Patch",
    )?
    .success()
    .stdout("M no_newline.txt\n");

    let contents = fs::read_to_string(&target_path)?;
    assert!(contents.ends_with('\n'));
    assert_eq!(contents, "first line\nsecond line\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_failure_leaves_no_file_changed() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let modify = tmp.path().join("modify.txt");
    fs::write(&modify, "old\n")?;

    apply_patch_command(tmp.path())?
        .arg("*** Begin Patch\n*** Add File: created.txt\n+hello\n*** Update File: modify.txt\n@@\n-old\n+new\n*** Update File: missing.txt\n@@\n-old\n+new\n*** End Patch")
        .assert()
        .failure()
        .stdout("")
        .stderr("missing.txt: cannot update, no such file\n");

    assert_eq!(fs::read_to_string(&modify)?, "old\n");
    assert!(!tmp.path().join("created.txt").exists());

    Ok(())
}

#[test]
fn test_apply_patch_cli_later_hunks_see_earlier_hunks() -> anyhow::Result<()> {
    let tmp = tempdir()?;

    run_apply_patch_in_dir(
        tmp.path(),
        "*** Begin Patch\n*** Add File: new/file.txt\n+original\n*** Update File: new/file.txt\n*** Move to: moved/file.txt\n@@\n-original\n+changed\n*** Add File: remove.txt\n+remove\n*** Delete File: remove.txt\n*** End Patch",
    )?
    .success()
    .stdout("A moved/file.txt\n");

    assert_eq!(
        fs::read_to_string(tmp.path().join("moved/file.txt"))?,
        "changed\n"
    );
    assert!(!tmp.path().join("new").exists());
    assert!(!tmp.path().join("remove.txt").exists());

    Ok(())
}

#[test]
fn test_apply_patch_cli_two_spellings_of_a_path_name_one_file() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    fs::create_dir(tmp.path().join("src"))?;

    run_apply_patch_in_dir(
        tmp.path(),
        "*** Begin Patch\n*** Add File: src/new.txt\n+original\n*** Update File: src/../src/new.txt\n@@\n-original\n+changed\n*** End Patch",
    )?
    .success()
    .stdout("A src/new.txt\n");

    assert_eq!(
        fs::read_to_string(tmp.path().join("src/new.txt"))?,
        "changed\n"
    );

    Ok(())
}

#[test]
fn test_apply_patch_cli_failed_write_restores_earlier_writes() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    let tmp = tempdir()?;
    let root = tmp.path();
    fs::write(root.join("a.txt"), "old\n")?;
    fs::write(root.join("target.txt"), "target\n")?;
    symlink("target.txt", root.join("link.txt"))?;
    let locked = root.join("locked");
    fs::create_dir(&locked)?;
    fs::write(locked.join("gone.txt"), "gone\n")?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555))?;

    let assert = apply_patch_command(root)?
        .arg("*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** Add File: sub/added.txt\n+added\n*** Delete File: link.txt\n*** Delete File: locked/gone.txt\n*** End Patch")
        .assert();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))?;
    assert.code(4).stdout("");

    assert_eq!(fs::read_to_string(root.join("a.txt"))?, "old\n");
    assert_eq!(
        fs::read_link(root.join("link.txt"))?,
        Path::new("target.txt")
    );
    let mut names: Vec<_> = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()?;
    names.sort();
    assert_eq!(names, ["a.txt", "link.txt", "locked", "target.txt"]);

    Ok(())
}

#[test]
fn test_apply_patch_cli_failed_directory_creation_leaves_no_directory() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let too_long = "x".repeat(300);

    run_apply_patch_in_dir(
        tmp.path(),
        &format!("*** Begin Patch\n*** Add File: new/{too_long}/f.txt\n+f\n*** End Patch"),
    )?
    .code(4);

    assert!(!tmp.path().join("new").exists());
    Ok(())
}

#[test]
fn test_apply_patch_cli_rejects_writing_a_link_and_its_target() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let root = tmp.path();
    fs::write(root.join("target.txt"), "one\n")?;
    std::os::unix::fs::symlink("target.txt", root.join("link.txt"))?;

    run_apply_patch_in_dir(
        root,
        "*** Begin Patch\n*** Update File: link.txt\n@@\n-one\n+two\n*** Update File: target.txt\n@@\n-one\n+three\n*** End Patch",
    )?
    .code(1)
    .stderr("target.txt: cannot write, link.txt names the same file\n");

    assert_eq!(fs::read_to_string(root.join("target.txt"))?, "one\n");
    Ok(())
}

#[test]
fn test_apply_patch_cli_update_keeps_symlink_and_permissions() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    let tmp = tempdir()?;
    let root = tmp.path();
    fs::write(root.join("script.sh"), "echo old\n")?;
    fs::set_permissions(root.join("script.sh"), fs::Permissions::from_mode(0o755))?;
    symlink("script.sh", root.join("link.sh"))?;

    run_apply_patch_in_dir(
        root,
        "*** Begin Patch\n*** Update File: link.sh\n@@\n-echo old\n+echo new\n*** End Patch",
    )?
    .success();

    assert_eq!(fs::read_link(root.join("link.sh"))?, Path::new("script.sh"));
    assert_eq!(fs::read_to_string(root.join("script.sh"))?, "echo new\n");
    assert_eq!(
        fs::metadata(root.join("script.sh"))?.permissions().mode() & 0o777,
        0o755
    );

    Ok(())
}

#[test]
fn test_apply_patch_cli_accepts_relative_and_absolute_paths() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let root = tmp.path().canonicalize()?;
    let absolute_add = root.join("absolute-add.txt");
    let absolute_delete = root.join("absolute-delete.txt");
    let absolute_update = root.join("absolute-update.txt");
    fs::write(&absolute_delete, "delete absolute\n")?;
    fs::write(&absolute_update, "absolute old\n")?;
    fs::write(root.join("relative-update.txt"), "relative old\n")?;

    let patch = format!(
        "*** Begin Patch\n*** Add File: {}\n+absolute add\n*** Delete File: {}\n*** Update File: relative-update.txt\n@@\n-relative old\n+relative new\n*** Update File: {}\n@@\n-absolute old\n+absolute new\n*** End Patch",
        absolute_add.display(),
        absolute_delete.display(),
        absolute_update.display(),
    );
    run_apply_patch_in_dir(&root, &patch)?
        .success()
        .stdout(format!(
            "A {}\nD {}\nM relative-update.txt\nM {}\n",
            absolute_add.display(),
            absolute_delete.display(),
            absolute_update.display(),
        ));

    assert_eq!(fs::read_to_string(&absolute_add)?, "absolute add\n");
    assert!(!absolute_delete.exists());
    assert_eq!(
        fs::read_to_string(root.join("relative-update.txt"))?,
        "relative new\n"
    );
    assert_eq!(fs::read_to_string(&absolute_update)?, "absolute new\n");
    Ok(())
}
