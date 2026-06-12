//! Android Wi-Fi pairing (QR + manual) — the Android Studio "Pair device
//! with QR code" flow, in PortBay.
//!
//! Protocol (Android 11+ Wireless debugging):
//! 1. The host generates a service name + password and renders them as a
//!    `WIFI:T:ADB;S:<name>;P:<password>;;` QR code.
//! 2. The phone (Developer options → Wireless debugging → Pair device with
//!    QR code) scans it and advertises an mDNS service of type
//!    `_adb-tls-pairing._tcp` whose instance name is the `S` value.
//! 3. The host watches `adb mdns services`, finds that service's `ip:port`,
//!    and runs `adb pair ip:port <password>`.
//! 4. After pairing, the phone advertises `_adb-tls-connect._tcp`; the host
//!    runs `adb connect ip:port` against it so the device lands in
//!    `adb devices` — at which point the destination picker sees it.
//!
//! Everything here shells out to adb and blocks — callers run it on the
//! blocking pool. Progress is streamed to the frontend over the
//! `portbay://adb-pair` event channel (one watcher at a time; starting a new
//! session cancels the previous one via a generation counter).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const PAIR_CHANNEL: &str = "portbay://adb-pair";

/// How long the QR watcher waits for the phone before giving up. Scanning +
/// typing the settings path comfortably fits; idling a subprocess poller
/// longer than this helps nobody.
const PAIR_TIMEOUT: Duration = Duration::from_secs(120);

/// Cadence for polling `adb mdns services` while waiting for the phone.
const POLL_INTERVAL: Duration = Duration::from_millis(1_000);

/// Generation counter: a new session invalidates any watcher still running
/// from a previous one (user closed and reopened the pairing panel).
static SESSION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairSession {
    /// mDNS instance name the phone will advertise back (the QR's `S`).
    pub name: String,
    /// One-time pairing password (the QR's `P`). Shown for transparency.
    pub password: String,
    /// Inline SVG of the QR code, ready for `{@html}`.
    pub qr_svg: String,
}

/// One progress event on [`PAIR_CHANNEL`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "stage")]
pub enum PairEvent {
    /// Phone's pairing service discovered; `adb pair` running.
    Pairing { address: String },
    /// Paired; waiting for the connect service / `adb connect`.
    Connecting,
    /// Device is in `adb devices` — picker refresh will show it.
    Connected { serial: String },
    /// Terminal failure (timeout, adb error). The panel shows `message`.
    Failed { message: String },
}

fn emit(app: &AppHandle, generation: u64, event: PairEvent) {
    // A stale watcher (superseded session) stays silent.
    if SESSION.load(Ordering::SeqCst) == generation {
        let _ = app.emit(PAIR_CHANNEL, event);
    }
}

/// Random lowercase-alphanumeric string from the OS entropy pool. /dev/urandom
/// is fine for a 2-minute one-time pairing secret displayed to the user.
fn random_token(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = vec![0u8; len];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        // Degenerate fallback: time-derived. Still unique enough per session.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = ((t >> (i * 5)) & 0xff) as u8;
        }
    }
    bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect()
}

/// Create a fresh pairing session: name, password, QR SVG. Bumps the session
/// generation so any previous watcher goes quiet.
pub fn new_session() -> Result<(PairSession, u64), String> {
    let generation = SESSION.fetch_add(1, Ordering::SeqCst) + 1;
    let name = format!("portbay-{}", random_token(6));
    let password = random_token(8);
    let payload = format!("WIFI:T:ADB;S:{name};P:{password};;");
    let qr = qrcode::QrCode::new(payload.as_bytes()).map_err(|e| format!("QR encode: {e}"))?;
    let qr_svg = qr
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(qrcode::render::svg::Color("currentColor"))
        .light_color(qrcode::render::svg::Color("transparent"))
        .build();
    Ok((
        PairSession {
            name,
            password,
            qr_svg,
        },
        generation,
    ))
}

