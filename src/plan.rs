use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use crate::ApplyPatchError;
use crate::Hunk;
use crate::file_update::derive_new_contents_from_chunks;
use similar::TextDiff;

/// The complete set of file writes and deletions a patch will make, computed without touching disk.
pub struct Plan {
    changes: Vec<FileChange>,
    /// Directories that `commit` created, outermost first.
    created_dirs: Vec<PathBuf>,
}

/// One path's contents before and after the patch.
pub struct FileChange {
    /// The path as the patch spells it.
    label: PathBuf,
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
    /// The link target when `path` is a symbolic link.
    link: Option<PathBuf>,
}

impl FileChange {
    /// `A` for a path that comes into existence, `D` for one that goes away, `M` otherwise.
    pub fn status(&self) -> char {
        match (&self.before, &self.after) {
            (None, _) => 'A',
            (_, None) => 'D',
            _ => 'M',
        }
    }

    pub fn label(&self) -> &Path {
        &self.label
    }

    /// A unified diff of the change, with `/dev/null` standing for a path that does not exist.
    pub fn unified_diff(&self) -> String {
        let name = |contents: &Option<Vec<u8>>, prefix: &str| match contents {
            Some(_) => format!("{prefix}/{}", self.label.display()),
            None => "/dev/null".to_string(),
        };
        let text = |contents: &Option<Vec<u8>>| {
            String::from_utf8_lossy(contents.as_deref().unwrap_or_default()).into_owned()
        };
        TextDiff::from_lines(&text(&self.before), &text(&self.after))
            .unified_diff()
            .header(&name(&self.before, "a"), &name(&self.after, "b"))
            .to_string()
    }

    /// Where new contents go, which for a symbolic link is the file it points to.
    fn write_path(&self) -> PathBuf {
        match &self.link {
            Some(_) => fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone()),
            None => self.path.clone(),
        }
    }

    fn temp_path(&self) -> PathBuf {
        let target = self.write_path();
        let mut name = std::ffi::OsString::from(".");
        name.push(target.file_name().unwrap_or_default());
        name.push(format!(".patchwork-{}", std::process::id()));
        target.with_file_name(name)
    }

    fn undo(&self) -> io::Result<()> {
        match (&self.before, &self.link) {
            (None, _) => match fs::remove_file(self.write_path()) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
            (Some(_), Some(link)) if self.after.is_none() => symlink(link, &self.path),
            (Some(before), _) => {
                let target = self.write_path();
                fs::write(&target, before)?;
                match &self.permissions {
                    Some(permissions) => fs::set_permissions(&target, permissions.clone()),
                    None => Ok(()),
                }
            }
        }
    }
}

#[cfg(unix)]
fn symlink(link: &Path, path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(link, path)
}

#[cfg(not(unix))]
fn symlink(_link: &Path, _path: &Path) -> io::Result<()> {
    Err(io::Error::other("cannot recreate a symbolic link"))
}

impl Plan {
    /// Works out every change the hunks make, with later hunks seeing the result of earlier ones.
    pub fn compute(hunks: &[Hunk], cwd: &Path) -> Result<Plan, ApplyPatchError> {
        let mut plan = Plan {
            changes: Vec::new(),
            created_dirs: Vec::new(),
        };
        for hunk in hunks {
            match hunk {
                Hunk::AddFile { path, contents } => {
                    let index = plan.change_for(path, cwd, "write")?;
                    plan.changes[index].after = Some(contents.clone().into_bytes());
                }
                Hunk::DeleteFile { path } => {
                    let index = plan.change_for(path, cwd, "delete")?;
                    plan.existing(index, "delete")?;
                    plan.changes[index].after = None;
                }
                Hunk::UpdateFile {
                    path,
                    move_path,
                    chunks,
                } => {
                    let index = plan.change_for(path, cwd, "update")?;
                    let original = String::from_utf8(plan.existing(index, "update")?.to_vec())
                        .map_err(|_| does_not_apply(path, "cannot update, not valid UTF-8"))?;
                    let new_contents = derive_new_contents_from_chunks(
                        &path.display().to_string(),
                        &original,
                        chunks,
                    )?
                    .into_bytes();
                    match move_path {
                        Some(dest) => {
                            plan.changes[index].after = None;
                            let dest_index = plan.change_for(dest, cwd, "write")?;
                            plan.changes[dest_index].after = Some(new_contents);
                        }
                        None => plan.changes[index].after = Some(new_contents),
                    }
                }
            }
        }
        plan.changes.retain(|change| change.before != change.after);
        Ok(plan)
    }

