# Capabilities

PortBay is a Tauri 2 app. The main window receives only the capabilities it needs for local development control.

## Main Window

| Capability | Why it exists |
| --- | --- |
| `core:default` | Basic Tauri window and app behavior. |
| `core:window:allow-start-dragging` | Native draggable window regions. |
| `shell:allow-execute` | Run approved commands through the shell plugin. |
| `shell:allow-spawn` | Spawn approved bundled sidecars. |
| `opener:default` | Open project URLs and external targets. |
| `dialog:default` | Pick local folders and files. |

## Allowed Sidecars

| Sidecar | Role |
| --- | --- |
| `process-compose` | Starts and supervises project processes. |
| `caddy` | Reverse proxy and HTTPS termination. |
| `mkcert` | Local certificate issuance. |
| `mailpit` | Local mail capture. |
| `cloudflared` | Public tunnel support. |

The app does not grant arbitrary sidecar spawn permission. Each executable is named in the capability file and expected under the Tauri sidecar layout.
