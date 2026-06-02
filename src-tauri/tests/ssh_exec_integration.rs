//! End-to-end tests for remote exec + deploy sequences against a real
//! in-process SSH server.
//!
//! The server answers `exec` channel requests with canned output keyed off the
//! command string (it does NOT run a real shell): a command containing "fail"
//! returns stderr + exit 1; anything else echoes a marker + exit 0. The client
//! side is our production `run_command` / `run_deploy`, so this proves the exec
//! channel plumbing (stdout/stderr/exit capture) and the deploy stop-on-failure
//! contract.

use std::sync::Arc;

use async_trait::async_trait;
use russh::server::{Auth, Handler as SshHandler, Msg, Server as _, Session};
use russh::{Channel, ChannelId, CryptoVec};
use russh_keys::key::KeyPair;

use portbay_lib::registry::{SshAuthKind, SshConnection, SshConnectionId};
use portbay_lib::ssh::exec::{run_command, run_deploy};

const PASSWORD: &str = "hunter2";

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
        if command.contains("fail") {
            session.extended_data(channel, 1, CryptoVec::from("boom\n".as_bytes().to_vec()));
            session.exit_status_request(channel, 1);
        } else {
            session.data(
                channel,
                CryptoVec::from(format!("ran: {command}\n").into_bytes()),
            );
            session.exit_status_request(channel, 0);
        }
        session.eof(channel);
        session.close(channel);
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
    let mut server = ExecServer;
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    port
}

fn connection(port: u16) -> SshConnection {
    SshConnection {
        id: SshConnectionId::new("it-exec"),
        name: "exec test".into(),
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

fn isolate_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    home
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exec_captures_stdout_and_exit_code() {
    let _home = isolate_home();
    let port = start_server().await;
    let conn = connection(port);

    let result = run_command(&conn, Some(PASSWORD), None, None, "echo hello", None, None)
        .await
        .expect("exec should run over a live handshake");
    assert!(result.stdout.contains("echo hello"));
    assert_eq!(result.exit_code, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deploy_stops_at_first_failure() {
    let _home = isolate_home();
    let port = start_server().await;
    let conn = connection(port);

    let steps = vec![
        "npm ci".to_string(),
        "fail-the-build".to_string(),
        "npm run start".to_string(),
    ];
    let results = run_deploy(&conn, Some(PASSWORD), None, None, &steps, None)
        .await
        .expect("deploy should run");

    // The third step must never run — the second failed.
    assert_eq!(
        results.len(),
        2,
        "deploy should stop after the failing step"
    );
    assert_eq!(results[0].exit_code, 0);
    assert_eq!(results[1].exit_code, 1);
    assert!(results[1].stderr.contains("boom"));
}
