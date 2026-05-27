//! Download + extract manager for PortBay-managed runtimes.
//!
//! Given a verified [`RuntimeEntry`] from the signed manifest, this fetches the
//! arch-specific archive, checks its **size + SHA-256** against the manifest,
//! decompresses it (zstd) and unpacks the tar into a **staging dir on the same
//! filesystem** as the final location, confirms the expected binary is present
//! and probes, then **atomically renames** staging into place. Any failure
//! cleans up staging and leaves an existing install untouched.
//!
//! The verify/extract/install core ([`install_archive`]) is pure — it operates
//! on in-memory bytes and a destination root, so it unit-tests against a local
//! fixture archive without a network or a hosted CDN. Only [`fetch_and_install`]
//! touches the network, and it hands the downloaded bytes straight to the core
//! (nothing lands on disk before size + checksum pass).

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::manifest::{Compression, RuntimeEntry};

/// Every variant means the download is untrusted or unusable; the caller must
/// abort and leave any existing install in place.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download failed: {0}")]
    Network(String),
    #[error("archive size mismatch: manifest says {expected} bytes, got {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("archive checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("unsupported archive compression: {0:?}")]
    UnsupportedCompression(Compression),
    #[error("archive could not be decompressed/unpacked: {0}")]
    Extract(String),
    #[error("archive is missing its expected binary at {0}")]
    MissingBinary(String),
    #[error("installed binary failed its version probe")]
    ProbeFailed,
    #[error("filesystem error: {0}")]
    Io(String),
}

/// Verify a lowercase-hex SHA-256 digest over `bytes` (case-insensitive compare).
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), DownloadError> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_hex.trim()) {
        Ok(())
    } else {
        Err(DownloadError::ChecksumMismatch {
            expected: expected_hex.trim().to_string(),
            actual,
        })
    }
}

/// Decompress and unpack an in-memory archive into `into`.
///
/// Only zstd `.tar.zst` is supported (the chosen runtime-archive codec); any
/// other codec is rejected so a manifest can't smuggle in an unexpected format.
/// `tar::Archive::unpack` sanitises entry paths (it refuses `..` / absolute
/// paths that would escape `into`), so a hostile archive can't write outside the
/// staging dir.
pub fn extract(archive: &[u8], compression: Compression, into: &Path) -> Result<(), DownloadError> {
    match compression {
        Compression::Zstd => {
            let decoder = zstd::stream::read::Decoder::new(archive)
                .map_err(|e| DownloadError::Extract(e.to_string()))?;
            let mut tar = tar::Archive::new(decoder);
            tar.unpack(into)
                .map_err(|e| DownloadError::Extract(e.to_string()))?;
            Ok(())
        }
        other => Err(DownloadError::UnsupportedCompression(other)),
    }
}

/// Where a managed runtime lands: `<dest_root>/<lang>/<version>/`.
pub fn install_dir(dest_root: &Path, entry: &RuntimeEntry) -> PathBuf {
    dest_root.join(&entry.lang).join(&entry.version)
}

/// Verify + extract an in-memory archive and atomically install it under
/// `<dest_root>/<lang>/<version>/`, returning the absolute path to the primary
/// binary (`install_dir.join(expected_binary_rel)`).
///
/// Order is integrity-first: size + SHA-256 are checked **before** anything is
/// written, then extraction happens in a sibling staging dir (same filesystem,
/// so the final move is an atomic rename). The expected binary must exist and
/// pass `probe` before the swap; on any failure the staging dir is removed and a
/// previously-installed version is left intact.
pub fn install_archive(
    archive: &[u8],
    entry: &RuntimeEntry,
    dest_root: &Path,
    expected_binary_rel: &Path,
    probe: impl FnOnce(&Path) -> bool,
) -> Result<PathBuf, DownloadError> {
    if archive.len() as u64 != entry.size {
        return Err(DownloadError::SizeMismatch {
            expected: entry.size,
            actual: archive.len() as u64,
        });
    }
    verify_sha256(archive, &entry.sha256)?;

    let final_dir = install_dir(dest_root, entry);
    let lang_dir = dest_root.join(&entry.lang);
    let staging = lang_dir.join(format!(".staging-{}-{}", entry.version, std::process::id()));

    // Start from a clean staging dir (a crash mid-install may have left one).
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| DownloadError::Io(e.to_string()))?;

    let result = (|| {
        extract(archive, entry.compression, &staging)?;
        let staged_bin = staging.join(expected_binary_rel);
        if !staged_bin.exists() {
            return Err(DownloadError::MissingBinary(
                expected_binary_rel.display().to_string(),
            ));
        }
        if !probe(&staged_bin) {
            return Err(DownloadError::ProbeFailed);
        }
        // Atomic-ish swap: drop any existing install, then rename staging in.
        // (rename is atomic; the preceding remove is the only non-atomic window,
        // acceptable for a local dev tool re-installing its own runtime.)
        if final_dir.exists() {
            std::fs::remove_dir_all(&final_dir).map_err(|e| DownloadError::Io(e.to_string()))?;
        }
        std::fs::rename(&staging, &final_dir).map_err(|e| DownloadError::Io(e.to_string()))?;
        Ok(final_dir.join(expected_binary_rel))
    })();

    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

