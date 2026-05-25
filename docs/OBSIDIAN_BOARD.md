# PortBay Full-Stack Parity Board

## Now

- [x] Make runtime selection project-specific across supported languages.
  - [x] Detect `.nvmrc`, `.node-version`, `.python-version`, `.php-version`,
    `.tool-versions`, `mise.toml`, package metadata, and framework files.
  - [x] Run PortBay-supervised projects with the pinned runtime first on `PATH`.
  - [x] Write standard runtime marker files so shells switch when you `cd` into a
    project.
- [x] Generalize web server selection as a project concern, not a PHP-only UI
  concern.
  - [x] Keep Caddy as the public hostname/TLS edge.
  - [x] Allow project-level Caddy, Nginx, and Apache modes where relevant.
- [x] Finish database parity.
  - [x] MySQL, MariaDB, PostgreSQL, MongoDB, Redis are required.
  - [x] Memcached is included for PHP ecosystem parity.
  - [x] Verify create/start/stop/link/client metadata coverage for every engine.
- [x] Verify zero-config DNS and trusted local SSL.
  - [x] Bundled dnsmasq for wildcard project domains.
  - [x] mkcert CA install + per-project cert issuance.
  - [x] Avoid per-project `/etc/hosts` editing once resolver is installed.

## Next

- [x] Add Flutter runtime detection and project detection.
  - [x] Detect Flutter SDK from PATH/Homebrew/FVM/asdf/mise where possible.
  - [x] Detect `pubspec.yaml` Flutter apps.
  - [x] Run `flutter run` through PortBay with device selection.
- [x] Add mobile project types.
  - [x] Xcode: detect `.xcodeproj` / `.xcworkspace` folders.
  - [x] Android: detect Gradle Android projects.
  - [x] Xcode schemes/destinations and Android variants/devices/emulators.
  - [x] Pressing Play launches the appropriate mobile command instead of
    requiring manual paths.
- [x] Add mobile run configuration UI.
  - [x] Scheme/flavor/variant/device selectors.
  - [x] Logs surfaced in the same project log viewer.
  - [x] Stop uses the shared supervised process lifecycle.

## Later

- [ ] Add richer per-project language configuration.
  - Project-level package manager choice.
  - Project-level environment templates.
  - Optional managed install prompts for missing runtime versions.
- [ ] Add importers for more local dev tools.
  - FlyEnv project/site import.
  - Xcode/Android recent-project discovery.
  - Flutter workspace discovery.
