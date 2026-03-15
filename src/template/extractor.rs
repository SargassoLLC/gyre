//! Secure tarball extraction for Gyre template bundles.
//!
//! # Security guarantees
//!
//! * **Path traversal protection** — every entry path is validated to stay inside
//!   the target directory. Absolute paths and `..` components are rejected.
//! * **SHA-256 checksum verification** — callers may supply the expected digest;
//!   extraction is aborted on mismatch.
//! * **Atomic extraction** — the tarball is extracted into a temporary directory
//!   first, then renamed into place. A failed extraction leaves no partial state.
//! * **Size limit** — individual files exceeding 50 MB are rejected to match the
//!   registry-enforced maximum.

use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// Maximum size for any single file inside a template bundle (50 MB).
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

/// Extract `tarball_path` into `dest_dir`.
///
/// * Creates `dest_dir` (and any parents) if it does not exist.
/// * Extracts atomically: uses a sibling temp directory, renames on success.
/// * Validates all entry paths against traversal attacks before writing.
/// * `expected_sha256` — if `Some`, the tarball bytes are hashed before
///   extraction and must match; returns `Err` on mismatch.
pub fn extract_tarball(tarball_path: &Path, dest_dir: &Path) -> anyhow::Result<ExtractResult> {
    extract_tarball_with_checksum(tarball_path, dest_dir, None)
}

