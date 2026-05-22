//! mkcert wrapper — issues per-project local-TLS certificates.
//!
//! mkcert (https://github.com/FiloSottile/mkcert) is a one-shot tool, not
//! a long-running daemon. We shell out to its binary on demand. PortBay
//! bundles the macOS arm64 build (see `tauri.conf.json` `bundle.externalBin`).
//!
//! Lifecycle:
//!   * First run: `is_ca_installed()` checks for the CA root cert on disk.
//!     If missing, the user is prompted (with `ASSESSMENT_AND_PLAN.md` §5.4
//!     envelope), then `install_ca()` runs `mkcert -install` — single sudo
//!     prompt for the lifetime of the install.
//!   * Per-project: `issue_cert(project_id, hostnames)` runs mkcert in a
//!     per-project directory, producing `cert.pem` + `key.pem`.
//!   * On project removal: `remove_cert(project_id)` deletes the dir.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::caddy::CertPaths;

#[derive(thiserror::Error, Debug)]
pub enum MkcertError {
    #[error("mkcert binary not found at {0}")]
    BinaryMissing(PathBuf),

    #[error("mkcert exited with status {status}: {stderr}")]
    ExitStatus { status: i32, stderr: String },

    #[error("could not locate mkcert's CA directory")]
    CaRootUnknown,

    #[error("CA root certificate missing at {0} — run mkcert -install first")]
    CaNotInstalled(PathBuf),

    #[error("cert files missing for project `{project_id}` at {dir}")]
    CertMissing { project_id: String, dir: PathBuf },

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl MkcertError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, MkcertError>;

/// Wrapper over the mkcert binary and a directory where per-project certs
/// live.
///
/// Cheap to clone — both fields are `PathBuf`. Held inside Tauri state and
/// passed to handlers as `&Mkcert` or cloned for use in async contexts.
#[derive(Debug, Clone)]
pub struct Mkcert {
    binary: PathBuf,
    certs_root: PathBuf,
}

impl Mkcert {
    /// Construct a wrapper. `binary` is the path to the mkcert executable;
    /// `certs_root` is the directory under which per-project subdirectories
    /// will be created (one per project id).
    pub fn new(binary: impl Into<PathBuf>, certs_root: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            certs_root: certs_root.into(),
        }
    }

