//! Writing files in the vault. A file is replaced whole: the new bytes go to
//! a temporary file outside the vault, which is then renamed into place, so
//! a reader sees the old file or the new one, never half of one. Nothing is
//! ever overwritten by a create, a rename or a move to the trash.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::path::VaultPath;
use crate::vault::Vault;

/// Why a write did not happen.
#[derive(Debug)]
pub enum WriteError {
    /// The file's bytes are not the ones the change was worked out from.
    Changed,
    /// The destination is taken.
    Exists,
    Io(io::Error),
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> WriteError {
        WriteError::Io(e)
    }
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Changed => f.write_str("the file changed while the change was being made"),
            WriteError::Exists => f.write_str("the destination already exists"),
            WriteError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Where temporary files go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmpPlacement {
    /// The temporary folder, on the vault's filesystem.
    TmpDir(PathBuf),
    /// The temporary folder is on another filesystem, so a rename from it
    /// cannot work: temporary files are written as hidden files (`.name`,
    /// which Obsidian ignores) next to their target instead.
    BesideTarget,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_name(hidden: bool) -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{}obsidian-mcp-{}-{nanos}-{n}.tmp", if hidden { "." } else { "" }, std::process::id())
}

fn is_cross_device(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::CrossesDevices
}

impl Vault {
    /// The default temporary folder: a hidden sibling of the vault folder.
    pub(crate) fn default_tmp_dir(root: &Path) -> PathBuf {
        let name = root.file_name().map_or("vault".into(), |n| n.to_string_lossy().into_owned());
        root.parent().unwrap_or(root).join(format!(".{name}.obsidian-mcp-tmp"))
    }

    /// Creates the temporary folder and tells where temporary files will go.
    pub fn prepare_tmp_dir(&self) -> io::Result<TmpPlacement> {
        fs::create_dir_all(&self.tmp_dir)?;
        let probe = self.tmp_dir.join(tmp_name(false));
        fs::write(&probe, b"")?;
        let target = self.root.join(tmp_name(true));
        let placement = match fs::rename(&probe, &target) {
            Ok(()) => {
                fs::remove_file(&target)?;
                TmpPlacement::TmpDir(self.tmp_dir.clone())
            }
            Err(e) if is_cross_device(&e) => {
                let _ = fs::remove_file(&probe);
                TmpPlacement::BesideTarget
            }
            Err(e) => {
                let _ = fs::remove_file(&probe);
                return Err(e);
            }
        };
        Ok(placement)
    }

    fn full(&self, path: &VaultPath) -> PathBuf {
        self.root.join(path.as_str())
    }

    /// Writes `bytes` to a new temporary file (permissions `perms` when
    /// given) and hands it to `place`; when that fails because the
    /// temporary folder is on another filesystem, tries again with a hidden
    /// temporary file beside `dest`.
    fn via_tmp(
        &self,
        dest: &Path,
        bytes: &[u8],
        perms: Option<fs::Permissions>,
        place: impl Fn(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        let beside = dest.parent().map(|d| d.join(tmp_name(true)));
        let in_tmp = fs::create_dir_all(&self.tmp_dir).ok().map(|_| self.tmp_dir.join(tmp_name(false)));
        for tmp in [in_tmp, beside].into_iter().flatten() {
            write_new(&tmp, bytes, perms.clone())?;
            match place(&tmp, dest) {
                Ok(()) => {
                    let _ = fs::remove_file(&tmp);
                    return Ok(());
                }
                Err(e) => {
                    let _ = fs::remove_file(&tmp);
                    if !is_cross_device(&e) {
                        return Err(e);
                    }
                }
            }
        }
        Err(io::Error::other("no temporary file could be placed next to the vault"))
    }

    /// The file's bytes as they are now.
    pub fn read_file(&self, path: &VaultPath) -> io::Result<Vec<u8>> {
        fs::read(self.full(path))
    }

    /// Replaces the file at `path` with `new`, if it still holds `old`.
    /// Keeps its permissions.
    pub fn replace_file(&self, path: &VaultPath, old: &[u8], new: &[u8]) -> Result<(), WriteError> {
        let dest = self.full(path);
        if fs::read(&dest)? != old {
            return Err(WriteError::Changed);
        }
        let perms = fs::metadata(&dest)?.permissions();
        self.via_tmp(&dest, new, Some(perms), |tmp, dest| fs::rename(tmp, dest))?;
        Ok(())
    }

    /// Creates a file at `path` (and missing folders); never overwrites.
    pub fn create_file(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), WriteError> {
        let dest = self.full(path);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        match self.via_tmp(&dest, bytes, None, link_no_clobber) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(WriteError::Exists),
            Err(e) => Err(e.into()),
        }
    }

    /// Moves the file at `from` to `to` (making folders), if it still holds
    /// `old`; never overwrites another file. A change of letter case only
    /// is a plain rename.
    pub fn move_file(&self, from: &VaultPath, to: &VaultPath, old: &[u8]) -> Result<(), WriteError> {
        let src = self.full(from);
        if fs::read(&src)? != old {
            return Err(WriteError::Changed);
        }
        let dest = self.full(to);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        if from.as_str().to_lowercase() == to.as_str().to_lowercase() {
            fs::rename(&src, &dest)?;
            return Ok(());
        }
        move_no_clobber(&src, &dest)
    }

    /// Removes the file at `path`, if it still holds `old`.
    pub fn remove_file(&self, path: &VaultPath, old: &[u8]) -> Result<(), WriteError> {
        let full = self.full(path);
        if fs::read(&full)? != old {
            return Err(WriteError::Changed);
        }
        fs::remove_file(full)?;
        Ok(())
    }

    /// File names already in the vault's `.trash/` folder.
    pub fn trash_names(&self) -> io::Result<Vec<String>> {
        match fs::read_dir(self.root.join(".trash")) {
            Ok(entries) => Ok(entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Moves the file at `path` to `.trash/<name>`, if it still holds
    /// `old`; `Exists` when that name is taken. Returns the vault-relative
    /// path in the trash.
    pub fn move_to_trash(&self, path: &VaultPath, name: &str, old: &[u8]) -> Result<String, WriteError> {
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err(WriteError::Io(io::Error::new(io::ErrorKind::InvalidInput, "bad trash name")));
        }
        let src = self.full(path);
        if fs::read(&src)? != old {
            return Err(WriteError::Changed);
        }
        let trash = self.root.join(".trash");
        fs::create_dir_all(&trash)?;
        move_no_clobber(&src, &trash.join(name))?;
        Ok(format!(".trash/{name}"))
    }
}

fn write_new(path: &Path, bytes: &[u8], perms: Option<fs::Permissions>) -> io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    if let Some(p) = perms {
        fs::set_permissions(path, p)?;
    }
    Ok(())
}

/// Puts `src` at `dest` without replacing an existing `dest`.
fn link_no_clobber(src: &Path, dest: &Path) -> io::Result<()> {
    match fs::hard_link(src, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists || is_cross_device(&e) => Err(e),
        // Filesystems without hard links: check, then rename.
        Err(_) if fs::symlink_metadata(dest).is_ok() => Err(io::ErrorKind::AlreadyExists.into()),
        Err(_) => fs::rename(src, dest),
    }
}

fn move_no_clobber(src: &Path, dest: &Path) -> Result<(), WriteError> {
    match link_no_clobber(src, dest) {
        Ok(()) => {
            if fs::symlink_metadata(src).is_ok() {
                fs::remove_file(src)?;
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(WriteError::Exists),
        Err(e) => Err(e.into()),
    }
}