/// Extract with an optional expected SHA-256 hex digest.
pub fn extract_tarball_with_checksum(
    tarball_path: &Path,
    dest_dir: &Path,
    expected_sha256: Option<&str>,
) -> anyhow::Result<ExtractResult> {
    // --- 1. Read tarball bytes and verify checksum ---
    let bytes = std::fs::read(tarball_path)
        .map_err(|e| anyhow::anyhow!("Cannot read tarball {}: {}", tarball_path.display(), e))?;

    let actual_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    };

    if let Some(expected) = expected_sha256 {
        if !subtle_compare(expected, &actual_sha256) {
            anyhow::bail!(
                "Checksum mismatch for {}:\n  expected: {}\n  actual:   {}",
                tarball_path.display(),
                expected,
                actual_sha256
            );
        }
    }

    // --- 2. Prepare atomic staging directory ---
    let parent = dest_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("dest_dir has no parent: {}", dest_dir.display()))?;

    let tmp_dir_name = format!(
        ".gyre-extract-{}-{}",
        dest_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("tmp"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    let staging = parent.join(&tmp_dir_name);

    // Clean up any leftover staging dir
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    // --- 3. Extract into staging ---
    let result = do_extract(&bytes, &staging);

    match result {
        Err(e) => {
            // Roll back: remove staging dir
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        Ok(stats) => {
            // --- 4. Atomic rename: staging -> dest_dir ---
            if dest_dir.exists() {
                std::fs::remove_dir_all(dest_dir)?;
            }
            std::fs::rename(&staging, dest_dir).map_err(|e| {
                let _ = std::fs::remove_dir_all(&staging);
                anyhow::anyhow!(
                    "Failed to move extracted template to {}: {}",
                    dest_dir.display(),
                    e
                )
            })?;

            Ok(ExtractResult {
                sha256: actual_sha256,
                files_extracted: stats.files_extracted,
                bytes_written: stats.bytes_written,
            })
        }
    }
}

/// Result of a successful extraction.
#[derive(Debug)]
pub struct ExtractResult {
    /// Actual SHA-256 hex digest of the tarball.
    pub sha256: String,
    /// Number of files written.
    pub files_extracted: usize,
    /// Total bytes written to disk.
    pub bytes_written: u64,
}

// ---------------------------------------------------------------------------
// Internal extraction logic
// ---------------------------------------------------------------------------

struct ExtractStats {
    files_extracted: usize,
    bytes_written: u64,
}

fn do_extract(gz_bytes: &[u8], staging: &Path) -> anyhow::Result<ExtractStats> {
    use flate2::read::GzDecoder;

    let cursor = std::io::Cursor::new(gz_bytes);
    let gz = GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(gz);

    let mut stats = ExtractStats {
        files_extracted: 0,
        bytes_written: 0,
    };

    for entry in archive.entries()? {
        let mut entry = entry.map_err(|e| anyhow::anyhow!("Error reading tarball entry: {}", e))?;

        let entry_path = entry
            .path()
            .map_err(|e| anyhow::anyhow!("Invalid path in tarball: {}", e))?
            .into_owned();

        // Validate and resolve the path inside staging
        let safe_path = safe_join(staging, &entry_path)?;

        let entry_type = entry.header().entry_type();

        if entry_type.is_dir() {
            std::fs::create_dir_all(&safe_path)?;
            continue;
        }

        if !entry_type.is_file() {
            // Skip symlinks, hard links, and special files
            tracing::debug!("Skipping non-file entry: {}", entry_path.display());
            continue;
        }

        // Enforce per-file size limit
        let size = entry.header().size().unwrap_or(0);
        if size > MAX_FILE_BYTES {
            anyhow::bail!(
                "File '{}' in tarball is too large ({} bytes, max {} bytes)",
                entry_path.display(),
                size,
                MAX_FILE_BYTES
            );
        }

        // Create parent directories
        if let Some(parent) = safe_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write the file
        let mut out = std::fs::File::create(&safe_path)
            .map_err(|e| anyhow::anyhow!("Cannot create {}: {}", safe_path.display(), e))?;

        let written = std::io::copy(&mut entry, &mut out)
            .map_err(|e| anyhow::anyhow!("Error writing {}: {}", safe_path.display(), e))?;

        stats.files_extracted += 1;
        stats.bytes_written += written;
    }

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Path traversal guard
// ---------------------------------------------------------------------------

/// Resolve `entry_path` relative to `base`, rejecting any path that:
/// * is absolute
/// * contains `..` components
/// * would escape `base` after normalization
fn safe_join(base: &Path, entry_path: &Path) -> anyhow::Result<PathBuf> {
    // Reject absolute paths
    if entry_path.is_absolute() {
        anyhow::bail!(
            "Path traversal rejected: absolute path '{}' in tarball",
            entry_path.display()
        );
    }

    // Check every component
    for component in entry_path.components() {
        match component {
            Component::ParentDir => {
                anyhow::bail!(
                    "Path traversal rejected: '..' component in tarball path '{}'",
                    entry_path.display()
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "Path traversal rejected: root/prefix component in tarball path '{}'",
                    entry_path.display()
                );
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    let resolved = base.join(entry_path);

    // Final canonical check: resolved must start with base
    // (handles edge cases where canonicalization changes things)
    // We do a prefix check on the string representation since the dir
    // may not exist yet (we haven't created it).
    let base_str = base.to_string_lossy();
    let resolved_str = resolved.to_string_lossy();

    if !resolved_str.starts_with(base_str.as_ref()) {
        anyhow::bail!(
            "Path traversal rejected: '{}' escapes extraction root",
            entry_path.display()
        );
    }

    Ok(resolved)
}

// ---------------------------------------------------------------------------
// Constant-time string comparison (avoids timing attacks on checksums)
// ---------------------------------------------------------------------------

fn subtle_compare(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_join_normal() {
        let base = Path::new("/tmp/base");
        let result = safe_join(base, Path::new("subdir/file.txt")).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/base/subdir/file.txt"));
    }

    #[test]
    fn test_safe_join_rejects_dotdot() {
        let base = Path::new("/tmp/base");
        assert!(safe_join(base, Path::new("../evil")).is_err());
    }

    #[test]
    fn test_safe_join_rejects_absolute() {
        let base = Path::new("/tmp/base");
        assert!(safe_join(base, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn test_safe_join_rejects_nested_dotdot() {
        let base = Path::new("/tmp/base");
        assert!(safe_join(base, Path::new("a/../../etc/passwd")).is_err());
    }

    #[test]
    fn test_safe_join_allows_curdir() {
        let base = Path::new("/tmp/base");
        // ./file.txt should be fine
        let result = safe_join(base, Path::new("./file.txt")).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/base/file.txt"));
    }

    #[test]
    fn test_subtle_compare() {
        assert!(subtle_compare("abc123", "abc123"));
        assert!(!subtle_compare("abc123", "abc124"));
        assert!(!subtle_compare("abc", "abcd"));
    }

    #[test]
    fn test_extract_roundtrip() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use sha2::{Digest, Sha256};

        let tmp = tempfile::tempdir().unwrap();

        // Build a minimal tarball in memory
        let buf = Vec::new();
        let gz = GzEncoder::new(buf, Compression::best());
        let mut builder = tar::Builder::new(gz);

        // Add a file
        let content = b"hello world\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "my-template/soul.md", content.as_slice())
            .unwrap();

        // Add a directory
        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        builder
            .append_data(&mut dir_header, "my-template/TELOS/", &[][..])
            .unwrap();

        let gz = builder.into_inner().unwrap();
        let tarball_bytes = gz.finish().unwrap();

        // Compute sha256
        let mut hasher = Sha256::new();
        hasher.update(&tarball_bytes);
        let sha256 = format!("{:x}", hasher.finalize());

        // Write tarball to disk
        let tarball_path = tmp.path().join("test.gyre.tar.gz");
        std::fs::write(&tarball_path, &tarball_bytes).unwrap();

        let dest = tmp.path().join("extracted");
        let result = extract_tarball_with_checksum(&tarball_path, &dest, Some(&sha256)).unwrap();

        assert_eq!(result.files_extracted, 1); // only the file, not the dir
        assert!(dest.join("my-template/soul.md").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("my-template/soul.md")).unwrap(),
            "hello world\n"
        );
    }
}
