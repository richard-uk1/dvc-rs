//! File abstractions that can be mocked for testing.

use camino::{Utf8Path, Utf8PathBuf};
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};

/// Basic file attributes exposed by [`FileSystem::metadata`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMetadata {
    /// Whether the path refers to a regular file.
    pub is_file: bool,
    /// Whether the path refers to a directory.
    pub is_dir: bool,
    /// The file size in bytes, or zero for directories.
    pub len: u64,
}

/// A file or directory returned by [`FileSystem::read_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// Full path to the entry.
    pub path: Utf8PathBuf,
    /// Whether the entry is a regular file.
    pub is_file: bool,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// A file reader returned by [`FileSystem::open_read`].
#[derive(Debug)]
pub enum FileReader {
    /// Reader for an operating-system file.
    File(File),
    /// Reader over in-memory file contents.
    Memory(Cursor<Vec<u8>>),
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::File(file) => file.read(buf),
            Self::Memory(cursor) => cursor.read(buf),
        }
    }
}

/// Abstraction over filesystem operations used by commands.
///
/// Implementations return UTF-8 paths and report filesystem failures as
/// [`io::Error`]. This trait allows command logic to be tested with an
/// in-memory filesystem.
pub trait FileSystem {
    /// Resolve a path to its canonical absolute form.
    fn canonicalize(&self, path: &Utf8Path) -> io::Result<Utf8PathBuf>;
    /// Read basic attributes for a path.
    fn metadata(&self, path: &Utf8Path) -> io::Result<FileMetadata>;
    /// Open a file for reading.
    fn open_read(&self, path: &Utf8Path) -> io::Result<FileReader>;
    /// Create a directory and any missing parent directories.
    fn create_dir_all(&self, path: &Utf8Path) -> io::Result<()>;
    /// Return whether a path exists.
    fn exists(&self, path: &Utf8Path) -> io::Result<bool>;
    /// Copy a file, returning the number of bytes copied.
    fn copy(&self, from: &Utf8Path, to: &Utf8Path) -> io::Result<u64>;
    /// Create or truncate a file and write all provided contents.
    fn write(&self, path: &Utf8Path, contents: &[u8]) -> io::Result<()>;
    /// Mark a file as executable where the platform supports file permissions.
    fn set_executable(&self, path: &Utf8Path) -> io::Result<()>;
    /// Return the immediate entries in a directory.
    fn read_dir(&self, path: &Utf8Path) -> io::Result<Vec<DirectoryEntry>>;
}

/// Filesystem implementation backed by the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdFileSystem;

impl FileSystem for StdFileSystem {
    fn canonicalize(&self, path: &Utf8Path) -> io::Result<Utf8PathBuf> {
        let path = fs::canonicalize(path)?;
        Utf8PathBuf::try_from(path)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path is not valid UTF-8"))
    }

    fn metadata(&self, path: &Utf8Path) -> io::Result<FileMetadata> {
        let metadata = fs::metadata(path)?;
        Ok(FileMetadata {
            is_file: metadata.is_file(),
            is_dir: metadata.is_dir(),
            len: metadata.len(),
        })
    }

    fn open_read(&self, path: &Utf8Path) -> io::Result<FileReader> {
        Ok(FileReader::File(File::open(path)?))
    }

    fn create_dir_all(&self, path: &Utf8Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn exists(&self, path: &Utf8Path) -> io::Result<bool> {
        path.try_exists()
    }

    fn copy(&self, from: &Utf8Path, to: &Utf8Path) -> io::Result<u64> {
        fs::copy(from, to)
    }

    fn write(&self, path: &Utf8Path, contents: &[u8]) -> io::Result<()> {
        let mut file = File::create(path)?;
        file.write_all(contents)
    }

    fn set_executable(&self, path: &Utf8Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(path)?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(())
        }
    }

    fn read_dir(&self, path: &Utf8Path) -> io::Result<Vec<DirectoryEntry>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = Utf8PathBuf::try_from(entry.path()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory entry path is not valid UTF-8",
                )
            })?;
            entries.push(DirectoryEntry {
                path,
                is_file: file_type.is_file(),
                is_dir: file_type.is_dir(),
            });
        }
        Ok(entries)
    }
}

#[cfg(test)]
pub(crate) mod mock {
    use super::{DirectoryEntry, FileMetadata, FileSystem};
    use camino::{Utf8Path, Utf8PathBuf};
    use std::collections::{BTreeMap, BTreeSet};
    use std::io;
    use std::sync::RwLock;