    /// Returns the index of the change for `label`, reading the path's current state on first use.
    fn change_for(
        &mut self,
        label: &Path,
        cwd: &Path,
        action: &str,
    ) -> Result<usize, ApplyPatchError> {
        let path = cwd.join(label);
        if let Some(index) = self.changes.iter().position(|change| change.path == path) {
            return Ok(index);
        }

        let link = fs::read_link(&path).ok();
        let (before, permissions) = match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                return Err(does_not_apply(
                    label,
                    &format!("cannot {action}, is a directory"),
                ));
            }
            Ok(metadata) => {
                let contents = fs::read(&path).map_err(|source| {
                    ApplyPatchError::io(format!("Failed to read {}", label.display()), source)
                })?;
                (Some(contents), Some(metadata.permissions()))
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => (None, None),
            Err(source) => {
                return Err(ApplyPatchError::io(
                    format!("Failed to read {}", label.display()),
                    source,
                ));
            }
        };
        self.changes.push(FileChange {
            label: label.to_path_buf(),
            path,
            after: before.clone(),
            before,
            permissions,
            link,
        });
        Ok(self.changes.len() - 1)
    }

    /// Returns the contents the path has at this point in the patch, or an error if it has none.
    fn existing(&self, index: usize, action: &str) -> Result<&[u8], ApplyPatchError> {
        let change = &self.changes[index];
        change
            .after
            .as_deref()
            .ok_or_else(|| does_not_apply(&change.label, &format!("cannot {action}, no such file")))
    }

    pub fn changes(&self) -> &[FileChange] {
        &self.changes
    }

    /// Writes every change, or leaves every file as it was.
    ///
    /// New contents are staged in temporary files beside their targets before the first
    /// file is replaced, so a failure to write leaves nothing to undo. A failure while
    /// replacing files undoes the replacements already made.
    pub fn commit(&mut self) -> Result<(), ApplyPatchError> {
        let staged = self.stage();
        let swapped = staged.and_then(|()| self.swap());
        if let Err((context, source)) = swapped {
            for change in &self.changes {
                let _ = fs::remove_file(change.temp_path());
            }
            self.remove_created_dirs();
            return Err(ApplyPatchError::io(context, source));
        }
        Ok(())
    }

    fn stage(&mut self) -> Result<(), (String, io::Error)> {
        for index in 0..self.changes.len() {
            let change = &self.changes[index];
            let Some(after) = &change.after else {
                continue;
            };
            let fail = |source| {
                (
                    format!("Failed to write {}", change.label.display()),
                    source,
                )
            };
            let temp = change.temp_path();
            self.created_dirs
                .extend(create_parent_dirs(&temp).map_err(fail)?);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(fail)?;
            file.write_all(after).map_err(fail)?;
            if let Some(permissions) = &change.permissions {
                file.set_permissions(permissions.clone()).map_err(fail)?;
            }
        }
        Ok(())
    }

    fn swap(&self) -> Result<(), (String, io::Error)> {
        for (index, change) in self.changes.iter().enumerate() {
            let result = match &change.after {
                Some(_) => fs::rename(change.temp_path(), change.write_path()),
                None => fs::remove_file(&change.path),
            };
            if let Err(source) = result {
                let verb = if change.after.is_some() {
                    "write"
                } else {
                    "delete"
                };
                let mut context = format!("Failed to {verb} {}", change.label.display());
                if let Err(undo_error) = undo(&self.changes[..index]) {
                    context = format!("{context} (and {undo_error})");
                }
                return Err((context, source));
            }
        }
        Ok(())
    }

    /// Puts every file back as it was before `commit`.
    pub fn restore(&mut self) -> Result<(), ApplyPatchError> {
        let undone = undo(&self.changes);
        self.remove_created_dirs();
        undone.map_err(ApplyPatchError::Restore)
    }

    fn remove_created_dirs(&mut self) {
        for dir in self.created_dirs.drain(..).rev() {
            let _ = fs::remove_dir(dir);
        }
    }
}

/// Undoes the changes in reverse order and reports the files that could not be restored.
fn undo(changes: &[FileChange]) -> Result<(), String> {
    let failures: Vec<String> = changes
        .iter()
        .rev()
        .filter_map(|change| {
            let error = change.undo().err()?;
            Some(format!("{}: {error}", change.label.display()))
        })
        .collect();
    match failures.is_empty() {
        true => Ok(()),
        false => Err(format!("could not restore {}", failures.join(", "))),
    }
}

/// Creates the missing ancestors of `path` and returns them, outermost first.
fn create_parent_dirs(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut dir = path.parent();
    while let Some(current) = dir
        && !current.as_os_str().is_empty()
        && !current.exists()
    {
        missing.push(current.to_path_buf());
        dir = current.parent();
    }
    missing.reverse();
    if let Some(innermost) = missing.last() {
        fs::create_dir_all(innermost)?;
    }
    Ok(missing)
}

fn does_not_apply(label: &Path, problem: &str) -> ApplyPatchError {
    ApplyPatchError::DoesNotApply(format!("{}: {problem}", label.display()))
}