    /// Default locations: bundled mkcert binary + `data_dir/PortBay/certs/`.
    /// Used by the Tauri app at runtime; tests use the explicit constructor.
    pub fn default_in_data_dir(binary: impl Into<PathBuf>) -> Option<Self> {
        let mut root = dirs::data_dir()?;
        root.push("PortBay");
        root.push("certs");
        Some(Self::new(binary, root))
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn certs_root(&self) -> &Path {
        &self.certs_root
    }

    /// Run `mkcert -CAROOT` and return the path it reports.
    pub fn ca_root(&self) -> Result<PathBuf> {
        let out = self.command().arg("-CAROOT").output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MkcertError::BinaryMissing(self.binary.clone())
            } else {
                MkcertError::io(&self.binary, e)
            }
        })?;
        if !out.status.success() {
            return Err(MkcertError::ExitStatus {
                status: out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() {
            return Err(MkcertError::CaRootUnknown);
        }
        Ok(PathBuf::from(path))
    }

    /// Cheap heuristic for "has the user already run `mkcert -install`?".
    ///
    /// We check whether `<CAROOT>/rootCA.pem` exists. This doesn't *guarantee*
    /// the CA is in the system trust store — only that mkcert has issued
    /// itself a root. Good enough for the first-run gate; the worst case is
    /// we re-prompt for sudo, and `mkcert -install` is idempotent.
    pub fn is_ca_installed(&self) -> bool {
        match self.ca_root() {
            Ok(p) => p.join("rootCA.pem").exists(),
            Err(_) => false,
        }
    }

    /// Run `mkcert -install`. Triggers the macOS keychain sudo prompt; on
    /// Firefox-enabled systems may also surface an NSS prompt.
    pub fn install_ca(&self) -> Result<()> {
        let status = self.command().arg("-install").status().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MkcertError::BinaryMissing(self.binary.clone())
            } else {
                MkcertError::io(&self.binary, e)
            }
        })?;
        if !status.success() {
            return Err(MkcertError::ExitStatus {
                status: status.code().unwrap_or(-1),
                stderr: String::new(),
            });
        }
        Ok(())
    }

    /// Issue a cert for `hostnames`, written into `<certs_root>/<project_id>/`
    /// as `cert.pem` and `key.pem`. Idempotent — calling again with the same
    /// hostnames simply rewrites the cert pair.
    pub fn issue_cert(&self, project_id: &str, hostnames: &[&str]) -> Result<CertPaths> {
        if !self.is_ca_installed() {
            let root = self.ca_root().unwrap_or_else(|_| PathBuf::from("(unknown)"));
            return Err(MkcertError::CaNotInstalled(root.join("rootCA.pem")));
        }

        let dir = self.certs_root.join(project_id);
        std::fs::create_dir_all(&dir).map_err(|e| MkcertError::io(&dir, e))?;

        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");

        let mut cmd = self.command();
        cmd.current_dir(&dir)
            .arg("-cert-file")
            .arg(&cert_path)
            .arg("-key-file")
            .arg(&key_path);
        for h in hostnames {
            cmd.arg(h);
        }

        let out = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MkcertError::BinaryMissing(self.binary.clone())
            } else {
                MkcertError::io(&self.binary, e)
            }
        })?;
        if !out.status.success() {
            return Err(MkcertError::ExitStatus {
                status: out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }

        if !cert_path.exists() || !key_path.exists() {
            return Err(MkcertError::CertMissing {
                project_id: project_id.into(),
                dir,
            });
        }

        Ok(CertPaths {
            certificate: cert_path,
            key: key_path,
        })
    }

    /// Look up existing cert paths for a project. Returns `None` if no
    /// cert has been issued. Pure (no `mkcert` invocation).
    pub fn cert_paths(&self, project_id: &str) -> Option<CertPaths> {
        let dir = self.certs_root.join(project_id);
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        if cert.exists() && key.exists() {
            Some(CertPaths {
                certificate: cert,
                key,
            })
        } else {
            None
        }
    }

    /// Remove a project's cert directory. Idempotent — missing is fine.
    pub fn remove_cert(&self, project_id: &str) -> Result<()> {
        let dir = self.certs_root.join(project_id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(MkcertError::io(&dir, e)),
        }
    }

    fn command(&self) -> Command {
        Command::new(&self.binary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// True iff the current host has `mkcert` on PATH AND the CA has been
    /// installed. Used to gate integration-flavoured tests.
    fn host_has_mkcert_ca() -> bool {
        let m = Mkcert::new("mkcert", "/tmp/portbay-mkcert-unused");
        m.is_ca_installed()
    }

    #[test]
    fn cert_paths_returns_none_when_dir_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Mkcert::new("mkcert", tmp.path());
        assert!(m.cert_paths("nope").is_none());
    }

    #[test]
    fn cert_paths_returns_some_when_files_present() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Mkcert::new("mkcert", tmp.path());
        let project_dir = tmp.path().join("p");
        std::fs::create_dir(&project_dir).unwrap();
        std::fs::write(project_dir.join("cert.pem"), b"fake").unwrap();
        std::fs::write(project_dir.join("key.pem"), b"fake").unwrap();
        let paths = m.cert_paths("p").unwrap();
        assert_eq!(paths.certificate, project_dir.join("cert.pem"));
        assert_eq!(paths.key, project_dir.join("key.pem"));
    }

    #[test]
    fn remove_cert_is_idempotent_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Mkcert::new("mkcert", tmp.path());
        // Should not error even though the dir doesn't exist.
        m.remove_cert("never-existed").unwrap();
    }

    #[test]
    fn remove_cert_deletes_existing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Mkcert::new("mkcert", tmp.path());
        let project_dir = tmp.path().join("p");
        std::fs::create_dir(&project_dir).unwrap();
        std::fs::write(project_dir.join("cert.pem"), b"x").unwrap();
        m.remove_cert("p").unwrap();
        assert!(!project_dir.exists());
    }

    #[test]
    fn ca_root_errors_when_binary_missing() {
        let m = Mkcert::new("/does/not/exist/mkcert", "/tmp");
        match m.ca_root() {
            Err(MkcertError::BinaryMissing(p)) => {
                assert_eq!(p, PathBuf::from("/does/not/exist/mkcert"));
            }
            other => panic!("expected BinaryMissing, got {other:?}"),
        }
    }

    #[test]
    fn is_ca_installed_returns_false_when_binary_missing() {
        let m = Mkcert::new("/does/not/exist/mkcert", "/tmp");
        assert!(!m.is_ca_installed());
    }

    // -- Integration tests (require a real mkcert + installed CA) ------------

    /// Reads the real mkcert -CAROOT and confirms it returns a non-empty
    /// path. Skipped if the host doesn't have mkcert set up.
    #[test]
    fn integration_ca_root_returns_path_when_host_has_mkcert() {
        if !host_has_mkcert_ca() {
            eprintln!("skipping — host has no mkcert CA installed");
            return;
        }
        let m = Mkcert::new("mkcert", "/tmp/portbay-mkcert-unused");
        let path = m.ca_root().unwrap();
        assert!(path.exists(), "CA root should exist on disk");
    }

    /// Issue a cert against the real mkcert on the host. Skipped if no CA.
    #[test]
    fn integration_issue_cert_writes_pem_files() {
        if !host_has_mkcert_ca() {
            eprintln!("skipping — host has no mkcert CA installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let m = Mkcert::new("mkcert", tmp.path());
        let paths = m
            .issue_cert("portbay-test", &["portbay-test.test"])
            .unwrap();
        assert!(paths.certificate.exists());
        assert!(paths.key.exists());
        let cert_contents = std::fs::read_to_string(&paths.certificate).unwrap();
        assert!(
            cert_contents.contains("BEGIN CERTIFICATE"),
            "cert.pem missing PEM header"
        );
    }
}