    #[derive(Debug, Default)]
    pub(crate) struct MockFileSystem {
        files: RwLock<BTreeMap<Utf8PathBuf, Vec<u8>>>,
        directories: RwLock<BTreeSet<Utf8PathBuf>>,
        executable_files: RwLock<BTreeSet<Utf8PathBuf>>,
    }

    impl MockFileSystem {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn add_dir(&self, path: impl AsRef<Utf8Path>) {
            let path = path.as_ref();
            let mut directories = self.directories.write().unwrap();
            let mut current = Utf8PathBuf::new();
            for component in path.components() {
                current.push(component.as_str());
                directories.insert(current.clone());
            }
        }

        pub(crate) fn add_file(&self, path: impl AsRef<Utf8Path>, contents: &[u8]) {
            let path = path.as_ref();
            if let Some(parent) = path.parent() {
                self.add_dir(parent);
            }
            self.files
                .write()
                .unwrap()
                .insert(path.to_path_buf(), contents.to_vec());
        }

        pub(crate) fn read_file(&self, path: impl AsRef<Utf8Path>) -> Option<Vec<u8>> {
            self.files.read().unwrap().get(path.as_ref()).cloned()
        }

        pub(crate) fn is_executable(&self, path: impl AsRef<Utf8Path>) -> bool {
            self.executable_files
                .read()
                .unwrap()
                .contains(path.as_ref())
        }
    }

    impl FileSystem for MockFileSystem {
        fn canonicalize(&self, path: &Utf8Path) -> io::Result<Utf8PathBuf> {
            let exists = self.files.read().unwrap().contains_key(path)
                || self.directories.read().unwrap().contains(path);
            if exists {
                Ok(path.to_path_buf())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "path does not exist",
                ))
            }
        }

        fn metadata(&self, path: &Utf8Path) -> io::Result<FileMetadata> {
            if let Some(contents) = self.files.read().unwrap().get(path) {
                return Ok(FileMetadata {
                    is_file: true,
                    is_dir: false,
                    len: contents.len() as u64,
                });
            }
            if self.directories.read().unwrap().contains(path) {
                return Ok(FileMetadata {
                    is_file: false,
                    is_dir: true,
                    len: 0,
                });
            }
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "path does not exist",
            ))
        }

        fn open_read(&self, path: &Utf8Path) -> io::Result<super::FileReader> {
            let contents = self
                .files
                .read()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "file does not exist"))?;
            Ok(super::FileReader::Memory(std::io::Cursor::new(contents)))
        }

        fn create_dir_all(&self, path: &Utf8Path) -> io::Result<()> {
            self.add_dir(path);
            Ok(())
        }

        fn exists(&self, path: &Utf8Path) -> io::Result<bool> {
            Ok(self.files.read().unwrap().contains_key(path)
                || self.directories.read().unwrap().contains(path))
        }

        fn copy(&self, from: &Utf8Path, to: &Utf8Path) -> io::Result<u64> {
            let contents = self
                .files
                .read()
                .unwrap()
                .get(from)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "source does not exist"))?;
            let len = contents.len() as u64;
            self.files
                .write()
                .unwrap()
                .insert(to.to_path_buf(), contents);
            Ok(len)
        }

        fn write(&self, path: &Utf8Path, contents: &[u8]) -> io::Result<()> {
            self.files
                .write()
                .unwrap()
                .insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }

        fn set_executable(&self, path: &Utf8Path) -> io::Result<()> {
            if self.files.read().unwrap().contains_key(path) {
                self.executable_files
                    .write()
                    .unwrap()
                    .insert(path.to_path_buf());
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "file does not exist",
                ))
            }
        }

        fn read_dir(&self, path: &Utf8Path) -> io::Result<Vec<DirectoryEntry>> {
            if !self.directories.read().unwrap().contains(path) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "directory does not exist",
                ));
            }
            let mut entries = Vec::new();
            for directory in self.directories.read().unwrap().iter() {
                if directory.parent() == Some(path) {
                    entries.push(DirectoryEntry {
                        path: directory.clone(),
                        is_file: false,
                        is_dir: true,
                    });
                }
            }
            for file in self.files.read().unwrap().keys() {
                if file.parent() == Some(path) {
                    entries.push(DirectoryEntry {
                        path: file.clone(),
                        is_file: true,
                        is_dir: false,
                    });
                }
            }
            Ok(entries)
        }
    }
}