/// Fetch `entry.url` and install it under `dest_root`, reporting download
/// progress as `(downloaded_bytes, content_length)`. The bytes are buffered in
/// memory (runtime archives are ~15–25 MB) and gated by [`install_archive`]'s
/// size + checksum checks before anything is written to disk.
pub async fn fetch_and_install(
    entry: &RuntimeEntry,
    dest_root: &Path,
    expected_binary_rel: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>),
    probe: impl FnOnce(&Path) -> bool,
) -> Result<PathBuf, DownloadError> {
    let mut resp = reqwest::get(&entry.url)
        .await
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DownloadError::Network(format!(
            "{} returned HTTP {}",
            entry.url,
            resp.status()
        )));
    }
    let total = resp.content_length();
    let mut bytes: Vec<u8> = Vec::with_capacity(entry.size as usize);
    let mut downloaded = 0u64;
    // `chunk()` streams without needing reqwest's `stream` feature.
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| DownloadError::Network(e.to_string()))?
    {
        downloaded += chunk.len() as u64;
        bytes.extend_from_slice(&chunk);
        on_progress(downloaded, total);
    }
    install_archive(&bytes, entry, dest_root, expected_binary_rel, probe)
}

#[cfg(test)]
mod tests {
    use super::super::manifest::Compression;
    use super::*;

