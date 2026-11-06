use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};

use bytes::Bytes;
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use thiserror::Error;
use zip::ZipArchive;

#[derive(Error, Debug)]
pub enum TreeError {
    #[error("Entry already exists.")]
    AlreadyExists,

    #[error("Parent entry does not exist.")]
    ParentMissing,

    #[error("Parent entry is not a directory.")]
    ParentNotADir,

    #[error("Given path is missing filename.")]
    MissingFilename,

    #[error(transparent)]
    ZipError(#[from] zip::result::ZipError),

    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    NarError(#[from] nix_archive::nar::Error),

    #[error(transparent)]
    Utf8PathError(#[from] camino::FromPathError),

    #[error(transparent)]
    Utf8StrError(#[from] std::str::Utf8Error),
}

#[derive(Debug, PartialEq)]
pub struct Tree(Node);

#[derive(Debug, PartialEq)]
pub enum Node {
    File(Bytes),
    Executable(Bytes),
    Symlink(Utf8PathBuf),
    Directory(BTreeMap<String, Self>),
}

impl Node {
    #[must_use]
    pub const fn new_dir() -> Self {
        Self::Directory(BTreeMap::new())
    }
}

impl Tree {
    #[must_use]
    pub const fn new() -> Self {
        Self(Node::new_dir())
    }

    #[must_use]
    pub fn new_file(data: &[u8]) -> Self {
        Self(Node::File(Bytes::copy_from_slice(data)))
    }

    pub fn from_zip(data: &[u8]) -> Result<Self, TreeError> {
        let mut archive = ZipArchive::new(Cursor::new(data))?;
        let mut tree = Self::new();

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let path = Utf8PathBuf::from(entry.name());

            tree.insert(
                &path,
                if entry.is_dir() {
                    Node::new_dir()
                } else if entry.is_symlink() {
                    let mut buf = String::new();
                    entry.read_to_string(&mut buf)?;
                    Node::Symlink(buf.into())
                } else {
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf)?;
                    if entry.unix_mode().is_some_and(|m| m & 0o111 != 0) {
                        Node::Executable(buf.into())
                    } else {
                        Node::File(buf.into())
                    }
                },
            )?;
        }

        tree.strip_single_top_level_dir();

        Ok(tree)
    }

    pub fn get(&self, path: &Utf8Path) -> Option<&Node> {
        let mut current = &self.0;

        for component in path.components() {
            let Utf8Component::Normal(name) = component else {
                return None;
            };
            let Node::Directory(children) = current else {
                return None;
            };
            current = children.get(name)?;
        }

        Some(current)
    }

    pub fn get_mut(&mut self, path: &Utf8Path) -> Option<&mut Node> {
        let mut current = &mut self.0;

        for component in path.components() {
            let Utf8Component::Normal(name) = component else {
                return None;
            };
            let Node::Directory(children) = current else {
                return None;
            };
            current = children.get_mut(name)?;
        }

        Some(current)
    }

    pub fn insert(&mut self, path: &Utf8Path, node: Node) -> Result<(), TreeError> {
        if self.get(path).is_some() {
            return Err(TreeError::AlreadyExists);
        }

        let parent = match path.parent() {
            None => &mut self.0,
            Some(p) => self.get_mut(p).ok_or(TreeError::ParentMissing)?,
        };

        if let Node::Directory(dir) = parent {
            dir.insert(
                path.file_name().ok_or(TreeError::MissingFilename)?.into(),
                node,
            );
            Ok(())
        } else {
            Err(TreeError::ParentNotADir)
        }
    }

    fn strip_single_top_level_dir(&mut self) {
        let Node::Directory(children) = &mut self.0 else {
            return;
        };
        if children.len() != 1 {
            return;
        }
        let Some(key) = children.keys().next().cloned() else {
            return;
        };
        let Some(Node::Directory(inner)) = children.remove(&key) else {
            return;
        };
        *children = inner;
    }

    pub fn into_nar(&self) -> Result<Bytes, TreeError> {
        fn encode(
            enc: &mut nix_archive::nar::Encoder<Vec<u8>>,
            name: Option<&[u8]>,
            node: &Node,
        ) -> Result<(), nix_archive::nar::Error> {
            match node {
                Node::Directory(children) => {
                    enc.start_directory(name)?;
                    for (name, node) in children {
                        encode(enc, Some(name.as_bytes()), node)?;
                    }
                    enc.end_directory()
                }
                Node::File(c) => enc.regular(name, false, c),
                Node::Executable(c) => enc.regular(name, true, c),
                Node::Symlink(t) => enc.symlink(name, t.as_str().as_bytes()),
            }
        }

        let mut enc = nix_archive::nar::Encoder::new(Vec::new())?;
        encode(&mut enc, None, &self.0)?;
        Ok(Bytes::from(enc.finish()?))
    }

    pub fn is_dir(&self) -> bool {
        matches!(self, Tree(Node::Directory(_)))
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}
