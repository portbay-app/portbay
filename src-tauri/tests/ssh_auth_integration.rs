//! End-to-end auth-pipeline tests against a real in-process SSH server.
//!
//! No mocks: a russh server advertises every auth method and a per-test
//! `Policy` decides which it actually accepts. The client side is our
//! **production** path — `connect_session` → the ordered key→agent→password→
//! keyboard-interactive pipeline — so this proves the pipeline picks the right
//! method and *falls through* when the preferred one is refused.
//!
//! The SSH-agent leg can't run in CI (no live `SSH_AUTH_SOCK` with a key the
//! server trusts), so it's covered by an `#[ignore]` live test below and a
//! documented manual check. Unix-only, matching the SFTP harness.
#![cfg(unix)]

use std::borrow::Cow;
use std::sync::Arc;

use async_trait::async_trait;
use russh::server::{Auth, Config as ServerConfig, Handler as SshHandler, Response, Server as _};
use russh_keys::key::{KeyPair, PublicKey};

use portbay_lib::registry::{SshAuthKind, SshConnection, SshConnectionId};
use portbay_lib::ssh::connect_session;

const PASSWORD: &str = "hunter2";

// ---------------------------------------------------------------------------
// SSH server: advertise all methods, accept only what `Policy` permits.
// ---------------------------------------------------------------------------

/// Which methods this server instance will accept. Lets each assertion force a
/// single accepted method and prove the client falls through to it.
#[derive(Clone, Copy)]
struct Policy {
    allow_key: bool,
    allow_password: bool,
    allow_keyboard_interactive: bool,
}

#[derive(Clone)]
struct AuthServer {
    policy: Policy,
    /// Fingerprint of the one public key this server trusts for pubkey auth.
    trusted_key_fp: String,
}

impl russh::server::Server for AuthServer {
    type Handler = AuthConn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> AuthConn {
        AuthConn {
            policy: self.policy,
            trusted_key_fp: self.trusted_key_fp.clone(),
        }
    }
}

struct AuthConn {
    policy: Policy,
    trusted_key_fp: String,
}

fn reject() -> Auth {
    Auth::Reject {
        proceed_with_methods: None,
    }
}

