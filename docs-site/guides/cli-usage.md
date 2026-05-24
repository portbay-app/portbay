# CLI Usage

The `portbay` CLI is a thin command-line interface over the same Rust core as the Tauri app.

## Connection Model

The CLI expects a PortBay daemon to be running. In the current build, that means the Tauri app should be open. The CLI reads the registry and talks to Process Compose through the discovered runtime port.

## Common Tasks

```bash
portbay list
portbay status
portbay status marketing-site
portbay start marketing-site
portbay logs marketing-site --limit 100
portbay open marketing-site
portbay stop marketing-site
portbay stop --all
```

## JSON Output

Use `--json` for machine-readable output:

```bash
portbay --json list
portbay --json status marketing-site
```

## Registry Override

Use `--registry` when testing against an isolated registry file:

```bash
portbay --registry /tmp/portbay-registry.json list
```

## Process Compose Port Override

Use `--pc-port` only when testing a non-standard daemon port:

```bash
portbay --pc-port 7432 status
```
