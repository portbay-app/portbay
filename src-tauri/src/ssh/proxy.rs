//! Forward-proxy handshakes for the in-process russh transport.
//!
//! A saved connection may sit behind a SOCKS5 or HTTP CONNECT proxy. When it
//! does, the first transport hop is dialled to the proxy and a CONNECT
//! handshake negotiates a raw tunnel to the real SSH target (or the first jump
//! host); russh then speaks SSH over the returned stream exactly as if it had
//! dialled the target directly. Both schemes are implemented by hand so we add
//! no dependency: SOCKS5 per RFC 1928 with optional RFC 1929 username/password,
//! and HTTP CONNECT with an optional Basic `Proxy-Authorization` header.

use base64::Engine;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::registry::{SshProxyConfig, SshProxyKind};
use crate::ssh::backend::{Result, SshError};

/// Dial `proxy` and negotiate a tunnel to `target_host:target_port`, returning
/// the connected stream ready for `client::connect_stream`. `proxy_password` is
/// consulted only for an authenticated proxy (one carrying a username).
pub async fn connect_via_proxy(
    proxy: &SshProxyConfig,
    target_host: &str,
    target_port: u16,
    proxy_password: Option<&str>,
) -> Result<TcpStream> {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| proxy_err(proxy, format!("failed to connect to proxy: {e}")))?;

    let username = proxy
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let password = proxy_password.map(str::trim).filter(|s| !s.is_empty());

    let outcome = match proxy.kind {
        SshProxyKind::Socks5 => {
            socks5_connect(&mut stream, target_host, target_port, username, password).await
        }
        SshProxyKind::Http => {
            http_connect(&mut stream, target_host, target_port, username, password).await
        }
    };
    outcome.map_err(|e| proxy_err(proxy, e))?;
    Ok(stream)
}

/// Tag a handshake failure with the proxy scheme + address for an unambiguous
/// error (`proxy (socks5 10.0.0.1:1080): …`).
fn proxy_err(proxy: &SshProxyConfig, msg: impl std::fmt::Display) -> SshError {
    let scheme = match proxy.kind {
        SshProxyKind::Socks5 => "socks5",
        SshProxyKind::Http => "http",
    };
    SshError::Russh(format!(
        "proxy ({scheme} {}:{}): {msg}",
        proxy.host, proxy.port
    ))
}

