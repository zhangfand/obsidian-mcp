use std::fmt;

/// A file's place in the vault: relative to the vault root, `/`-separated,
/// with no `.`, `..` or hidden (dot) component. Holding one means the path
/// cannot name anything outside the vault's visible files.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VaultPath(String);

/// Why a path was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Absolute,
    ParentDir,
    /// A component starts with `.` (`.obsidian`, `.git`, `.trash`, ...).
    Hidden,
    /// A symlink on the way points outside the vault.
    LeavesVault,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathError::Empty => "the path is empty",
            PathError::Absolute => "absolute paths are not allowed; use a path relative to the vault",
            PathError::ParentDir => "`..` is not allowed in a path",
            PathError::Hidden => "hidden folders and files (starting with `.`) are not part of the vault",
            PathError::LeavesVault => "the path goes through a symlink that leaves the vault",
        })
    }
}

impl std::error::Error for PathError {}

impl VaultPath {
    /// Parses a vault-relative path. `\` counts as a separator, `.` and empty
    /// components are dropped.
    pub fn parse(raw: &str) -> Result<VaultPath, PathError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(PathError::Empty);
        }
        let unified = raw.replace('\\', "/");
        let bytes = unified.as_bytes();
        let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        if unified.starts_with('/') || unified.starts_with('~') || has_drive {
            return Err(PathError::Absolute);
        }
        let mut parts = Vec::new();
        for part in unified.split('/') {
            match part {
                "" | "." => continue,
                ".." => return Err(PathError::ParentDir),
                p if p.starts_with('.') => return Err(PathError::Hidden),
                p => parts.push(p),
            }
        }
        if parts.is_empty() {
            return Err(PathError::Empty);
        }
        Ok(VaultPath(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// The folder holding the file, `""` at the vault root.
    pub fn folder(&self) -> &str {
        self.0.rfind('/').map_or("", |i| &self.0[..i])
    }

    /// True for Markdown files, the only files that are notes.
    pub fn is_note(&self) -> bool {
        self.0.to_lowercase().ends_with(".md")
    }

    /// The note name: the file name without `.md`.
    pub fn stem(&self) -> &str {
        let name = self.file_name();
        if name.to_lowercase().ends_with(".md") {
            &name[..name.len() - 3]
        } else {
            name
        }
    }

    pub fn is_inside(&self, folder: &VaultPath) -> bool {
        self.0.to_lowercase().starts_with(&format!("{}/", folder.0.to_lowercase()))
    }
}

impl fmt::Display for VaultPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_escapes() {
        assert_eq!(VaultPath::parse("../x.md"), Err(PathError::ParentDir));
        assert_eq!(VaultPath::parse("a/../../x.md"), Err(PathError::ParentDir));
        assert_eq!(VaultPath::parse("/etc/passwd"), Err(PathError::Absolute));
        assert_eq!(VaultPath::parse("C:\\x.md"), Err(PathError::Absolute));
        assert_eq!(VaultPath::parse(".obsidian/app.json"), Err(PathError::Hidden));
        assert_eq!(VaultPath::parse("a/.git/config"), Err(PathError::Hidden));
        assert_eq!(VaultPath::parse(".trash/x.md"), Err(PathError::Hidden));
        assert_eq!(VaultPath::parse("  "), Err(PathError::Empty));
    }

    #[test]
    fn normalizes() {
        let p = VaultPath::parse("./a//b\\c.md").unwrap();
        assert_eq!(p.as_str(), "a/b/c.md");
        assert_eq!(p.stem(), "c");
        assert_eq!(p.folder(), "a/b");
        assert!(p.is_note());
    }
}
