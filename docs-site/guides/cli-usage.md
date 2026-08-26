---
title: PortBay CLI Usage — Terminal Parity with the App
description: Use the portbay CLI to list, start, stop, and inspect local projects from your terminal — same Rust core as the GUI, with JSON output and registry override flags.
---

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

## Shell Completions

The CLI generates its own completion scripts, so they always match the
subcommands and flags your build actually has:

```bash
portbay completions bash
portbay completions zsh
portbay completions fish
```

Every release CLI archive (`portbay-cli-<version>-<target>.tar.gz`) also ships
them pre-generated in a `completions/` directory next to the binary, so you can
install them without running the CLI first.

Install them where your shell looks for completions:

```bash
# bash — needs bash-completion installed
portbay completions bash > "$(brew --prefix)/etc/bash_completion.d/portbay"

# zsh — anywhere on your $fpath, as a file named _portbay
portbay completions zsh > ~/.zfunc/_portbay   # then: fpath=(~/.zfunc $fpath); compinit

# fish
portbay completions fish > ~/.config/fish/completions/portbay.fish
```

Regenerate after upgrading PortBay — a completion script from an older build
will not offer newer subcommands.

Building from source? `scripts/generate-completions.sh` writes all three at once
(`--bin` to point at a specific binary, `--out` for the target directory). It
refuses to write anything if the CLI is missing or emits nothing, because an
empty completion file installs successfully and completes nothing.
