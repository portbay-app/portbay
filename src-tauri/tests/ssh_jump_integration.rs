//! End-to-end test for the `ProxyJump` chain on the in-process russh path.
//!
//! No mocks: two real in-process russh servers. A **jump** server accepts a
//! public key and bridges any `direct-tcpip` channel to the address the client
//! asked for (dialling the second server over real TCP); an **exec** server is
//! the destination, accepting a password and answering `exec`. The client side
//! is our production `connect_session` → `run_command`, so this proves the full
//! chain: hop 0 authenticates by key, the destination is reached *through* the
//! jump via direct-tcpip and authenticates with the full pipeline (key refused
//! → password), and exec output round-trips back through both hops.
//!
//! Unix-only, matching the other SSH harnesses (we clear `SSH_AUTH_SOCK` so the
//! agent leg is deterministically skipped).
#![cfg(unix)]

use std::sync::Arc;

use async_trait::async_trait;
use russh::server::{
    Auth, Config as ServerConfig, Handler as SshHandler, Msg, Server as _, Session,
};
use russh::{Channel, ChannelId, CryptoVec};
use russh_keys::key::{KeyPair, PublicKey};

use portbay_lib::registry::{SshAuthKind, SshConnection, SshConnectionId};
use portbay_lib::ssh::exec::run_command;

const PASSWORD: &str = "hunter2";

// ---------------------------------------------------------------------------
// Jump server: accept a trusted pubkey (when allowed) and bridge direct-tcpip
// channels to wherever the client asked, dialling over real TCP.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct JumpServer {
    trusted_fp: String,
    allow_key: bool,
}

impl russh::server::Server for JumpServer {
    type Handler = JumpConn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> JumpConn {
        JumpConn {
            trusted_fp: self.trusted_fp.clone(),
            allow_key: self.allow_key,
        }
    }
}

struct JumpConn {
    trusted_fp: String,
    allow_key: bool,
}

#[async_trait]
impl SshHandler for JumpConn {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        if self.allow_key && public_key.fingerprint() == self.trusted_fp {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::Reject {
                proceed_with_methods: None,
            })
        }
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let addr = format!("{host_to_connect}:{port_to_connect}");
        tokio::spawn(async move {
            match tokio::net::TcpStream::connect(addr).await {
                Ok(mut downstream) => {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut downstream).await;
                }
                Err(e) => {
                    eprintln!("jump bridge dial failed: {e}");
                }
            }
        });
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Exec server (destination): reject pubkey, accept the password, answer exec.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ExecServer;

impl russh::server::Server for ExecServer {
    type Handler = ExecConn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> ExecConn {
        ExecConn
    }
}

struct ExecConn;

#[async_trait]
impl SshHandler for ExecConn {
    type Error = russh::Error;

    // Reject keys so the destination must fall through the pipeline to the
    // password — proving the destination auth differs from the jump's.
    async fn auth_publickey(
        &mut self,
        _user: &str,
        _public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Reject {
            proceed_with_methods: None,
        })
    }

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        if password == PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::Reject {
                proceed_with_methods: None,
            })
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();
        session.data(
            channel,
            CryptoVec::from(format!("via-jump: {command}\n").into_bytes()),
        );
        session.exit_status_request(channel, 0);
        session.eof(channel);
        session.close(channel);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn start_jump(trusted_fp: String, allow_key: bool) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(ServerConfig {
        keys: vec![KeyPair::generate_ed25519().unwrap()],
        ..Default::default()
    });
    let mut server = JumpServer {
        trusted_fp,
        allow_key,
    };
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    port
}

async fn start_exec() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(ServerConfig {
        keys: vec![KeyPair::generate_ed25519().unwrap()],
        ..Default::default()
    });
    let mut server = ExecServer;
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    port
}

/// Generate an ed25519 client key as a PKCS#8 PEM and return `(path, fp)`.
fn write_client_key(dir: &std::path::Path) -> (String, String) {
    let keypair = KeyPair::generate_ed25519().unwrap();
    let fingerprint = keypair.clone_public_key().unwrap().fingerprint();
    let mut pem = Vec::new();
    russh_keys::encode_pkcs8_pem(&keypair, &mut pem).unwrap();
    let path = dir.join("id_ed25519");
    std::fs::write(&path, &pem).unwrap();
    (path.to_string_lossy().into_owned(), fingerprint)
}

fn isolate_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    home
}

/// A connection that reaches `exec_port` through the jump at `jump_port`. It
/// carries both a key (used to auth the jump) and a password (used at the
/// destination, which refuses keys).
fn jumped_connection(jump_port: u16, exec_port: u16, key_path: String) -> SshConnection {
    SshConnection {
        id: SshConnectionId::new("it-jump"),
        name: "jump test".into(),
        ssh_host: "127.0.0.1".into(),
        ssh_port: exec_port,
        ssh_user: "tester".into(),
        auth_kind: SshAuthKind::Key,
        key_path: Some(key_path),
        proxy_jump: Some(format!("127.0.0.1:{jump_port}")),
        identity_id: None,
        proxy: None,
        metadata: Default::default(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exec_round_trips_through_a_two_hop_chain() {
    let home = isolate_home();
    std::env::remove_var("SSH_AUTH_SOCK");
    let (key_path, key_fp) = write_client_key(&home.path().join(".ssh"));

    let exec_port = start_exec().await;
    let jump_port = start_jump(key_fp, true).await;
    let conn = jumped_connection(jump_port, exec_port, key_path);

    let result = run_command(&conn, Some(PASSWORD), None, None, "echo hello", None, None)
        .await
        .expect("exec should round-trip through the jump chain");
    // The marker proves the command reached the *destination* exec server,
    // having traversed jump → destination.
    assert!(
        result.stdout.contains("via-jump: echo hello"),
        "stdout was: {:?}",
        result.stdout
    );
    assert_eq!(result.exit_code, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rejecting_jump_hop_names_the_failing_hop() {
    let home = isolate_home();
    std::env::remove_var("SSH_AUTH_SOCK");
    let (key_path, key_fp) = write_client_key(&home.path().join(".ssh"));

    let exec_port = start_exec().await;
    // Jump refuses the key (and there's no agent) → the hop can't authenticate.
    let jump_port = start_jump(key_fp, false).await;
    let conn = jumped_connection(jump_port, exec_port, key_path);

    let err = run_command(&conn, Some(PASSWORD), None, None, "echo hello", None, None)
        .await
        .expect_err("a rejecting jump hop must fail the connection");
    let msg = err.to_string();
    assert!(msg.contains("jump hop 1/1"), "error was: {msg}");
}