/// Run a SOCKS5 CONNECT handshake to `target_host:target_port`. Offers no-auth
/// and, when a `username` is configured, RFC 1929 username/password. The target
/// is addressed by domain name (ATYP `0x03`) so the proxy resolves it.
async fn socks5_connect<S>(
    stream: &mut S,
    target_host: &str,
    target_port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> std::result::Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // 1. Greeting — advertise the methods we can satisfy.
    let methods: &[u8] = if username.is_some() {
        &[0x00, 0x02]
    } else {
        &[0x00]
    };
    let mut greeting = vec![0x05u8, methods.len() as u8];
    greeting.extend_from_slice(methods);
    stream
        .write_all(&greeting)
        .await
        .map_err(|e| format!("writing greeting failed: {e}"))?;

    // 2. Method selection.
    let mut selection = [0u8; 2];
    stream
        .read_exact(&mut selection)
        .await
        .map_err(|e| format!("reading method selection failed: {e}"))?;
    if selection[0] != 0x05 {
        return Err(format!(
            "unexpected SOCKS version {:#04x} in method selection",
            selection[0]
        ));
    }
    match selection[1] {
        0x00 => {} // no authentication required
        0x02 => {
            let user = username.ok_or_else(|| {
                "proxy demanded username/password but none is configured".to_string()
            })?;
            let pass = password.unwrap_or("");
            if user.len() > 255 || pass.len() > 255 {
                return Err(
                    "proxy username/password too long for SOCKS5 (max 255 bytes each)".into(),
                );
            }
            let mut auth = vec![0x01u8]; // RFC 1929 subnegotiation version
            auth.push(user.len() as u8);
            auth.extend_from_slice(user.as_bytes());
            auth.push(pass.len() as u8);
            auth.extend_from_slice(pass.as_bytes());
            stream
                .write_all(&auth)
                .await
                .map_err(|e| format!("writing username/password failed: {e}"))?;
            let mut reply = [0u8; 2];
            stream
                .read_exact(&mut reply)
                .await
                .map_err(|e| format!("reading auth reply failed: {e}"))?;
            if reply[1] != 0x00 {
                return Err("proxy rejected the username/password".into());
            }
        }
        0xFF => return Err("proxy offered no acceptable authentication method".into()),
        other => {
            return Err(format!(
                "proxy selected an unsupported SOCKS5 method {other:#04x}"
            ))
        }
    }

    // 3. CONNECT request (ATYP 0x03 = domain name).
    if target_host.len() > 255 {
        return Err("target host too long for SOCKS5 (max 255 bytes)".into());
    }
    let mut request = vec![0x05u8, 0x01, 0x00, 0x03, target_host.len() as u8];
    request.extend_from_slice(target_host.as_bytes());
    request.extend_from_slice(&target_port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|e| format!("writing CONNECT request failed: {e}"))?;

    // 4. Reply: VER REP RSV ATYP BND.ADDR BND.PORT.
    let mut head = [0u8; 4];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|e| format!("reading CONNECT reply failed: {e}"))?;
    if head[0] != 0x05 {
        return Err(format!(
            "unexpected SOCKS version {:#04x} in CONNECT reply",
            head[0]
        ));
    }
    if head[1] != 0x00 {
        return Err(format!("CONNECT failed: {}", socks5_reply_message(head[1])));
    }
    // Drain the bound address + port so the stream is positioned at the tunnel.
    let addr_len = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut len = [0u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|e| format!("reading bound-address length failed: {e}"))?;
            len[0] as usize
        }
        other => {
            return Err(format!(
                "proxy replied with unknown address type {other:#04x}"
            ))
        }
    };
    let mut discard = vec![0u8; addr_len + 2];
    stream
        .read_exact(&mut discard)
        .await
        .map_err(|e| format!("reading bound address failed: {e}"))?;
    Ok(())
}

/// Map a SOCKS5 reply code (REP field) to a human-readable reason.
fn socks5_reply_message(code: u8) -> &'static str {
    match code {
        0x01 => "general SOCKS server failure",
        0x02 => "connection not allowed by ruleset",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown failure",
    }
}