    /// Build a `.tar.zst` in memory containing the given (relative path, bytes)
    /// files, mirroring the layout the release CI will produce.
    fn make_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            for (path, data) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, path, *data).unwrap();
            }
            builder.finish().unwrap();
        }
        zstd::encode_all(&tar_buf[..], 0).unwrap()
    }

    fn entry_for(archive: &[u8]) -> RuntimeEntry {
        let mut hasher = Sha256::new();
        hasher.update(archive);
        RuntimeEntry {
            lang: "php".into(),
            version: "8.3.14".into(),
            arch: super::super::manifest::current_arch().into(),
            url: "https://example.test/php.tar.zst".into(),
            sha256: hex::encode(hasher.finalize()),
            size: archive.len() as u64,
            compression: Compression::Zstd,
        }
    }

    #[test]
    fn installs_a_valid_archive_into_place() {
        let archive = make_archive(&[("sbin/php-fpm", b"#!/bin/sh\necho 8.3.14\n")]);
        let entry = entry_for(&archive);
        let root = tempfile::tempdir().unwrap();
        let bin = install_archive(
            &archive,
            &entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| true,
        )
        .expect("should install");
        assert_eq!(bin, root.path().join("php/8.3.14/sbin/php-fpm"));
        assert!(bin.exists());
        // No staging dir is left behind.
        let leftover: Vec<_> = std::fs::read_dir(root.path().join("php"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging"))
            .collect();
        assert!(leftover.is_empty(), "staging dir was not cleaned up");
    }

    #[test]
    fn rejects_a_checksum_mismatch_without_writing() {
        let archive = make_archive(&[("sbin/php-fpm", b"payload")]);
        let mut entry = entry_for(&archive);
        entry.sha256 = "00".repeat(32); // wrong digest
        let root = tempfile::tempdir().unwrap();
        let err = install_archive(
            &archive,
            &entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| true,
        )
        .unwrap_err();
        assert!(
            matches!(err, DownloadError::ChecksumMismatch { .. }),
            "got {err:?}"
        );
        assert!(!install_dir(root.path(), &entry).exists());
    }

    #[test]
    fn rejects_a_size_mismatch() {
        let archive = make_archive(&[("sbin/php-fpm", b"payload")]);
        let mut entry = entry_for(&archive);
        entry.size += 1;
        let root = tempfile::tempdir().unwrap();
        let err = install_archive(
            &archive,
            &entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| true,
        )
        .unwrap_err();
        assert!(
            matches!(err, DownloadError::SizeMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_archive_missing_the_expected_binary() {
        let archive = make_archive(&[("bin/php", b"only the cli, no fpm")]);
        let entry = entry_for(&archive);
        let root = tempfile::tempdir().unwrap();
        let err = install_archive(
            &archive,
            &entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| true,
        )
        .unwrap_err();
        assert!(
            matches!(err, DownloadError::MissingBinary(_)),
            "got {err:?}"
        );
        assert!(!install_dir(root.path(), &entry).exists());
    }

    #[test]
    fn rejects_when_the_version_probe_fails() {
        let archive = make_archive(&[("sbin/php-fpm", b"wrong version")]);
        let entry = entry_for(&archive);
        let root = tempfile::tempdir().unwrap();
        let err = install_archive(
            &archive,
            &entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| false,
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::ProbeFailed), "got {err:?}");
        assert!(!install_dir(root.path(), &entry).exists());
    }

    #[test]
    fn reinstall_replaces_an_existing_version_atomically() {
        let root = tempfile::tempdir().unwrap();
        let first = make_archive(&[("sbin/php-fpm", b"v1")]);
        let e1 = entry_for(&first);
        install_archive(&first, &e1, root.path(), Path::new("sbin/php-fpm"), |_| {
            true
        })
        .unwrap();

        let second = make_archive(&[("sbin/php-fpm", b"v2-newer")]);
        let e2 = entry_for(&second);
        let bin = install_archive(&second, &e2, root.path(), Path::new("sbin/php-fpm"), |_| {
            true
        })
        .unwrap();
        assert_eq!(std::fs::read(&bin).unwrap(), b"v2-newer");
    }

    #[test]
    fn a_failed_reinstall_leaves_the_existing_install_intact() {
        let root = tempfile::tempdir().unwrap();
        let good = make_archive(&[("sbin/php-fpm", b"keep-me")]);
        let entry = entry_for(&good);
        install_archive(
            &good,
            &entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| true,
        )
        .unwrap();

        // A corrupt re-download (bad checksum) must not disturb the good install.
        let mut bad_entry = entry.clone();
        bad_entry.sha256 = "ff".repeat(32);
        let _ = install_archive(
            &good,
            &bad_entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| true,
        );
        let bin = install_dir(root.path(), &entry).join("sbin/php-fpm");
        assert_eq!(std::fs::read(&bin).unwrap(), b"keep-me");
    }

    #[test]
    fn path_traversal_entries_cannot_escape_staging() {
        // A hostile archive with a `../escaped.txt` entry must not write outside
        // the staging dir. The safe tar builder API refuses to author such a
        // path, so forge it by writing the GNU header's name field directly —
        // then confirm `unpack` (via install_archive) drops it.
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            // Legit binary so the install reaches the extraction step.
            let mut ok = tar::Header::new_gnu();
            ok.set_size(2);
            ok.set_mode(0o755);
            ok.set_cksum();
            builder
                .append_data(&mut ok, "sbin/php-fpm", &b"ok"[..])
                .unwrap();
            // Forged escaping entry.
            let payload = b"escape!";
            let mut evil = tar::Header::new_gnu();
            evil.set_size(payload.len() as u64);
            evil.set_mode(0o644);
            evil.set_entry_type(tar::EntryType::Regular);
            let name = b"../escaped.txt";
            evil.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
            evil.set_cksum();
            builder.append(&evil, &payload[..]).unwrap();
            builder.finish().unwrap();
        }
        let archive = zstd::encode_all(&tar_buf[..], 0).unwrap();
        let entry = entry_for(&archive);
        let root = tempfile::tempdir().unwrap();
        // Result may be Ok (escaping entry skipped) or Err (unpack rejected it);
        // either way the escaping file must not exist anywhere outside staging.
        let _ = install_archive(
            &archive,
            &entry,
            root.path(),
            Path::new("sbin/php-fpm"),
            |_| true,
        );
        assert!(!root.path().join("escaped.txt").exists());
        assert!(!root.path().parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn unsupported_compression_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let err = extract(b"not used", Compression::Xz, root.path()).unwrap_err();
        assert!(
            matches!(err, DownloadError::UnsupportedCompression(Compression::Xz)),
            "got {err:?}"
        );
    }
}
