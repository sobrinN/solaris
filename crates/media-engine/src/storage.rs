use std::fs;
use std::path::{Component, Path, PathBuf};

use uuid::Uuid;

use crate::error::MediaError;

/// Abstraction over asset persistence so Desktop (`LocalFsStorage`) and future
/// Web SaaS (`S3Storage` / `R2Storage`) share the exact same server & engine logic.
pub trait AssetStorage: Send + Sync {
    fn backend_kind(&self) -> &'static str {
        "local_fs"
    }
    fn resolve_readable_path(&self, raw_path: &str) -> Result<PathBuf, MediaError>;
    fn save_bytes(&self, relative_key: &str, bytes: &[u8]) -> Result<PathBuf, MediaError>;
    fn sync_file(&self, _relative_key: &str, local_path: &Path) -> Result<String, MediaError> {
        Ok(local_path.to_string_lossy().to_string())
    }
}

#[derive(Debug, Clone)]
pub struct LocalFsStorage {
    workspace_root: PathBuf,
    root_canonical: bool,
    allow_external_reads: bool,
}

impl LocalFsStorage {
    pub fn new(workspace_root: impl Into<PathBuf>, allow_external_reads: bool) -> Self {
        let root = workspace_root.into();
        let _ = fs::create_dir_all(&root);
        match fs::canonicalize(&root) {
            Ok(canonical) => Self {
                workspace_root: canonical,
                root_canonical: true,
                allow_external_reads,
            },
            Err(_) => Self {
                workspace_root: root,
                root_canonical: false,
                allow_external_reads,
            },
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn canonical_root(&self) -> Result<PathBuf, MediaError> {
        if self.root_canonical {
            Ok(self.workspace_root.clone())
        } else {
            fs::canonicalize(&self.workspace_root).map_err(|e| {
                MediaError::StorageError(format!(
                    "workspace root '{}' is not accessible: {e}",
                    self.workspace_root.display()
                ))
            })
        }
    }

    /// Validates that `relative_key` has no traversal or root/prefix components and that
    /// no existing ancestor symlink under `workspace_root` escapes outside `workspace_root`
    /// BEFORE creating any directories on disk.
    pub fn validate_confined_write_target(
        &self,
        relative_key: &str,
    ) -> Result<(PathBuf, PathBuf), MediaError> {
        let root = self.canonical_root()?;
        let rel = Path::new(relative_key);
        if rel.is_absolute()
            || rel.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(MediaError::StorageError(
                "relative storage key must not be absolute, prefixed, or contain '..'".to_string(),
            ));
        }
        let mut cursor = root.clone();
        for comp in rel.components() {
            if let Component::Normal(seg) = comp {
                cursor.push(seg);
                if cursor.exists() || cursor.symlink_metadata().is_ok() {
                    let canon = fs::canonicalize(&cursor).map_err(|e| {
                        MediaError::StorageError(format!(
                            "cannot canonicalize existing path '{}': {e}",
                            cursor.display()
                        ))
                    })?;
                    if !canon.starts_with(&root) {
                        return Err(MediaError::StorageError(format!(
                            "symlink '{}' escapes workspace root '{}'",
                            cursor.display(),
                            root.display()
                        )));
                    }
                }
            }
        }
        if cursor == root {
            return Err(MediaError::StorageError(
                "relative storage key must name a file".to_string(),
            ));
        }
        Ok((root, cursor))
    }
}

impl AssetStorage for LocalFsStorage {
    fn resolve_readable_path(&self, raw_path: &str) -> Result<PathBuf, MediaError> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err(MediaError::StorageError(
                "storage path must not be empty".to_string(),
            ));
        }
        if trimmed.contains("://") {
            return Err(MediaError::StorageError(format!(
                "URL scheme in '{trimmed}' is not allowed for local storage"
            )));
        }
        let root = self.canonical_root()?;
        let candidate = PathBuf::from(trimmed);
        let target = if candidate.is_absolute() {
            candidate
        } else {
            root.join(candidate)
        };
        let canonical = fs::canonicalize(&target).map_err(|e| {
            MediaError::StorageError(format!("cannot access '{}': {e}", target.display()))
        })?;

        if !self.allow_external_reads && !canonical.starts_with(&root) {
            return Err(MediaError::StorageError(format!(
                "path '{}' escapes workspace root '{}'",
                canonical.display(),
                root.display()
            )));
        }
        if !canonical.is_file() {
            return Err(MediaError::StorageError(format!(
                "path '{}' is not a regular file",
                canonical.display()
            )));
        }
        Ok(canonical)
    }

    fn save_bytes(&self, relative_key: &str, bytes: &[u8]) -> Result<PathBuf, MediaError> {
        let (root, dest) = self.validate_confined_write_target(relative_key)?;
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
            let canon_parent = fs::canonicalize(parent).map_err(|e| {
                MediaError::StorageError(format!("cannot canonicalize '{}': {e}", parent.display()))
            })?;
            if !canon_parent.starts_with(&root) {
                return Err(MediaError::StorageError(format!(
                    "parent '{}' escapes workspace root '{}'",
                    canon_parent.display(),
                    root.display()
                )));
            }
        }
        let file_name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("blob");
        let tmp = dest.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
        if let Err(err) = fs::write(&tmp, bytes) {
            let _ = fs::remove_file(&tmp);
            return Err(MediaError::Io(err));
        }
        if let Err(err) = fs::rename(&tmp, &dest) {
            let _ = fs::remove_file(&tmp);
            return Err(MediaError::Io(err));
        }
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confines_relative_writes_and_blocks_traversal() {
        let temp =
            std::env::temp_dir().join(format!("solaris_storage_test_{}", std::process::id()));
        let storage = LocalFsStorage::new(&temp, false);

        let err = storage.save_bytes("../escape.txt", b"bad").unwrap_err();
        assert!(matches!(err, MediaError::StorageError(_)));

        let saved = storage.save_bytes("uploads/clip.txt", b"ok").unwrap();
        assert!(saved.exists());

        let resolved = storage
            .resolve_readable_path("uploads/clip.txt")
            .expect("resolve inside workspace");
        assert_eq!(resolved, fs::canonicalize(&saved).unwrap());

        // Reject empty paths and directories
        assert!(matches!(
            storage.resolve_readable_path(""),
            Err(MediaError::StorageError(_))
        ));
        assert!(matches!(
            storage.resolve_readable_path("   "),
            Err(MediaError::StorageError(_))
        ));
        assert!(matches!(
            storage.resolve_readable_path("uploads"),
            Err(MediaError::StorageError(_))
        ));

        let _ = fs::remove_dir_all(&temp);
    }
}
