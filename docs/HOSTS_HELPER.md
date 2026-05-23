# Privileged Hosts Helper

PortBay can manage `/etc/hosts` through a small privileged helper instead of prompting for `sudo` on every hostname change.

## Components

| Component | Path | Purpose |
| --- | --- | --- |
| Helper binary | `src-tauri/src/bin/portbay-hosts-helper.rs` | LaunchDaemon executable that owns privileged hosts writes. |
| Protocol and client | `src-tauri/src/hosts_helper.rs` | Line-delimited JSON RPC, suffix validation, CLI fallback client. |
| LaunchDaemon plist | `src-tauri/macos/com.portbay-app.portbay.hosts-helper.plist` | SMAppService daemon registration payload. |
| Hosts parser | `src-tauri/src/hosts.rs` | Existing delimited-block parser and atomic rewriter. |

## Protocol

The helper listens on:

```text
/var/run/portbay-hosts-helper.sock
```

Requests are single-line JSON documents:

```json
{ "op": "add", "hostname": "app.test", "ip": "127.0.0.1", "domain_suffix": "test" }
```

Supported operations:

- `list`
- `add`
- `remove`
- `clear`
- `replace_all`

Every mutating request that carries hostnames must also carry the active `domain_suffix`. The helper rejects hostnames outside that suffix before it touches the hosts file.

## Development Smoke

Use a temporary hosts file when testing without root:

```bash
cargo run --bin portbay-hosts-helper -- \
  --socket /tmp/portbay-hosts-helper.sock \
  --hosts-file /tmp/portbay-hosts
```

The CLI attempts the helper first and falls back to direct `HostsManager` writes when the helper is unavailable, preserving the Phase 1 `sudo portbay hosts ...` path.

## Production Install Notes

SMAppService registration requires a signed app and helper. The plist label is:

```text
com.portbay-app.portbay.hosts-helper
```

The production bundle must place the helper executable at:

```text
PortBay.app/Contents/MacOS/portbay-hosts-helper
```

and the plist at:

```text
PortBay.app/Contents/Library/LaunchDaemons/com.portbay-app.portbay.hosts-helper.plist
```

The final registration call belongs in the signed macOS app layer:

```swift
try SMAppService.daemon(plistName: "com.portbay-app.portbay.hosts-helper.plist").register()
```

The Developer ID card owns the certificate and notarization prerequisites. Without that certificate, macOS will not provide the real production approval path.