#[async_trait]
impl SshHandler for AuthConn {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        // `"*"` is a live-test sentinel: trust whatever key the agent signs with
        // (used by `agent_auth_via_real_socket_live`, where we can't know the
        // agent's fingerprint up front). Normal tests pin an exact fingerprint.
        let trusted = self.trusted_key_fp == "*" || public_key.fingerprint() == self.trusted_key_fp;
        if self.policy.allow_key && trusted {
            Ok(Auth::Accept)
        } else {
            Ok(reject())
        }
    }

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        if self.policy.allow_password && password == PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(reject())
        }
    }

    async fn auth_keyboard_interactive(
        &mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'async_trait>>,
    ) -> Result<Auth, Self::Error> {
        if !self.policy.allow_keyboard_interactive {
            return Ok(reject());
        }
        match response {
            // First exchange: ask for the password (one non-echoed prompt).
            None => Ok(Auth::Partial {
                name: Cow::Borrowed("Password authentication"),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
            }),
            // Second exchange: verify the echoed password.
            Some(mut answers) => {
                let answer = answers
                    .next()
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                    .unwrap_or_default();
                if answer == PASSWORD {
                    Ok(Auth::Accept)
                } else {
                    Ok(reject())
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Boot the in-process SSH server on an ephemeral port; return the port.
async fn start_server(policy: Policy, trusted_key_fp: String) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(ServerConfig {
        keys: vec![KeyPair::generate_ed25519().unwrap()],
        ..Default::default()
    });
    let mut server = AuthServer {
        policy,
        trusted_key_fp,
    };
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    port
}

fn connection(port: u16, auth_kind: SshAuthKind, key_path: Option<String>) -> SshConnection {
    SshConnection {
        id: SshConnectionId::new("it-auth"),
        name: "auth test".into(),
        ssh_host: "127.0.0.1".into(),
        ssh_port: port,
        ssh_user: "tester".into(),
        auth_kind,
        key_path,
        proxy_jump: None,
        identity_id: None,
        proxy: None,
        metadata: Default::default(),
    }
}

/// Generate an ed25519 client key, write it as a PKCS#8 PEM the production
/// `load_secret_key` path reads, and return `(path, fingerprint)`.
fn write_client_key(dir: &std::path::Path) -> (String, String) {
    let keypair = KeyPair::generate_ed25519().unwrap();
    let fingerprint = keypair.clone_public_key().unwrap().fingerprint();
    let mut pem = Vec::new();
    russh_keys::encode_pkcs8_pem(&keypair, &mut pem).unwrap();
    let path = dir.join("id_ed25519");
    std::fs::write(&path, &pem).unwrap();
    (path.to_string_lossy().into_owned(), fingerprint)
}

/// Isolate host-key TOFU writes (connect persists into `~/.ssh/known_hosts`).
fn isolate_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    home
}

// ---------------------------------------------------------------------------
// Tests — one fn so process-global env (HOME, SSH_AUTH_SOCK) has no intra-file race.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auth_pipeline_picks_each_method_and_falls_through() {
    let home = isolate_home();
    // Deterministically skip the agent leg: with no socket the pipeline's agent
    // step returns immediately, so attempt budgets are stable regardless of the
    // developer's running ssh-agent.
    std::env::remove_var("SSH_AUTH_SOCK");
    let key_dir = home.path().join(".ssh");
    let (key_path, key_fp) = write_client_key(&key_dir);

    // (1) KEY — server accepts only the trusted pubkey; Key-preferred connection
    //     authenticates on the first method.
    {
        let port = start_server(
            Policy {
                allow_key: true,
                allow_password: false,
                allow_keyboard_interactive: false,
            },
            key_fp.clone(),
        )
        .await;
        let conn = connection(port, SshAuthKind::Key, Some(key_path.clone()));
        assert!(
            connect_session(&conn, None, None, None, None).await.is_ok(),
            "key auth should succeed against a key-only server"
        );
    }

    // (2) PASSWORD — server accepts only the password; Password-preferred
    //     connection authenticates with the supplied password.
    {
        let port = start_server(
            Policy {
                allow_key: false,
                allow_password: true,
                allow_keyboard_interactive: false,
            },
            key_fp.clone(),
        )
        .await;
        let conn = connection(port, SshAuthKind::Password, None);
        assert!(
            connect_session(&conn, Some(PASSWORD), None, None, None)
                .await
                .is_ok(),
            "password auth should succeed against a password-only server"
        );
    }

    // (3) KEYBOARD-INTERACTIVE — server accepts only KI; the pipeline tries
    //     password first (refused), then falls through to KI echoing the password.
    {
        let port = start_server(
            Policy {
                allow_key: false,
                allow_password: false,
                allow_keyboard_interactive: true,
            },
            key_fp.clone(),
        )
        .await;
        let conn = connection(port, SshAuthKind::Password, None);
        assert!(
            connect_session(&conn, Some(PASSWORD), None, None, None)
                .await
                .is_ok(),
            "keyboard-interactive should succeed when password proper is refused"
        );
    }

    // (4) FALL-THROUGH — Key-preferred connection whose key the server refuses,
    //     but a stored password is available: pipeline tries key, then password.
    {
        let port = start_server(
            Policy {
                allow_key: false,
                allow_password: true,
                allow_keyboard_interactive: false,
            },
            key_fp.clone(),
        )
        .await;
        let conn = connection(port, SshAuthKind::Key, Some(key_path.clone()));
        assert!(
            connect_session(&conn, Some(PASSWORD), None, None, None)
                .await
                .is_ok(),
            "a refused key should fall through to the stored password"
        );
    }

    // (5) ALL REFUSED — wrong password, key not trusted, no agent: a single
    //     aggregated error, not a hang or a panic.
    {
        let port = start_server(
            Policy {
                allow_key: true,
                allow_password: true,
                allow_keyboard_interactive: true,
            },
            "SHA256:not-the-clients-key".into(),
        )
        .await;
        let conn = connection(port, SshAuthKind::Key, Some(key_path.clone()));
        assert!(
            connect_session(&conn, Some("wrong-password"), None, None, None)
                .await
                .is_err(),
            "no acceptable credential must yield an error, not a session"
        );
    }
}

/// Live SSH-agent check — **ignored** because it needs a real agent and a host
/// that trusts an agent-held key (impossible to fixture in CI). Run manually:
///
/// ```sh
/// eval "$(ssh-agent)" && ssh-add ~/.ssh/id_ed25519
/// PORTBAY_SSH_AGENT_HOST=1.2.3.4 PORTBAY_SSH_AGENT_USER=ubuntu \
///   cargo test --test ssh_auth_integration agent_only -- --ignored --nocapture
/// ```
///
/// Asserts the pipeline authenticates an agent-only connection (no key_path,
/// no password) purely via `SSH_AUTH_SOCK`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a live ssh-agent + a host that trusts an agent key"]
async fn agent_only_auth_succeeds_live() {
    let host = std::env::var("PORTBAY_SSH_AGENT_HOST")
        .expect("set PORTBAY_SSH_AGENT_HOST to the test host");
    let user = std::env::var("PORTBAY_SSH_AGENT_USER")
        .expect("set PORTBAY_SSH_AGENT_USER to the login user");
    let port: u16 = std::env::var("PORTBAY_SSH_AGENT_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(22);

    let conn = SshConnection {
        id: SshConnectionId::new("it-agent"),
        name: "agent live".into(),
        ssh_host: host,
        ssh_port: port,
        ssh_user: user,
        auth_kind: SshAuthKind::Agent,
        key_path: None,
        proxy_jump: None,
        identity_id: None,
        proxy: None,
        metadata: Default::default(),
    };
    connect_session(&conn, None, None, None, None)
        .await
        .expect("agent-only auth should succeed against a host trusting the agent key");
}

/// Live SSH-agent check against our **own in-process server** over the real
/// `SSH_AUTH_SOCK` — proves the agent leg end to end (`connect_env` →
/// `request_identities` → `authenticate_future` signing) without needing an
/// external host or touching `~/.ssh/authorized_keys`. Ignored because it needs
/// a loaded agent. The runner loads an ephemeral key first:
///
/// ```sh
/// ssh-keygen -t ed25519 -N '' -f /tmp/pb-agent-test
/// ssh-add -t 120 /tmp/pb-agent-test
/// cargo test --test ssh_auth_integration agent_auth_via_real_socket_live -- --ignored --nocapture
/// ssh-add -d /tmp/pb-agent-test            # cleanup
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a loaded ssh-agent (SSH_AUTH_SOCK with >=1 identity)"]
async fn agent_auth_via_real_socket_live() {
    let _home = isolate_home();
    assert!(
        std::env::var("SSH_AUTH_SOCK").is_ok(),
        "SSH_AUTH_SOCK must be set (start ssh-agent and ssh-add a key)"
    );
    // Server trusts any agent-signed key (`"*"`); the connection authenticates
    // purely through the agent — no key file, no password.
    let port = start_server(
        Policy {
            allow_key: true,
            allow_password: false,
            allow_keyboard_interactive: false,
        },
        "*".into(),
    )
    .await;
    let conn = connection(port, SshAuthKind::Agent, None);
    connect_session(&conn, None, None, None, None)
        .await
        .expect("agent auth over the real SSH_AUTH_SOCK should succeed");
}
