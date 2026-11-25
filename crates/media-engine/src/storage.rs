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

fn is_reserved_credentials_path(root: &Path, candidate: &Path) -> bool {
    let Some(name) = candidate.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let is_cred_name = name.eq_ignore_ascii_case("agent_credentials.json")
        || name
            .to_ascii_lowercase()
            .starts_with(".agent_credentials.json.");
    if !is_cred_name {
        return false;
    }
    candidate
        .parent()
        .is_some_and(|parent| parent == root || candidate.starts_with(root))
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
        if is_reserved_credentials_path(&root, &cursor) {
            return Err(MediaError::StorageError(
                "access to internal credentials file is denied".to_string(),
            ));
        }
        Ok((root, cursor))
    }
}

fn strip_redundant_root_prefix(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root_name = root.file_name()?;
    let mut normals = Vec::new();
    for comp in candidate.components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(seg) => normals.push(seg),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if normals.len() >= 2 && normals[0] == root_name {
        let mut stripped = PathBuf::new();
        for seg in &normals[1..] {
            stripped.push(seg);
        }
        Some(stripped)
    } else {
        None
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
            let joined = root.join(&candidate);
            if joined.exists() {
                joined
            } else if let Some(stripped) = strip_redundant_root_prefix(&root, &candidate) {
                root.join(stripped)
            } else {
                joined
            }
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
        if is_reserved_credentials_path(&root, &canonical) {
            return Err(MediaError::StorageError(
                "access to internal credentials file is denied".to_string(),
            ));
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

    fn sync_file(&self, _relative_key: &str, local_path: &Path) -> Result<String, MediaError> {
        let resolved = fs::canonicalize(local_path).unwrap_or_else(|_| local_path.to_path_buf());
        Ok(resolved.to_string_lossy().to_string())
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

        // Reject reads and writes targeting Solaris's internal credentials file
        let cred_file = temp.join("agent_credentials.json");
        fs::write(&cred_file, r#"{"openrouter_api_key":"sk-or-secret"}"#).unwrap();
        assert!(matches!(
            storage.resolve_readable_path("agent_credentials.json"),
            Err(MediaError::StorageError(_))
        ));
        assert!(matches!(
            storage.save_bytes("agent_credentials.json", b"overwrite"),
            Err(MediaError::StorageError(_))
        ));
        // Even with allow_external_reads = true, internal credentials file stays blocked
        let open_storage = LocalFsStorage::new(&temp, true);
        assert!(matches!(
            open_storage.resolve_readable_path(&cred_file.to_string_lossy()),
            Err(MediaError::StorageError(_))
        ));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn sync_file_returns_canonical_path_and_resolves_legacy_root_prefixed_relative_paths() {
        let dir_name = format!("solaris_rel_storage_test_{}", Uuid::new_v4().simple());
        let temp = std::env::temp_dir().join(&dir_name);
        let storage = LocalFsStorage::new(&temp, false);
        let export_file = storage
            .save_bytes("exports/final_abcd1234.mp4", b"fake-mp4-bytes")
            .unwrap();

        let synced = storage
            .sync_file("exports/final_abcd1234.mp4", &export_file)
            .expect("sync_file succeeds");
        assert!(
            Path::new(&synced).is_absolute(),
            "sync_file must return an absolute canonical path"
        );
        let resolved_synced = storage
            .resolve_readable_path(&synced)
            .expect("resolve synced export path");
        assert_eq!(resolved_synced, fs::canonicalize(&export_file).unwrap());

        // Legacy persisted relative paths prefixed with workspace root folder name
        // (e.g. `.solaris-data/exports/foo.mp4`) must resolve cleanly without doubling the root.
        let legacy_prefixed = format!("{dir_name}/exports/final_abcd1234.mp4");
        let resolved_legacy = storage
            .resolve_readable_path(&legacy_prefixed)
            .expect("resolve legacy root-prefixed relative path");
        assert_eq!(resolved_legacy, fs::canonicalize(&export_file).unwrap());

        // Traversal with root prefix must still be rejected.
        let traversal = format!("{dir_name}/../outside.mp4");
        assert!(matches!(
            storage.resolve_readable_path(&traversal),
            Err(MediaError::StorageError(_))
        ));

        let _ = fs::remove_dir_all(&temp);
    }
}