/// Run an HTTP CONNECT handshake to `target_host:target_port`, sending a Basic
/// `Proxy-Authorization` header when a `username` is configured.
async fn http_connect<S>(
    stream: &mut S,
    target_host: &str,
    target_port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> std::result::Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let authority = format!("{target_host}:{target_port}");
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(user) = username {
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{user}:{}", password.unwrap_or("")));
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("writing CONNECT request failed: {e}"))?;

    // Read the status line + headers up to the terminating blank line. A CONNECT
    // 2xx response carries no body, so the byte after `\r\n\r\n` starts the
    // tunnel; reading exactly that far leaves the stream correctly positioned.
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = stream
            .read(&mut byte)
            .await
            .map_err(|e| format!("reading CONNECT reply failed: {e}"))?;
        if n == 0 {
            return Err("proxy closed the connection during CONNECT".into());
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8192 {
            return Err("CONNECT response header block too large".into());
        }
    }

    let head = String::from_utf8_lossy(&buf);
    let status_line = head.lines().next().unwrap_or("");
    // e.g. "HTTP/1.1 200 Connection established".
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| format!("malformed CONNECT status line: {status_line:?}"))?;
    if (200..300).contains(&code) {
        Ok(())
    } else if code == 407 {
        Err("proxy requires authentication (407) — check the proxy username/password".into())
    } else {
        Err(format!("CONNECT failed with HTTP status {code}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Read the SOCKS5 CONNECT request after method selection and assert it
    /// targets `host:port` via ATYP 0x03; returns once the request is drained.
    async fn expect_connect_request(server: &mut tokio::io::DuplexStream, host: &str, port: u16) {
        let mut head = [0u8; 5];
        server.read_exact(&mut head).await.unwrap();
        assert_eq!(&head[..4], &[0x05, 0x01, 0x00, 0x03], "CONNECT header");
        let hlen = head[4] as usize;
        let mut rest = vec![0u8; hlen + 2];
        server.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest[..hlen], host.as_bytes(), "target host");
        assert_eq!(&rest[hlen..], &port.to_be_bytes(), "target port");
    }

    #[tokio::test]
    async fn socks5_no_auth_connect_succeeds() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            socks5_connect(&mut client, "example.com", 22, None, None).await
        });

        let mut greeting = [0u8; 3];
        server.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [0x05, 0x01, 0x00], "no-auth greeting");
        server.write_all(&[0x05, 0x00]).await.unwrap();

        expect_connect_request(&mut server, "example.com", 22).await;
        // Success reply, ATYP IPv4 0.0.0.0:0.
        server
            .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();

        task.await.unwrap().expect("handshake should succeed");
    }

    #[tokio::test]
    async fn socks5_username_password_connect_succeeds() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            socks5_connect(
                &mut client,
                "db.internal",
                5432,
                Some("alice"),
                Some("s3cret"),
            )
            .await
        });

        let mut greeting = [0u8; 4];
        server.read_exact(&mut greeting).await.unwrap();
        assert_eq!(
            greeting,
            [0x05, 0x02, 0x00, 0x02],
            "offers no-auth + user/pass"
        );
        // Select username/password.
        server.write_all(&[0x05, 0x02]).await.unwrap();

        // RFC 1929: 01 ulen user plen pass.
        let mut hdr = [0u8; 2];
        server.read_exact(&mut hdr).await.unwrap();
        assert_eq!(hdr[0], 0x01, "RFC 1929 version");
        let ulen = hdr[1] as usize;
        let mut user = vec![0u8; ulen];
        server.read_exact(&mut user).await.unwrap();
        assert_eq!(&user, b"alice");
        let mut plen = [0u8; 1];
        server.read_exact(&mut plen).await.unwrap();
        let mut pass = vec![0u8; plen[0] as usize];
        server.read_exact(&mut pass).await.unwrap();
        assert_eq!(&pass, b"s3cret");
        // Auth success.
        server.write_all(&[0x01, 0x00]).await.unwrap();

        expect_connect_request(&mut server, "db.internal", 5432).await;
        server
            .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();

        task.await
            .unwrap()
            .expect("authenticated handshake should succeed");
    }

    #[tokio::test]
    async fn socks5_connect_refused_is_an_error() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            socks5_connect(&mut client, "denied.host", 22, None, None).await
        });

        let mut greeting = [0u8; 3];
        server.read_exact(&mut greeting).await.unwrap();
        server.write_all(&[0x05, 0x00]).await.unwrap();
        expect_connect_request(&mut server, "denied.host", 22).await;
        // REP 0x05 = connection refused.
        server
            .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();

        let err = task.await.unwrap().expect_err("refused CONNECT must error");
        assert!(err.contains("connection refused"), "got: {err}");
    }

    #[tokio::test]
    async fn http_connect_200_succeeds_with_basic_auth() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            http_connect(&mut client, "host.example", 22, Some("bob"), Some("pw")).await
        });

        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            server.read_exact(&mut byte).await.unwrap();
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let req = String::from_utf8(buf).unwrap();
        assert!(
            req.starts_with("CONNECT host.example:22 HTTP/1.1\r\n"),
            "req: {req:?}"
        );
        assert!(req.contains("Host: host.example:22\r\n"));
        // base64("bob:pw") == "Ym9iOnB3".
        assert!(
            req.contains("Proxy-Authorization: Basic Ym9iOnB3\r\n"),
            "req: {req:?}"
        );

        server
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        task.await.unwrap().expect("200 CONNECT should succeed");
    }

    #[tokio::test]
    async fn http_connect_407_is_an_auth_error() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let task =
            tokio::spawn(
                async move { http_connect(&mut client, "host.example", 22, None, None).await },
            );

        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            server.read_exact(&mut byte).await.unwrap();
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        server
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
            .await
            .unwrap();

        let err = task.await.unwrap().expect_err("407 must error");
        assert!(err.contains("407"), "got: {err}");
    }
}
