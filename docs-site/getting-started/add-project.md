---
title: Add a Project to PortBay — GUI & CLI Walkthrough
description: Register a local folder as a PortBay project via the GUI wizard or CLI, configure its hostname, port, type, and environment variables, then start it with one click.
---

# Add A Project

Projects are registered from local folders. PortBay records the launch command, port, hostname, HTTPS setting, optional services, and readiness probe in the registry.

![PortBay projects list](/screenshots/projects.png)

## Supported Project Types

| Type | Typical command | Notes |
| --- | --- | --- |
| Next | `pnpm dev` | Usually binds to an HTTP port and uses an HTTP readiness probe. |
| Vite | `pnpm dev --host 127.0.0.1` | Make sure the dev server binds to the port PortBay routes. |
| PHP | Service-backed | Uses PHP-FPM and an optional document root such as `public`. |
| Static | None | Served by Caddy where supported. |
| Node | `npm run dev` | Use a concrete port and readiness policy. |
| Custom | Any shell command | Best for frameworks PortBay does not infer yet. |

## Add From The App

1. Click **Add project**.
2. Pick or paste the project folder path.
3. Confirm the detected type, hostname, and port.
4. Add environment variables if the dev server needs them.
5. Open the raw config step only when the generated registry record needs hand editing.
6. Save, then start the project from its row action.

## Add From The CLI

```bash
portbay add ~/Projects/marketing-site \
  --id marketing-site \
  --name "Marketing Site" \
  --kind next \
  --port 3010 \
  --start-command "pnpm dev" \
  --hostname marketing-site.test
```

## After Save

- Start the project.
- Open the browser action for the generated `https://<hostname>` URL.
- Open logs if the row stays in `starting`, `unhealthy`, or `crashed`.
- Stop the project before changing its port or launch command.
