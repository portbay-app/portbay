//! Integration test for the SSH tunnel reconnect supervisor against a *live*
//! SSH handshake — no mocks, no `#[ignore]`, no external `sshd`.
//!
//! The pure backoff/state machine is unit-tested in `src/ssh/manager.rs`. This
//! proves the other half of the P0 contract end to end, in-process and CI-safe:
//!
//!   1. `SshManager::start` performs a real russh handshake + password auth and
//!      brings a local (-L) forward up — bytes flow through it (UP).
//!   2. When the upstream session is dropped, the manager observes the tunnel as
//!      no-longer-running (the `is_closed()` liveness probe), so the supervisor
//!      can act — instead of the old behaviour where a russh tunnel looked "up"
//!      forever after a silent drop.
//!   3. `reconnect_due()` restores the forward once the server is reachable
//!      again, and traffic flows once more.
//!   4. `stop` tears the forward down (DOWN).
//!
//! Everything runs over loopback against an in-process russh **server** that
//! pipes `direct-tcpip` channels to a tiny echo service — the exact shape of a
//! `-L` forward to a remote TCP service.
//!
//! It drives the *password* path on purpose: key auth shells out to the system
//! `ssh` binary (needs a real `sshd`, not CI-friendly), whereas the password
//! path is the in-process russh client we can exercise deterministically. Both
//! share the same supervisor, `status()`, and `reconnect_due()` code.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::keys::{key::safe_rng, Algorithm, PrivateKey};
use russh::server::{Auth, Config, Handler, Msg, Server as _, Session};
use russh::{Channel, Disconnect};
use tokio::runtime::Runtime;
use tokio::sync::Mutex as TokioMutex;

use portbay_lib::registry::{SshAuthKind, SshConnectionId, SshForwardKind, SshTunnelId};
use portbay_lib::ssh::backend::EffectiveSshTunnel;
use portbay_lib::ssh::SshManager;

const PASSWORD: &str = "hunter2";

/// In-process SSH server. Accepts a fixed password and forwards every
/// `direct-tcpip` channel (what a `-L` forward opens) to the address the client
/// requested, piping bytes both ways. Session handles are tracked so the test
/// can sever a live connection on demand to simulate a dropped link.
#[derive(Clone)]
struct TestServer {
    handles: Arc<TokioMutex<Vec<russh::server::Handle>>>,
}

impl russh::server::Server for TestServer {
    type Handler = TestServer;
    fn new_client(&mut self, _: Option<SocketAddr>) -> TestServer {
        self.clone()
    }
}

impl Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        if password == PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.handles.lock().await.push(session.handle());
        let addr = format!("{host_to_connect}:{port_to_connect}");
        tokio::spawn(async move {
            let Ok(mut upstream) = tokio::net::TcpStream::connect(&addr).await else {
                return;
            };
            let mut chan = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut chan, &mut upstream).await;
        });
        Ok(true)
    }
}

/// Connect to `port`, send a line, and confirm it echoes back unchanged.
fn echo_roundtrip(port: u16) -> bool {
    use std::io::{Read, Write};
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    if s.write_all(b"ping\n").is_err() {
        return false;
    }
    let mut buf = [0u8; 5];
    s.read_exact(&mut buf).is_ok() && &buf == b"ping\n"
}

/// Grab an ephemeral port the OS hands out, then release it for the tunnel to
/// bind. (A small TOCTOU window, fine for a loopback test.)
fn free_local_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn live_handshake_drop_reconnect_and_stop() {
    // Isolate known_hosts writes: the client learns the server's host key on
    // first use, and we don't want that touching the developer's ~/.ssh.
    // `dirs::home_dir()` (used by russh-keys) reads $HOME on unix.
    let home = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", home.path());
    std::fs::create_dir_all(home.path().join(".ssh")).expect("mk .ssh");

    // One runtime hosts the echo service + SSH server for the whole test. The
    // tunnel client runs in its own thread+runtime inside the manager.
    let server_rt = Runtime::new().expect("server runtime");

    // Echo service: every accepted connection mirrors its bytes back.
    let echo_addr = server_rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = sock.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });
        addr
    });

    // SSH server bound to an ephemeral port.
    let handles = Arc::new(TokioMutex::new(Vec::new()));
    let server = TestServer {
        handles: handles.clone(),
    };
    let ssh_port = server_rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Arc::new(Config {
            keys: vec![PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap()],
            ..Default::default()
        });
        let mut server = server.clone();
        tokio::spawn(async move {
            let _ = server.run_on_socket(config, &listener).await;
        });
        port
    });

    let local_port = free_local_port();
    let profile = EffectiveSshTunnel {
        id: SshTunnelId::new("it"),
        connection_id: SshConnectionId::new("it-conn"),
        name: "Integration tunnel".into(),
        ssh_host: "127.0.0.1".into(),
        ssh_port,
        ssh_user: "tester".into(),
        auth_kind: SshAuthKind::Password,
        key_path: None,
        proxy_jump: None,
        local_host: "127.0.0.1".into(),
        local_port,
        remote_host: "127.0.0.1".into(),
        remote_port: echo_addr.port(),
        forward_kind: SshForwardKind::Local,
        keep_alive: false,
        auto_reconnect: true,
    };
    let profiles = std::slice::from_ref(&profile);

    let mut mgr = SshManager::new();

    // (1) UP — a real handshake + password auth, then live forwarded traffic.
    // No interactor: headless test path — silent TOFU against the throwaway
    // server container, same trust model this suite has always exercised.
    mgr.start(profile.clone(), Some(PASSWORD.to_string().into()), None)
        .expect("tunnel should start over a live SSH handshake");
    assert!(
        poll_until(Duration::from_secs(5), || echo_roundtrip(local_port)),
        "forward should carry echo traffic while up"
    );
    assert!(
        mgr.list(profiles)[0].running,
        "manager should report it live"
    );

    // (2) DROP — sever the live session from the server side. The client's
    // is_closed() probe must flip the tunnel to not-running.
    server_rt.block_on(async {
        let mut guard = handles.lock().await;
        for handle in guard.iter() {
            let _ = handle
                .disconnect(
                    Disconnect::ByApplication,
                    "test drop".to_string(),
                    String::new(),
                )
                .await;
        }
        guard.clear();
    });
    assert!(
        poll_until(Duration::from_secs(5), || !mgr.list(profiles)[0].running),
        "a dropped russh session must be observed as not-running (the liveness probe)"
    );
    assert_eq!(
        mgr.list(profiles)[0].state,
        portbay_lib::ssh::manager::SshTunnelState::Reconnecting,
        "auto-reconnect tunnel should report Reconnecting, not Down, after a drop"
    );

    // (3) RECONNECT — the server is still listening; the supervisor restores it.
    // reconnect_due() self-gates on backoff, so poll it until the forward is
    // back (the freed local port may take a beat to rebind).
    assert!(
        poll_until(Duration::from_secs(10), || {
            mgr.reconnect_due();
            mgr.list(profiles)[0].running
        }),
        "supervisor should restore the tunnel once the server is reachable again"
    );
    assert!(
        poll_until(Duration::from_secs(5), || echo_roundtrip(local_port)),
        "restored forward should carry traffic again"
    );

    // (4) DOWN — stop tears the forward down for good.
    mgr.stop("it").expect("stop should succeed");
    assert!(
        poll_until(Duration::from_secs(3), || {
            std::net::TcpStream::connect(("127.0.0.1", local_port)).is_err()
        }),
        "local port should stop accepting once the tunnel is stopped"
    );
}
