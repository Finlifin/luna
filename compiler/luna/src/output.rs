use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub fn write_stdout(dump: &str) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    write_dump(&mut stdout, dump)
        .and_then(|()| stdout.flush())
        .map_err(|error| format!("cannot write stdout: {error}"))
}

/// Publish only a complete dump, leaving an existing output intact on failure.
pub fn write_file(input: &Path, output: &Path, dump: &str) -> Result<(), String> {
    write_file_inner(input, output, dump)
        .map_err(|error| format!("cannot write output '{}': {error}", output.display()))
}

fn write_file_inner(input: &Path, output: &Path, dump: &str) -> io::Result<()> {
    let input_path = input.canonicalize()?;
    let input_metadata = fs::metadata(&input_path)?;
    let (destination, permissions) = match output.canonicalize() {
        Ok(path) => {
            let metadata = fs::metadata(&path)?;
            if input_path == path || same_file(&input_metadata, &metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output must not refer to the input file",
                ));
            }
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output must be a regular file",
                ));
            }
            if metadata.permissions().readonly() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "output is read-only",
                ));
            }
            (path, Some(metadata.permissions()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A dangling symlink is an invalid destination, not a new file.
            match fs::symlink_metadata(output) {
                Ok(_) => return Err(error),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let name = output.file_name().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "output has no file name")
            })?;
            let parent = output
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            (parent.canonicalize()?.join(name), None)
        }
        Err(error) => return Err(error),
    };

    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "output has no parent directory",
        )
    })?;
    let (temporary, mut file) = TemporaryOutput::create(parent, &destination)?;
    write_dump(&mut file, dump)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.sync_all()?;
    drop(file);
    temporary.publish(&destination)
}

fn write_dump(writer: &mut impl Write, dump: &str) -> io::Result<()> {
    writer.write_all(dump.as_bytes())?;
    if !dump.ends_with('\n') {
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        false
    }
}

struct TemporaryOutput {
    path: PathBuf,
    published: bool,
}

impl TemporaryOutput {
    fn create(parent: &Path, destination: &Path) -> io::Result<(Self, File)> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".luna-{}-{id}.tmp", std::process::id()));
            if path == destination {
                continue;
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok((
                        Self {
                            path,
                            published: false,
                        },
                        file,
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot create a unique temporary output file",
        ))
    }

    fn publish(mut self, destination: &Path) -> io::Result<()> {
        fs::rename(&self.path, destination)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_disarms_cleanup_even_when_paths_coincide() {
        let root = std::env::temp_dir();
        let (temporary, mut file) =
            TemporaryOutput::create(&root, &root.join("unused-destination")).unwrap();
        let destination = temporary.path.clone();
        file.write_all(b"complete dump\n").unwrap();
        drop(file);
        temporary.publish(&destination).unwrap();
        let actual = fs::read(&destination);
        let _ = fs::remove_file(&destination);
        assert_eq!(actual.unwrap(), b"complete dump\n");
    }
}
