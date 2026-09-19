//! The directories `batch_run` may read and write (`--allow-dir`), and the only way it opens a
//! file.
//!
//! A path from a tool call is untrusted. It is made absolute (a relative path is taken from the
//! first allowed directory), then resolved with every `..` and symbolic link followed, and only a
//! result inside an allowed directory is used. An output file does not exist yet, so its parent is
//! resolved instead, and the file is created with `create_new`, which never follows a link planted
//! in its place. After a file is opened or created, its path is resolved again and checked to be
//! the one that was allowed, so a directory swapped for a link in between is noticed.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::error::CliError;

/// The allowed directories, resolved.
#[derive(Debug)]
pub(super) struct Roots {
    roots: Vec<PathBuf>,
    /// The first directory as given, made absolute but not resolved: what a relative path is
    /// taken from. On Windows a resolved path is a `\\?\` path, in which `/` and `..` are
    /// ordinary characters, so a relative path joined to it would not mean what it says.
    base: Option<PathBuf>,
}

impl Roots {
    /// Resolves each `--allow-dir`.
    ///
    /// # Errors
    ///
    /// A usage error when one does not exist or is not a directory.
    pub(super) fn new(dirs: &[PathBuf]) -> Result<Self, CliError> {
        let mut roots = Vec::with_capacity(dirs.len());
        for dir in dirs {
            let root = fs::canonicalize(dir)
                .ok()
                .filter(|root| root.is_dir())
                .ok_or_else(|| {
                    CliError::usage(format!(
                        "--allow-dir {} is not an existing directory",
                        dir.display()
                    ))
                    .hint("create the directory first, or pass one that exists")
                })?;
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        let base = match dirs.first() {
            Some(first) => Some(std::path::absolute(first).map_err(|error| {
                CliError::usage(format!(
                    "--allow-dir {} cannot be made absolute: {error}",
                    first.display()
                ))
                .hint("pass the directory as an absolute path")
            })?),
            None => None,
        };
        Ok(Self { roots, base })
    }

    /// The allowed directories, as a tool description or an error lists them.
    pub(super) fn listed(&self) -> String {
        self.roots
            .iter()
            .map(|root| format!("`{}`", root.display()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Resolves the path of a file that must exist, such as the rows to read.
    ///
    /// # Errors
    ///
    /// A `path_not_allowed` error when it resolves outside every allowed directory, and a usage
    /// error when it does not exist or is not a file.
    pub(super) fn existing_file(&self, path: &str, argument: &str) -> Result<PathBuf, CliError> {
        let absolute = self.absolute(path, argument)?;
        let resolved = match fs::canonicalize(&absolute) {
            Ok(resolved) => resolved,
            Err(error) => return Err(self.unresolved(path, argument, &absolute, &error)),
        };
        if !self.contains(&resolved) {
            return Err(self.outside(path, argument));
        }
        if !resolved.is_file() {
            return Err(
                CliError::usage(format!("`{argument}` {path} is not a file"))
                    .hint(format!("pass the path of a file as `{argument}`")),
            );
        }
        Ok(resolved)
    }

    /// Resolves the path of a file to be created, such as the records to write. Nothing is
    /// created yet.
    ///
    /// # Errors
    ///
    /// A `path_not_allowed` error when its directory resolves outside every allowed directory,
    /// and a usage error when that directory does not exist or the file already does.
    pub(super) fn new_file(&self, path: &str, argument: &str) -> Result<PathBuf, CliError> {
        let absolute = self.absolute(path, argument)?;
        let mut components = absolute.components();
        let Some(Component::Normal(name)) = components.next_back() else {
            return Err(
                CliError::usage(format!("`{argument}` {path} does not name a file"))
                    .hint("end the path with a file name, e.g. `results.jsonl`"),
            );
        };
        let parent = components.as_path();
        let directory = match fs::canonicalize(parent) {
            Ok(directory) => directory,
            Err(error) => return Err(self.unresolved(path, argument, parent, &error)),
        };
        if !self.contains(&directory) {
            return Err(self.outside(path, argument));
        }
        let resolved = directory.join(name);
        match fs::symlink_metadata(&resolved) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(resolved),
            Ok(_) => Err(CliError::usage(format!(
                "`{argument}` {path} already exists; batch_run never overwrites or appends to a file"
            ))
            .hint(format!("pass a path that does not exist yet for `{argument}`"))),
            Err(error) => Err(CliError::usage(format!(
                "`{argument}` {path} cannot be checked: {error}"
            ))
            .hint(format!("pass another path for `{argument}`"))),
        }
    }

    /// Opens a file [`Roots::existing_file`] resolved, for reading.
    ///
    /// # Errors
    ///
    /// A usage error when it cannot be opened, and a `path_not_allowed` error when it no longer
    /// resolves to the same place.
    pub(super) fn open(&self, resolved: &Path) -> Result<File, CliError> {
        let file = File::open(resolved).map_err(|error| {
            CliError::usage(format!("cannot read {}: {error}", resolved.display()))
                .hint("check that the file is readable")
        })?;
        self.still(resolved, &file)?;
        Ok(file)
    }

    /// Creates a file [`Roots::new_file`] resolved, for writing. It fails if anything, a link
    /// included, has appeared at that path since.
    ///
    /// # Errors
    ///
    /// A usage error when it cannot be created, and a `path_not_allowed` error when it no longer
    /// resolves to the same place.
    pub(super) fn create(&self, resolved: &Path) -> Result<File, CliError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(resolved)
            .map_err(|error| {
                CliError::usage(format!("cannot create {}: {error}", resolved.display()))
                    .hint("pass a path that does not exist yet, in a directory that can be written")
            })?;
        self.still(resolved, &file)?;
        Ok(file)
    }

    /// Checks that a path opened as `file` still resolves to itself, inside an allowed directory,
    /// and, where the platform says, to the very file that was opened.
    fn still(&self, resolved: &Path, file: &File) -> Result<(), CliError> {
        let unchanged = fs::canonicalize(resolved).is_ok_and(|again| again == resolved)
            && self.contains(resolved)
            && same_file(file, resolved);
        if unchanged {
            return Ok(());
        }
        Err(path_not_allowed(format!(
            "{} changed while it was being opened, and now resolves elsewhere; nothing was read or written",
            resolved.display()
        ))
        .hint("do not replace directories with symbolic links while batch_run runs"))
    }

    /// A path made absolute: a relative one is taken from the first allowed directory.
    fn absolute(&self, path: &str, argument: &str) -> Result<PathBuf, CliError> {
        if path.trim().is_empty() {
            return Err(CliError::usage(format!("`{argument}` is empty"))
                .hint(format!("pass a path as `{argument}`")));
        }
        let path = Path::new(path);
        Ok(match &self.base {
            Some(first) if path.is_relative() => first.join(path),
            _ => path.to_path_buf(),
        })
    }

    fn contains(&self, resolved: &Path) -> bool {
        self.roots.iter().any(|root| resolved.starts_with(root))
    }

    /// The error for a path that cannot be resolved. Whether it is missing is said only when its
    /// nearest existing ancestor is inside an allowed directory, so that a tool call cannot learn
    /// what exists anywhere else.
    fn unresolved(
        &self,
        path: &str,
        argument: &str,
        absolute: &Path,
        error: &std::io::Error,
    ) -> CliError {
        let nearest = absolute
            .ancestors()
            .find_map(|ancestor| fs::canonicalize(ancestor).ok());
        if !nearest.is_some_and(|nearest| self.contains(&nearest)) {
            return self.outside(path, argument);
        }
        if error.kind() == ErrorKind::NotFound {
            CliError::usage(format!("`{argument}` {path} does not exist")).hint(format!(
                "check the path; the allowed directories are {}",
                self.listed()
            ))
        } else {
            CliError::usage(format!("`{argument}` {path} cannot be resolved: {error}"))
                .hint(format!("pass another path for `{argument}`"))
        }
    }

    fn outside(&self, path: &str, argument: &str) -> CliError {
        path_not_allowed(format!(
            "`{argument}` {path} is outside the directories this server may use; nothing was read or written"
        ))
        .hint(format!(
            "use a path inside {}; a relative path is taken from the first",
            self.listed()
        ))
    }
}

/// A path a tool call is not allowed to use.
fn path_not_allowed(message: String) -> CliError {
    let mut error = CliError::usage(message);
    error.code = "path_not_allowed";
    error
}

/// Whether `file` is the file at `path`. Only Unix can tell cheaply; elsewhere the resolved path
/// is the only check.
#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    match (file.metadata(), fs::metadata(path)) {
        (Ok(opened), Ok(named)) => opened.dev() == named.dev() && opened.ino() == named.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_: &File, _: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::Roots;

    /// A fresh directory with `allowed/` (holding `rows.jsonl`) and `outside/` (holding
    /// `secret.jsonl`).
    fn sandbox(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("jev-roots-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("allowed/sub")).unwrap();
        fs::create_dir_all(base.join("outside")).unwrap();
        fs::write(base.join("allowed/rows.jsonl"), "{}\n").unwrap();
        fs::write(base.join("allowed/sub/rows.jsonl"), "{}\n").unwrap();
        fs::write(base.join("outside/secret.jsonl"), "{}\n").unwrap();
        base
    }

    fn roots(base: &std::path::Path) -> Roots {
        Roots::new(&[base.join("allowed")]).unwrap()
    }

    #[test]
    fn a_file_inside_is_resolved_whether_the_path_is_relative_or_absolute() {
        let base = sandbox("inside");
        let roots = roots(&base);
        let expected = fs::canonicalize(base.join("allowed/sub/rows.jsonl")).unwrap();

        let relative = roots.existing_file("sub/rows.jsonl", "input").unwrap();
        let absolute = roots
            .existing_file(
                base.join("allowed/sub/rows.jsonl").to_str().unwrap(),
                "input",
            )
            .unwrap();
        let dotted = roots
            .existing_file("sub/../sub/./rows.jsonl", "input")
            .unwrap();

        assert_eq!([&relative, &absolute, &dotted], [&expected; 3]);
        assert!(roots.open(&relative).is_ok());
    }

    #[test]
    fn dot_dot_and_absolute_paths_outside_are_refused_without_saying_what_exists() {
        let base = sandbox("escape");
        let roots = roots(&base);
        let outside = base.join("outside/secret.jsonl");

        for path in [
            "../outside/secret.jsonl".to_owned(),
            "sub/../../outside/secret.jsonl".to_owned(),
            outside.to_str().unwrap().to_owned(),
            base.join("outside/missing.jsonl")
                .to_str()
                .unwrap()
                .to_owned(),
        ] {
            let error = roots.existing_file(&path, "input").unwrap_err();
            assert_eq!(error.code, "path_not_allowed", "{path}");
            assert!(
                error.message.contains("outside"),
                "{path}: {}",
                error.message
            );
            let error = roots.new_file(&path, "out").unwrap_err();
            assert_eq!(error.code, "path_not_allowed", "{path}");
        }

        let missing = roots.existing_file("missing.jsonl", "input").unwrap_err();
        assert_eq!(
            (missing.code, missing.message.as_str()),
            ("usage", "`input` missing.jsonl does not exist")
        );
    }

    #[test]
    fn an_output_must_be_new_and_name_a_file() {
        let base = sandbox("output");
        let roots = roots(&base);

        let fresh = roots.new_file("sub/results.jsonl", "out").unwrap();
        let existing = roots.new_file("rows.jsonl", "out").unwrap_err();
        let directory = roots.new_file("sub/..", "out").unwrap_err();
        let no_parent = roots.new_file("nowhere/results.jsonl", "out").unwrap_err();

        assert_eq!(
            fresh,
            fs::canonicalize(base.join("allowed/sub"))
                .unwrap()
                .join("results.jsonl")
        );
        assert!(
            existing.message.contains("already exists"),
            "{}",
            existing.message
        );
        assert!(
            directory.message.contains("does not name a file"),
            "{}",
            directory.message
        );
        assert!(
            no_parent.message.contains("does not exist"),
            "{}",
            no_parent.message
        );
        roots.create(&fresh).unwrap();
        assert!(
            roots.create(&fresh).is_err(),
            "a file is never created twice"
        );
    }

    #[test]
    fn a_missing_allowed_directory_is_a_usage_error() {
        let base = sandbox("missing-root");

        let error = Roots::new(&[base.join("nope")]).unwrap_err();
        let file = Roots::new(&[base.join("allowed/rows.jsonl")]).unwrap_err();

        assert_eq!((error.code, error.exit.code()), ("usage", 2));
        assert!(error.message.contains("is not an existing directory"));
        assert_eq!(file.code, "usage");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_out_is_refused_for_reading_and_as_the_directory_of_an_output() {
        use std::os::unix::fs::symlink;

        let base = sandbox("symlink");
        let roots = roots(&base);
        symlink(
            base.join("outside/secret.jsonl"),
            base.join("allowed/link.jsonl"),
        )
        .unwrap();
        symlink(base.join("outside"), base.join("allowed/out-dir")).unwrap();
        symlink(
            base.join("outside/new.jsonl"),
            base.join("allowed/dangling.jsonl"),
        )
        .unwrap();
        symlink(
            base.join("allowed/rows.jsonl"),
            base.join("allowed/inner.jsonl"),
        )
        .unwrap();

        let link = roots.existing_file("link.jsonl", "input").unwrap_err();
        let through = roots
            .existing_file("out-dir/secret.jsonl", "input")
            .unwrap_err();
        let output = roots.new_file("out-dir/results.jsonl", "out").unwrap_err();
        let dangling = roots.new_file("dangling.jsonl", "out").unwrap_err();
        let inner = roots.existing_file("inner.jsonl", "input").unwrap();

        assert_eq!(
            [link.code, through.code, output.code],
            ["path_not_allowed"; 3]
        );
        assert!(
            dangling.message.contains("already exists"),
            "{}",
            dangling.message
        );
        assert!(!base.join("outside/new.jsonl").exists());
        assert_eq!(
            inner,
            fs::canonicalize(base.join("allowed/rows.jsonl")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_swapped_for_a_link_after_the_check_is_noticed() {
        use std::os::unix::fs::symlink;

        let base = sandbox("swap");
        let roots = roots(&base);
        let input = roots.existing_file("sub/rows.jsonl", "input").unwrap();
        let output = roots.new_file("sub/results.jsonl", "out").unwrap();
        // The same layout outside, then `sub` replaced by a link to it.
        fs::write(base.join("outside/rows.jsonl"), "{}\n").unwrap();
        fs::remove_dir_all(base.join("allowed/sub")).unwrap();
        symlink(base.join("outside"), base.join("allowed/sub")).unwrap();

        let read = roots.open(&input).unwrap_err();
        let written = roots.create(&output).unwrap_err();

        assert_eq!([read.code, written.code], ["path_not_allowed"; 2]);
    }
}
