//! Ground-truth test for the client-side password auth pipeline against a real
//! in-process russh server. Mirrors the "Open Host" path: `connect_session`
//! with an inline password, the same call `ssh_exec_run` makes on the retry.

use std::sync::Arc;

use std::borrow::Cow;

use async_trait::async_trait;
use russh::server::{Auth, Handler as SshHandler, Msg, Response, Session};
use russh::{Channel, MethodSet};
use russh_keys::key::KeyPair;

use portbay_lib::registry::{SshAuthKind, SshConnection, SshConnectionId};
use portbay_lib::ssh::connect_session;

const GOOD_PASSWORD: &str = "correct-horse";

/// A server that accepts `GOOD_PASSWORD` over the plain `password` method.
#[derive(Clone)]
struct PasswordServer;

impl russh::server::Server for PasswordServer {
    type Handler = PasswordConn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> PasswordConn {
        PasswordConn
    }
}

struct PasswordConn;

#[async_trait]
impl SshHandler for PasswordConn {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        if password == GOOD_PASSWORD {
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
}

/// A server that refuses `password` and only accepts via keyboard-interactive,
/// echoing the supplied secret — the common modern-sshd (PAM) shape.
#[derive(Clone)]
struct KiServer;

impl russh::server::Server for KiServer {
    type Handler = KiConn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> KiConn {
        KiConn
    }
}

struct KiConn;

#[async_trait]
impl SshHandler for KiConn {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        // Decline password; tell the client to try keyboard-interactive.
        Ok(Auth::Reject {
            proceed_with_methods: Some(MethodSet::KEYBOARD_INTERACTIVE),
        })
    }

    async fn auth_keyboard_interactive(
        &mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'async_trait>>,
    ) -> Result<Auth, Self::Error> {
        match response {
            // First call: send a single password prompt.
            None => Ok(Auth::Partial {
                name: Cow::Borrowed("Password"),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
            }),
            // Subsequent call: accept if the echoed answer matches.
            Some(mut response) => {
                let answer = response
                    .next()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_default();
                if answer == GOOD_PASSWORD {
                    Ok(Auth::Accept)
                } else {
                    Ok(Auth::Reject {
                        proceed_with_methods: None,
                    })
                }
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

async fn start<S>(server: S) -> u16
where
    S: russh::server::Server + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(russh::server::Config {
        keys: vec![KeyPair::generate_ed25519().unwrap()],
        ..Default::default()
    });
    let mut server = server;
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    port
}

fn connection(port: u16) -> SshConnection {
    SshConnection {
        id: SshConnectionId::new("it-pw-auth"),
        name: "pw auth test".into(),
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
async fn correct_password_authenticates_over_password_method() {
    let _home = isolate_home();
    let port = start(PasswordServer).await;
    let conn = connection(port);

    let result = connect_session(&conn, Some(GOOD_PASSWORD), None, None, None).await;
    assert!(
        result.is_ok(),
        "a correct password must authenticate, got: {:?}",
        result.err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_password_fails_without_claiming_missing_password() {
    let _home = isolate_home();
    let port = start(PasswordServer).await;
    let conn = connection(port);

    let msg = match connect_session(&conn, Some("nope"), None, None, None).await {
        Ok(_) => panic!("a wrong password must not authenticate"),
        Err(e) => e.to_string(),
    };
    assert!(
        !msg.contains("needs an SSH password"),
        "a supplied-but-rejected password must not report MissingPassword, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn correct_password_authenticates_over_keyboard_interactive() {
    let _home = isolate_home();
    let port = start(KiServer).await;
    let conn = connection(port);

    let result = connect_session(&conn, Some(GOOD_PASSWORD), None, None, None).await;
    assert!(
        result.is_ok(),
        "a KI-only server must authenticate via the keyboard-interactive fallback, got: {:?}",
        result.err()
    );
}