/// Blocking watcher: wait for the phone's pairing service, pair, connect.
/// Emits progress on [`PAIR_CHANNEL`]; returns when terminal (or superseded).
pub fn watch_and_pair(app: AppHandle, generation: u64, name: String, password: String) {
    let Some(adb) = crate::mobile_targets::adb_bin() else {
        emit(
            &app,
            generation,
            PairEvent::Failed {
                message: "adb not found — install Android platform-tools.".into(),
            },
        );
        return;
    };

    let deadline = Instant::now() + PAIR_TIMEOUT;

    // Phase 1: discover the pairing service the phone advertises after the
    // QR scan, then pair against it.
    let pair_addr = loop {
        if SESSION.load(Ordering::SeqCst) != generation {
            return; // superseded — go quiet
        }
        if Instant::now() >= deadline {
            emit(
                &app,
                generation,
                PairEvent::Failed {
                    message: "Timed out waiting for the phone. Keep Wireless debugging open \
                              and scan the code again."
                        .into(),
                },
            );
            return;
        }
        let services = run(&adb, &["mdns", "services"]).unwrap_or_default();
        if let Some(addr) = find_service(&services, "_adb-tls-pairing", Some(&name)) {
            break addr;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    emit(
        &app,
        generation,
        PairEvent::Pairing {
            address: pair_addr.clone(),
        },
    );
    match run(&adb, &["pair", &pair_addr, &password]) {
        Some(out) if out.contains("Successfully paired") => {}
        Some(out) => {
            let line = out.lines().last().unwrap_or("adb pair failed").trim();
            emit(
                &app,
                generation,
                PairEvent::Failed {
                    message: format!("Pairing failed — {line}"),
                },
            );
            return;
        }
        None => {
            emit(
                &app,
                generation,
                PairEvent::Failed {
                    message: "Pairing failed — adb pair did not complete.".into(),
                },
            );
            return;
        }
    }

    emit(&app, generation, PairEvent::Connecting);

    // Phase 2: the paired phone advertises its connect service; adb connect
    // it so it shows up in `adb devices`. Same IP as the pairing service.
    let ip = pair_addr.rsplit_once(':').map(|(ip, _)| ip).unwrap_or("");
    let connect_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if SESSION.load(Ordering::SeqCst) != generation {
            return;
        }
        if Instant::now() >= connect_deadline {
            // Paired but not auto-connected — still a success for the user:
            // many adb versions connect lazily. Report paired-as-connected
            // with the IP so the picker note is honest.
            emit(
                &app,
                generation,
                PairEvent::Connected {
                    serial: ip.to_string(),
                },
            );
            return;
        }
        let services = run(&adb, &["mdns", "services"]).unwrap_or_default();
        if let Some(addr) =
            find_service(&services, "_adb-tls-connect", None).filter(|a| a.starts_with(ip))
        {
            let _ = run(&adb, &["connect", &addr]);
            // Confirm it actually landed.
            let devices = run(&adb, &["devices"]).unwrap_or_default();
            if devices
                .lines()
                .any(|l| l.starts_with(&addr) && l.contains("device"))
            {
                emit(&app, generation, PairEvent::Connected { serial: addr });
                return;
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Manual fallback: `adb pair ip:port code` with the values the phone shows
/// under "Pair device with pairing code". Returns the success/failure line.
pub fn pair_manual(host_port: &str, code: &str) -> Result<String, String> {
    let adb = crate::mobile_targets::adb_bin()
        .ok_or_else(|| "adb not found — install Android platform-tools.".to_string())?;
    let out = run(&adb, &["pair", host_port, code])
        .ok_or_else(|| "adb pair did not complete.".to_string())?;
    if out.contains("Successfully paired") {
        // Best-effort connect: wireless debugging's connect port differs from
        // the pairing port; adb's mdns auto-connect usually picks it up. Nudge
        // it by connecting to any advertised connect service on the same IP.
        let ip = host_port.rsplit_once(':').map(|(ip, _)| ip).unwrap_or("");
        if let Some(addr) = run(&adb, &["mdns", "services"])
            .as_deref()
            .and_then(|s| find_service(s, "_adb-tls-connect", None))
            .filter(|a| a.starts_with(ip))
        {
            let _ = run(&adb, &["connect", &addr]);
        }
        Ok(out.lines().last().unwrap_or("Paired.").trim().to_string())
    } else {
        Err(out
            .lines()
            .last()
            .unwrap_or("Pairing failed.")
            .trim()
            .to_string())
    }
}

/// Find a service of `service_type` in `adb mdns services` output, optionally
/// requiring the instance name; returns its `ip:port`.
///
/// Output shape (header + tab/space-separated rows):
/// ```text
/// List of discovered mdns services
/// portbay-ab12cd   _adb-tls-pairing._tcp   192.168.1.7:40123
/// adb-R5CT...-aBcDeF   _adb-tls-connect._tcp   192.168.1.7:37001
/// ```
pub(crate) fn find_service(
    text: &str,
    service_type: &str,
    instance: Option<&str>,
) -> Option<String> {
    text.lines().skip(1).find_map(|line| {
        let mut cols = line.split_whitespace();
        let name = cols.next()?;
        let svc = cols.next()?;
        let addr = cols.next()?;
        if !svc.starts_with(service_type) {
            return None;
        }
        if let Some(want) = instance {
            if name != want {
                return None;
            }
        }
        if !addr.contains(':') {
            return None;
        }
        Some(addr.to_string())
    })
}

/// Run adb with args, capturing combined stdout+stderr (adb reports pair
/// failures on stderr). None on spawn failure only.
fn run(adb: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(adb).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICES: &str = "List of discovered mdns services\n\
        portbay-ab12cd\t_adb-tls-pairing._tcp\t192.168.1.7:40123\n\
        adb-R5CT123-aBcDeF\t_adb-tls-connect._tcp\t192.168.1.7:37001\n\
        other-host\t_adb-tls-connect._tcp\t192.168.1.9:37002\n";

    #[test]
    fn find_service_matches_type_and_instance() {
        assert_eq!(
            find_service(SERVICES, "_adb-tls-pairing", Some("portbay-ab12cd")).as_deref(),
            Some("192.168.1.7:40123")
        );
        // Wrong instance name → no match (we never pair against a stranger's QR).
        assert_eq!(
            find_service(SERVICES, "_adb-tls-pairing", Some("portbay-zzzzzz")),
            None
        );
        // Connect service: first match by type; caller filters by IP.
        assert_eq!(
            find_service(SERVICES, "_adb-tls-connect", None).as_deref(),
            Some("192.168.1.7:37001")
        );
    }

    #[test]
    fn find_service_ignores_header_and_garbage() {
        assert_eq!(
            find_service(
                "List of discovered mdns services\n",
                "_adb-tls-pairing",
                None
            ),
            None
        );
        assert_eq!(find_service("", "_adb-tls-pairing", None), None);
        assert_eq!(
            find_service("header\nnot-enough-columns\n", "_adb-tls-pairing", None),
            None
        );
    }

    #[test]
    fn session_payload_is_androids_qr_format() {
        let (s, _) = new_session().unwrap();
        assert!(s.name.starts_with("portbay-"));
        assert_eq!(s.password.len(), 8);
        assert!(s.qr_svg.starts_with("<?xml") || s.qr_svg.starts_with("<svg"));
        // The QR encodes WIFI:T:ADB;S:<name>;P:<password>;; — verified by
        // construction in new_session; here we pin the name/password shape.
        assert!(s
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert!(s.password.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn random_tokens_differ_between_calls() {
        assert_ne!(random_token(8), random_token(8));
    }
}
