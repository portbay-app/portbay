//! End-to-end test for the read-only host probe against a real in-process SSH
//! server.
//!
//! The probe never authenticates — it completes the transport handshake (which
//! delivers the server's host key) and drops. So the test server only needs a
//! host key and to exist; it never has to accept a password or open a channel.
//! We assert the probe reports the host reachable, with a latency, the server's
//! SHA256 fingerprint, and `trust == "new"` (the host is absent from the
//! isolated `known_hosts`).

use std::sync::Arc;

use async_trait::async_trait;
use russh::server::{Auth, Handler as SshHandler, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_keys::key::KeyPair;

use portbay_lib::registry::{SshAuthKind, SshConnection, SshConnectionId};
use portbay_lib::ssh::probe::{probe_connection, HostTrust, ProbeHealth};

#[derive(Clone)]
struct ProbeServer;

impl russh::server::Server for ProbeServer {
    type Handler = ProbeConn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> ProbeConn {
        ProbeConn
    }
}

struct ProbeConn;

#[async_trait]
impl SshHandler for ProbeConn {
    type Error = russh::Error;

    // The probe never reaches auth, but a Handler must implement it.
    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
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
        _channel: ChannelId,
        _data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

async fn start_server() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(russh::server::Config {
        keys: vec![KeyPair::generate_ed25519().unwrap()],
        ..Default::default()
    });
    let mut server = ProbeServer;
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    port
}

fn connection(port: u16) -> SshConnection {
    SshConnection {
        id: SshConnectionId::new("it-probe"),
        name: "probe test".into(),
        ssh_host: "127.0.0.1".into(),
        ssh_port: port,
        ssh_user: "tester".into(),
        auth_kind: SshAuthKind::Password,
        key_path: None,
        proxy_jump: None,
        identity_id: None,
        proxy: None,
        metadata: Default::default(),
    }
}

/// Isolate `known_hosts` so the host reads as a first contact (`new`), and so
/// the probe's read can never touch the developer's real file.
fn isolate_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    home
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_reports_reachable_with_fingerprint_and_new_trust() {
    let _home = isolate_home();
    let port = start_server().await;
    let conn = connection(port);

    let result = probe_connection(&conn).await;

    assert!(result.reachable, "server should answer the TCP dial");
    assert!(
        result.latency_ms.is_some(),
        "a completed handshake yields a latency"
    );
    let fingerprint = result.fingerprint.expect("host key should be captured");
    assert!(
        fingerprint.starts_with("SHA256:"),
        "fingerprint should be OpenSSH SHA256 form, got {fingerprint}"
    );
    assert_eq!(
        result.trust,
        HostTrust::New,
        "host is absent from the isolated known_hosts"
    );
    assert!(
        matches!(result.health, ProbeHealth::Healthy | ProbeHealth::Degraded),
        "a reachable host is healthy or degraded, never down"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_reports_down_for_an_unreachable_host() {
    let _home = isolate_home();
    // Port 1 on loopback has nothing listening → connection refused.
    let mut conn = connection(1);
    conn.ssh_host = "127.0.0.1".into();

    let result = probe_connection(&conn).await;

    assert!(!result.reachable);
    assert_eq!(result.health, ProbeHealth::Down);
    assert!(result.fingerprint.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_skips_jumped_hosts_as_unknown() {
    let _home = isolate_home();
    let port = start_server().await;
    let mut conn = connection(port);
    conn.proxy_jump = Some("bastion.example.net".into());

    let result = probe_connection(&conn).await;

    // A jumped host isn't dialled directly; we report indeterminate, not a
    // misleading "down".
    assert!(!result.reachable);
    assert_eq!(result.health, ProbeHealth::Unknown);
    assert_eq!(result.trust, HostTrust::Unknown);
}
