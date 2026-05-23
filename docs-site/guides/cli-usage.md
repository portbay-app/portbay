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

## Shell Completions

Generate completion scripts from the installed CLI:

```bash
portbay completions zsh > ~/.zsh/completions/_portbay
portbay completions bash > ~/.local/share/bash-completion/completions/portbay
portbay completions fish > ~/.config/fish/completions/portbay.fish
```

The generated scripts include dynamic project-id hooks:

```bash
portbay --complete-projects
portbay --complete-running-projects
```

`start`, `restart`, `open`, `logs`, and `remove` complete registered project ids. `stop` can use the running-project helper in shell integrations that support custom dynamic sources.

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
