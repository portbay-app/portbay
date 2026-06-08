/**
 * Canonical demo roster for the PortBay simulator, screenshot pipeline, and
 * e2e harness — ONE source of truth so every surface tells the same story.
 *
 * ⛔ DUMMY DATA ONLY. Every value here is fictional: invented project names,
 * `.test` domains, generic `/Users/dev/Sites/...` paths, and an example Pro
 * account. Never a real project, path, domain, or account. A screenshot or
 * simulator showing anything else does not ship.
 *
 * Typed against the real wire interfaces (`$lib/types/*`) so the fixtures stay
 * in lockstep with the UI. Type-only imports are erased at build time, so this
 * module is safe to import from both the SvelteKit app and the Playwright
 * harness (which doesn't resolve the `$lib` alias for value imports).
 */
import type { ProjectView, RuntimeInfo } from "$lib/types/projects";
import type { GroupView } from "$lib/types/groups";
import type { SidecarHealth } from "$lib/types/sidecars";
import type { EffectiveEntitlement } from "$lib/types/entitlements";
import type { RequestEntry } from "$lib/types/inspector";
import type { LanguageView } from "$lib/types/runtimes";
import type {
  DatabaseEngineView,
  DatabaseInstanceView,
} from "$lib/types/databases";
import type { DnsPreflight, DnsRecord, ResolverStatus } from "$lib/types/dns";
import type { WebServerInfo } from "$lib/types/webservers";
import type { SystemMetrics } from "$lib/types/metrics";
import type { DevToolInfo } from "$lib/types/devTools";
import type { ProbeResult, SshConnectionView } from "$lib/types/sshConnections";
import type { SshIdentityView } from "$lib/types/sshIdentities";
import type { SftpEntry, SshTunnelRuntimeStatus } from "$lib/types/sshTunnels";
import type { DetectedTunnel, TunnelStatus } from "$lib/types/tunnel";
import type { AgentInfo } from "$lib/ssh/agent";

/** Everything the mock IPC layer can serve, in one bag. */
export interface DemoFixtures {
  projects: ProjectView[];
  groups: GroupView[];
  sidecars: SidecarHealth;
  entitlement: EffectiveEntitlement;
  requests: RequestEntry[];
  runtimes: LanguageView[];
  databaseEngines: DatabaseEngineView[];
  databaseInstances: DatabaseInstanceView[];
  dnsRecords: DnsRecord[];
  dnsPreflight: DnsPreflight;
  resolverStatus: ResolverStatus;
  webServers: WebServerInfo[];
  metrics: SystemMetrics;
  devTools: DevToolInfo[];
  logs: Record<string, string[]>;
  /** Saved SSH/remote connections (the host table). */
  sshConnections: SshConnectionView[];
  /** Reusable SSH identities (key/agent/password presets). */
  sshIdentities: SshIdentityView[];
  /** Saved port-forwards / tunnels with their live runtime state. */
  sshTunnels: SshTunnelRuntimeStatus[];
  /** Reachability + host-trust per connection id (the Health column). */
  sshProbes: Record<string, ProbeResult>;
  /** Per-connection behavioural data the workspace serves: home dir, snapshot
   *  + ps + ss output, an SFTP tree with file contents, and the on-host agent. */
  sshHosts: Record<string, DemoSshHost>;
  /** Active Cloudflare quick tunnels (public sharing). */
  cfTunnels: TunnelStatus[];
  /** Named Cloudflare tunnels detected under ~/.cloudflared. */
  cfNamedTunnels: DetectedTunnel[];
}

/** Everything the SSH workspace needs to render one host without a real server. */
export interface DemoSshHost {
  /** Absolute home directory `sftp_connect` / `sftp_home_dir` return. */
  homeDir: string;
  /** Marker-delimited stdout for the host-snapshot command. */
  snapshotStdout: string;
  /** `ps aux` stdout (header + rows) for the Processes panel. */
  psStdout: string;
  /** `ss -tlnpH` stdout for the Ports panel. */
  portsStdout: string;
  /** SFTP directory listings keyed by absolute path. */
  sftp: Record<string, SftpEntry[]>;
  /** Readable file contents keyed by absolute path (the editor + preview). */
  files: Record<string, string>;
  /** What model tooling the on-host AI agent panel reports. */
  agent: AgentInfo;
}

/** A healthy-looking process snapshot for a running project. */
function runtimeOf(pid: number, memMb: number, cpu: number): RuntimeInfo {
  return {
    pid,
    restarts: 0,
    isReady: "true",
    hasReadyProbe: true,
    exitCode: 0,
    age: 42 * 60 * 1_000_000_000, // 42 min, in nanoseconds (PC's unit)
    memBytes: memMb * 1024 * 1024,
    cpuPercent: cpu,
  };
}

const PROJECTS: ProjectView[] = [
  {
    id: "acme-storefront",
    name: "Acme Storefront",
    path: "/Users/dev/Sites/acme-storefront",
    type: "next",
    startCommand: "pnpm dev",
    port: 3000,
    extraPorts: [],
    hostname: "acme-storefront.test",
    url: "https://acme-storefront.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: { NODE_ENV: "development" },
    autoStart: true,
    tags: ["storefront", "next"],
    sandboxed: false,
    status: "running",
    runtime: runtimeOf(48211, 312, 1.8),
  },
  {
    id: "billing-api",
    name: "Billing API",
    path: "/Users/dev/Sites/billing-api",
    type: "php",
    port: 8080,
    extraPorts: [],
    hostname: "billing-api.test",
    url: "https://billing-api.test",
    https: true,
    preStart: [],
    postStart: [],
    services: ["mysql"],
    env: { APP_ENV: "local" },
    autoStart: false,
    tags: ["laravel", "api"],
    documentRoot: "public",
    phpVersion: "8.3",
    webServer: "nginx",
    sandboxed: false,
    status: "running",
    runtime: runtimeOf(48377, 96, 0.4),
  },
  {
    id: "pulsar-api",
    name: "Pulsar API",
    path: "/Users/dev/Sites/pulsar-api",
    type: "python",
    startCommand: "uvicorn app.main:app --reload --port 8000",
    port: 8000,
    extraPorts: [],
    hostname: "pulsar-api.test",
    url: "https://pulsar-api.test",
    https: true,
    preStart: [],
    postStart: [],
    services: ["postgres"],
    env: { APP_ENV: "local", PYTHONUNBUFFERED: "1" },
    autoStart: false,
    tags: ["python", "fastapi"],
    sandboxed: false,
    status: "running",
    runtime: runtimeOf(48533, 134, 1.2),
  },
  {
    id: "dashboard-ui",
    name: "Dashboard UI",
    path: "/Users/dev/Sites/dashboard-ui",
    type: "vite",
    startCommand: "pnpm dev",
    port: 5173,
    extraPorts: [],
    hostname: "dashboard.test",
    url: "https://dashboard.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: {},
    autoStart: false,
    tags: ["svelte", "vite"],
    sandboxed: false,
    status: "stopped",
  },
  {
    id: "marketing-site",
    name: "Marketing Site",
    path: "/Users/dev/Sites/marketing-site",
    type: "static",
    extraPorts: [],
    hostname: "marketing.test",
    url: "https://marketing.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: {},
    autoStart: true,
    tags: ["static"],
    sandboxed: false,
    domain: {
      notes: "Marketing + per-campaign landing pages.",
      pathPrefix: null,
      resolverMode: "auto",
      autoManageCert: true,
      sslMode: "automatic_local",
      includeWildcardSubdomains: true,
      exposeWhenRunning: false,
    },
    status: "running",
    runtime: runtimeOf(48402, 18, 0.0),
  },
  {
    id: "legacy-importer",
    name: "Legacy Importer",
    path: "/Users/dev/Sites/legacy-importer",
    type: "node",
    startCommand: "node server.js",
    port: 4000,
    extraPorts: [],
    hostname: "legacy.test",
    url: "https://legacy.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: {},
    autoStart: false,
    tags: ["node", "legacy"],
    sandboxed: false,
    status: "unhealthy",
    runtime: runtimeOf(48455, 142, 7.3),
  },
  {
    id: "untrusted-demo",
    name: "Untrusted Demo",
    path: "/Users/dev/Sites/untrusted-demo",
    type: "node",
    startCommand: "npm start",
    port: 6060,
    extraPorts: [],
    hostname: "sandbox.test",
    url: "https://sandbox.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: {},
    autoStart: false,
    tags: ["sandbox", "imported"],
    sandboxed: true,
    sandbox: { enabled: true, network: "loopback_only", ephemeral: true },
    status: "running",
    runtime: runtimeOf(48511, 64, 0.9),
  },
  {
    id: "vendor-portal",
    name: "Vendor Portal (eval)",
    path: "/Users/dev/Sites/vendor-portal",
    type: "node",
    startCommand: "npm start",
    port: 3000,
    extraPorts: [8000],
    hostname: "vendor-portal.test",
    url: "https://vendor-portal.test",
    https: true,
    preStart: [],
    postStart: [],
    services: ["web", "worker", "postgres"],
    env: {
      NODE_ENV: "production",
      DATABASE_URL: "postgres://app:s3cr3t@127.0.0.1:5432/vendor",
      REDIS_URL: "redis://127.0.0.1:6379",
      OCR_LANGUAGES: "eng+deu",
    },
    autoStart: false,
    tags: ["sandbox", "eval", "node"],
    sandboxed: true,
    sandbox: { enabled: true, network: "outbound", ephemeral: false },
    status: "running",
    runtime: runtimeOf(48533, 188, 1.2),
  },
  {
    id: "hatchway-cms",
    name: "Hatchway CMS",
    path: "/Users/dev/Sites/hatchway-cms",
    type: "next",
    startCommand: "pnpm dev",
    port: 3100,
    extraPorts: [],
    hostname: "hatchway.test",
    url: "https://hatchway.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: { NODE_ENV: "development" },
    autoStart: false,
    tags: ["cms", "next"],
    sandboxed: false,
    domain: {
      notes: "Headless CMS admin + preview.",
      pathPrefix: null,
      resolverMode: "auto",
      autoManageCert: true,
      sslMode: "automatic_local",
      includeWildcardSubdomains: false,
      exposeWhenRunning: false,
    },
    status: "running",
    runtime: runtimeOf(48560, 204, 1.1),
  },
  {
    id: "pinpoint-maps",
    name: "Pinpoint Maps",
    path: "/Users/dev/Sites/pinpoint-maps",
    type: "vite",
    startCommand: "pnpm dev",
    port: 5180,
    extraPorts: [],
    hostname: "pinpoint.test",
    url: "https://pinpoint.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: {},
    autoStart: false,
    tags: ["vite", "maps"],
    sandboxed: false,
    status: "running",
    runtime: runtimeOf(48574, 96, 0.5),
  },
  {
    id: "quill-docs",
    name: "Quill Docs",
    path: "/Users/dev/Sites/quill-docs",
    type: "static",
    extraPorts: [],
    hostname: "docs.quill.test",
    url: "https://docs.quill.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: {},
    autoStart: true,
    tags: ["docs", "static"],
    sandboxed: false,
    domain: {
      notes: "Versioned docs; each release gets its own subdomain.",
      pathPrefix: null,
      resolverMode: "dnsmasq",
      autoManageCert: true,
      sslMode: "automatic_local",
      includeWildcardSubdomains: true,
      exposeWhenRunning: false,
    },
    status: "running",
    runtime: runtimeOf(48588, 14, 0.0),
  },
  {
    id: "relay-api",
    name: "Relay API",
    path: "/Users/dev/Sites/relay-api",
    type: "node",
    startCommand: "node server.js",
    port: 4200,
    extraPorts: [],
    hostname: "api.relay.test",
    url: "https://api.relay.test",
    https: true,
    preStart: [],
    postStart: [],
    services: ["postgres"],
    env: { NODE_ENV: "development" },
    autoStart: false,
    tags: ["api", "node"],
    sandboxed: false,
    domain: {
      notes: "Public API gateway, mounted under /v1.",
      pathPrefix: "/v1",
      resolverMode: "auto",
      autoManageCert: true,
      sslMode: "automatic_local",
      includeWildcardSubdomains: false,
      exposeWhenRunning: true,
    },
    status: "running",
    runtime: runtimeOf(48601, 132, 0.8),
  },
  {
    id: "cobalt-admin",
    name: "Cobalt Admin",
    path: "/Users/dev/Sites/cobalt-admin",
    type: "next",
    startCommand: "pnpm dev",
    port: 3200,
    extraPorts: [],
    hostname: "cobalt-admin.test",
    url: "https://cobalt-admin.test",
    https: true,
    preStart: [],
    postStart: [],
    services: [],
    env: {},
    autoStart: false,
    tags: ["admin", "next"],
    sandboxed: false,
    domain: {
      notes: "Internal admin — only routed while running.",
      pathPrefix: null,
      resolverMode: "auto",
      autoManageCert: true,
      sslMode: "automatic_local",
      includeWildcardSubdomains: false,
      exposeWhenRunning: true,
    },
    status: "running",
    runtime: runtimeOf(48615, 168, 0.9),
  },
  {
    id: "bookslash",
    name: "BookSlash",
    path: "/Users/dev/Sites/bookslash",
    type: "next",
    startCommand: "pnpm dev",
    port: 3300,
    extraPorts: [],
    hostname: "bookslash.test",
    url: "https://bookslash.test",
    https: true,
    preStart: [],
    postStart: [],
    services: ["postgres"],
    env: { NODE_ENV: "development" },
    autoStart: true,
    tags: ["app", "next"],
    sandboxed: false,
    status: "running",
    runtime: runtimeOf(48630, 256, 1.5),
  },
];

const GROUPS: GroupView[] = [
  {
    id: "acme-platform",
    name: "Acme Platform",
    projectIds: ["acme-storefront", "billing-api", "dashboard-ui"],
    knownIds: ["acme-storefront", "billing-api", "dashboard-ui"],
    memberCount: 3,
  },
];

const SIDECARS: SidecarHealth = {
  processCompose: { name: "process-compose", status: "running" },
  caddy: { name: "caddy", status: "running" },
  mkcertCa: { name: "mkcert-ca", status: "running", detail: "CA installed" },
  dnsmasq: { name: "dnsmasq", status: "running", detail: "*.test → 127.0.0.1" },
  mailpit: { name: "mailpit", status: "running", detail: "SMTP :1025 · UI :8025" },
  hostsHelper: { name: "hosts-helper", status: "running" },
};

const ENTITLEMENT: EffectiveEntitlement = {
  state: "pro",
  tier: "pro",
  source: "subscription",
  account: { github_id: 24601, login: "acme-dev" },
  entitlements: {
    max_projects: null,
    sync: true,
    custom_port_cors: true,
    mail: "full",
    early_access: true,
    priority_support: true,
  },
};

const now = Date.UTC(2026, 4, 25, 16, 30, 0); // fixed clock for deterministic shots
const REQUESTS: RequestEntry[] = [
  {
    ts: now - 1_200,
    method: "GET",
    host: "acme-storefront.test",
    uri: "/",
    status: 200,
    durationMs: 42,
    size: 18_443,
    projectId: "acme-storefront",
    reqHeaders: {
      accept: ["text/html"],
      "user-agent": ["Mozilla/5.0 (Macintosh)"],
    },
  },
  {
    ts: now - 3_400,
    method: "GET",
    host: "acme-storefront.test",
    uri: "/_next/static/chunks/main.js",
    status: 304,
    durationMs: 3,
    size: 0,
    projectId: "acme-storefront",
  },
  {
    ts: now - 5_900,
    method: "POST",
    host: "billing-api.test",
    uri: "/api/invoices",
    status: 201,
    durationMs: 88,
    size: 512,
    projectId: "billing-api",
    reqHeaders: { "content-type": ["application/json"] },
  },
  {
    ts: now - 8_100,
    method: "GET",
    host: "billing-api.test",
    uri: "/api/invoices/9999",
    status: 404,
    durationMs: 11,
    size: 74,
    projectId: "billing-api",
  },
  {
    ts: now - 12_500,
    method: "GET",
    host: "legacy.test",
    uri: "/import/run",
    status: 500,
    durationMs: 1_204,
    size: 281,
    projectId: "legacy-importer",
  },
];

const RUNTIMES: LanguageView[] = [
  {
    id: "node",
    displayName: "Node.js",
    installHint: "Install via Homebrew, nvm, or mise.",
    defaultVersion: "20.11.1",
    versions: [
      {
        install: {
          version: "20.11.1",
          binary: "/opt/homebrew/bin/node",
          source: "homebrew",
        },
        tabs: [
          {
            id: "overview",
            label: "Overview",
            rows: [
              {
                key: "binary",
                label: "Binary",
                value: "/opt/homebrew/bin/node",
                isPath: true,
                field: { kind: "readonly" },
              },
              {
                key: "npm",
                label: "npm",
                value: "10.2.4",
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "php",
    displayName: "PHP",
    installHint: "Install via Homebrew or ServBay.",
    defaultVersion: "8.3.6",
    versions: [
      {
        install: {
          version: "8.3.6",
          binary: "/opt/homebrew/opt/php@8.3/bin/php",
          source: "homebrew",
        },
        tabs: [
          {
            id: "overview",
            label: "Overview",
            rows: [
              {
                key: "binary",
                label: "Binary",
                value: "/opt/homebrew/opt/php@8.3/bin/php",
                isPath: true,
                field: { kind: "readonly" },
              },
              {
                key: "memory_limit",
                label: "memory_limit",
                value: "512M",
                field: { kind: "text" },
              },
            ],
            editable: true,
          },
        ],
      },
    ],
  },
  {
    id: "bun",
    displayName: "Bun",
    installHint: "brew install oven-sh/bun/bun",
    defaultVersion: "1.1.42",
    versions: [
      {
        install: {
          version: "1.1.42",
          binary: "/opt/homebrew/bin/bun",
          source: "homebrew",
        },
        tabs: [
          {
            id: "info",
            label: "Info",
            rows: [
              {
                key: "binary",
                label: "Binary",
                value: "/opt/homebrew/bin/bun",
                isPath: true,
                field: { kind: "readonly" },
              },
              {
                key: "source",
                label: "Source",
                value: "Homebrew",
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "python",
    displayName: "Python",
    installHint: "brew install python",
    defaultVersion: "3.13.1",
    versions: [
      {
        install: {
          version: "3.13.1",
          binary: "/opt/homebrew/opt/python@3.13/bin/python3.13",
          source: "homebrew",
        },
        tabs: [
          {
            id: "index",
            label: "Package index",
            editable: true,
            rows: [
              {
                key: "index-url",
                label: "Index URL",
                value: "",
                hint: "pip's package index (pip.conf [global] index-url). Blank uses the default PyPI (https://pypi.org/simple).",
                field: { kind: "text" },
              },
              {
                key: "config_file",
                label: "Config file",
                value: "/Users/dev/.config/pip/pip.conf",
                isPath: true,
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
      {
        install: {
          version: "3.12.7",
          binary: "/Users/dev/.pyenv/versions/3.12.7/bin/python",
          source: "pyenv",
        },
        tabs: [
          {
            id: "index",
            label: "Package index",
            editable: true,
            rows: [
              {
                key: "index-url",
                label: "Index URL",
                value: "https://pypi.org/simple",
                hint: "pip's package index (pip.conf [global] index-url). Blank uses the default PyPI (https://pypi.org/simple).",
                field: { kind: "text" },
              },
              {
                key: "config_file",
                label: "Config file",
                value: "/Users/dev/.config/pip/pip.conf",
                isPath: true,
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "flutter",
    displayName: "Flutter",
    installHint: "brew install --cask flutter",
    defaultVersion: "3.27.1",
    versions: [
      {
        install: {
          version: "3.27.1",
          binary: "/opt/homebrew/caskroom/flutter/bin/flutter",
          source: "homebrew",
        },
        tabs: [
          {
            id: "info",
            label: "Info",
            rows: [
              {
                key: "binary",
                label: "Binary",
                value: "/opt/homebrew/caskroom/flutter/bin/flutter",
                isPath: true,
                field: { kind: "readonly" },
              },
              {
                key: "source",
                label: "Source",
                value: "Homebrew",
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "go",
    displayName: "Go",
    installHint: "brew install go",
    defaultVersion: "1.23.4",
    versions: [
      {
        install: {
          version: "1.23.4",
          binary: "/opt/homebrew/bin/go",
          source: "homebrew",
        },
        tabs: [
          {
            id: "env",
            label: "Environment",
            editable: true,
            rows: [
              {
                key: "GOPROXY",
                label: "GOPROXY",
                value: "https://proxy.golang.org,direct",
                hint: "Module proxy. Blank uses Go's default (https://proxy.golang.org,direct). Accepts a comma list, `direct`, or `off`.",
                field: { kind: "text" },
              },
              {
                key: "GOPATH",
                label: "GOPATH",
                value: "/Users/dev/go",
                hint: "Workspace root. Blank uses Go's default (~/go).",
                field: { kind: "text" },
              },
              {
                key: "env_file",
                label: "Env file",
                value: "/Users/dev/Library/Application Support/go/env",
                isPath: true,
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "ruby",
    displayName: "Ruby",
    installHint: "brew install ruby",
    defaultVersion: "3.3.6",
    versions: [
      {
        install: {
          version: "3.3.6",
          binary: "/opt/homebrew/opt/ruby/bin/ruby",
          source: "homebrew",
        },
        tabs: [
          {
            id: "config",
            label: "RubyGems",
            editable: true,
            rows: [
              {
                key: "gem",
                label: "Default gem flags",
                value: "--no-document",
                hint: "Flags applied to every `gem` command via ~/.gemrc (e.g. --no-document). Blank removes the override.",
                field: { kind: "text" },
              },
              {
                key: "config_file",
                label: "Config file",
                value: "/Users/dev/.gemrc",
                isPath: true,
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
      {
        install: {
          version: "2.6.10",
          binary: "/usr/bin/ruby",
          source: "system",
        },
        tabs: [
          {
            id: "config",
            label: "RubyGems",
            editable: true,
            rows: [
              {
                key: "gem",
                label: "Default gem flags",
                value: "",
                hint: "Flags applied to every `gem` command via ~/.gemrc (e.g. --no-document). Blank removes the override.",
                field: { kind: "text" },
              },
              {
                key: "config_file",
                label: "Config file",
                value: "/Users/dev/.gemrc",
                isPath: true,
                field: { kind: "readonly" },
              },
            ],
          },
        ],
      },
    ],
  },
];

const DATABASE_ENGINES: DatabaseEngineView[] = [
  {
    id: "mysql",
    label: "MySQL",
    installed: true,
    version: "8.0.36",
    defaultPort: 3306,
    clientAvailable: true,
    installHint: "",
    managed: false,
    managedVersion: "",
  },
  {
    id: "mariadb",
    label: "MariaDB",
    installed: true,
    version: "11.4.2",
    defaultPort: 3306,
    clientAvailable: true,
    installHint: "",
    managed: false,
    managedVersion: "",
  },
  {
    id: "postgres",
    label: "PostgreSQL",
    installed: true,
    version: "16.2",
    defaultPort: 5432,
    clientAvailable: true,
    installHint: "",
    managed: false,
    managedVersion: "",
  },
  {
    id: "redis",
    label: "Redis",
    installed: true,
    version: "7.2.4",
    defaultPort: 6379,
    clientAvailable: true,
    installHint: "",
    managed: false,
    managedVersion: "",
  },
  {
    id: "mongo",
    label: "MongoDB",
    installed: true,
    version: "7.0.5",
    defaultPort: 27017,
    clientAvailable: true,
    installHint: "",
    managed: false,
    managedVersion: "",
  },
  {
    id: "memcached",
    label: "Memcached",
    installed: true,
    version: "1.6.31",
    defaultPort: 11211,
    clientAvailable: false,
    installHint: "",
    managed: false,
    managedVersion: "",
  },
];

const DATABASE_INSTANCES: DatabaseInstanceView[] = [
  {
    id: "acme-mysql",
    name: "acme-mysql",
    engine: "mysql",
    engineLabel: "MySQL",
    version: "8.0.36",
    port: 3306,
    status: "running",
    autoStart: true,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/acme-mysql",
    socketPath: "/tmp/portbay-acme-mysql.sock",
    connectionUrl: "mysql://root@127.0.0.1:3306",
    account: "root",
    linkedProjects: ["billing-api"],
    binaryAvailable: true,
    provisioned: true,
    fileBased: false,
  },
  {
    id: "acme-redis",
    name: "acme-redis",
    engine: "redis",
    engineLabel: "Redis",
    version: "7.2.4",
    port: 6379,
    status: "running",
    autoStart: true,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/acme-redis",
    connectionUrl: "redis://127.0.0.1:6379",
    account: "",
    linkedProjects: [],
    binaryAvailable: true,
    provisioned: true,
    fileBased: false,
  },
  {
    id: "relay-mariadb",
    name: "relay-mariadb",
    engine: "mariadb",
    engineLabel: "MariaDB",
    version: "11.4.2",
    port: 3307,
    status: "running",
    autoStart: true,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/relay-mariadb",
    socketPath: "/tmp/portbay-relay-mariadb.sock",
    connectionUrl: "mysql://root@127.0.0.1:3307/",
    account: "root",
    linkedProjects: ["relay-api"],
    binaryAvailable: true,
    provisioned: true,
    fileBased: false,
  },
  {
    id: "hatchway-pg",
    name: "hatchway-pg",
    engine: "postgres",
    engineLabel: "PostgreSQL",
    version: "16.2",
    port: 5432,
    status: "running",
    autoStart: true,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/hatchway-pg",
    socketPath: "/tmp/.s.PGSQL.5432",
    connectionUrl: "postgresql://postgres@127.0.0.1:5432/postgres",
    account: "postgres",
    linkedProjects: ["hatchway-cms", "quill-docs"],
    binaryAvailable: true,
    provisioned: true,
    fileBased: false,
  },
  {
    id: "pinpoint-mongo",
    name: "pinpoint-mongo",
    engine: "mongo",
    engineLabel: "MongoDB",
    version: "7.0.5",
    port: 27017,
    status: "running",
    autoStart: false,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/pinpoint-mongo",
    connectionUrl: "mongodb://127.0.0.1:27017",
    account: "",
    linkedProjects: ["pinpoint-maps"],
    binaryAvailable: true,
    provisioned: true,
    fileBased: false,
  },
  {
    id: "cobalt-cache",
    name: "cobalt-cache",
    engine: "memcached",
    engineLabel: "Memcached",
    version: "1.6.31",
    port: 11211,
    status: "stopped",
    autoStart: false,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/cobalt-cache",
    connectionUrl: "memcached://127.0.0.1:11211",
    account: "",
    linkedProjects: ["cobalt-admin"],
    binaryAvailable: true,
    provisioned: true,
    fileBased: false,
  },
  {
    id: "quill-sqlite",
    name: "quill-sqlite",
    engine: "sqlite",
    engineLabel: "SQLite",
    version: "3.43.2",
    port: 0,
    status: "running",
    autoStart: false,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/quill-sqlite/data",
    filePath:
      "/Users/dev/Library/Application Support/PortBay/db/quill-sqlite/data/database.sqlite",
    connectionUrl:
      "sqlite:///Users/dev/Library/Application Support/PortBay/db/quill-sqlite/data/database.sqlite",
    account: "",
    linkedProjects: ["quill-docs"],
    binaryAvailable: true,
    provisioned: true,
    fileBased: true,
  },
  {
    id: "bookslash-pg",
    name: "bookslash-pg",
    engine: "postgres",
    engineLabel: "PostgreSQL",
    version: "16.2",
    port: 5433,
    status: "running",
    autoStart: true,
    dataDir: "/Users/dev/Library/Application Support/PortBay/db/bookslash-pg",
    socketPath: "/tmp/.s.PGSQL.5433",
    connectionUrl: "postgresql://postgres@127.0.0.1:5433/postgres",
    account: "postgres",
    linkedProjects: ["bookslash"],
    binaryAvailable: true,
    provisioned: true,
    fileBased: false,
  },
];

const DNS_RECORDS: DnsRecord[] = [
  {
    hostname: "*.test",
    target: "127.0.0.1",
    kind: "wildcard",
    projectId: null,
    projectName: null,
    routedVia: "dnsmasq",
  },
  ...PROJECTS.map<DnsRecord>((p) => ({
    hostname: p.hostname,
    target: "127.0.0.1",
    kind: "project",
    projectId: p.id,
    projectName: p.name,
    routedVia: "dnsmasq",
  })),
];

const DNS_PREFLIGHT: DnsPreflight = {
  suffix: "test",
  dnsmasqPort: 53531,
  helperInstalled: true,
  hostsActive: true,
  resolverInstalled: true,
  dnsmasqRunning: true,
  port80InUse: false,
  port443InUse: false,
  ready: true,
};

const RESOLVER_STATUS: ResolverStatus = {
  suffix: "test",
  installed: true,
  path: "/etc/resolver/test",
  currentContents: "nameserver 127.0.0.1\nport 53531",
  currentPort: 53531,
};

/**
 * Web servers, as the `/web-servers` page sees them. Caddy is the bundled edge;
 * Nginx is "installed" and serves the one PHP project (Billing API); Apache is
 * detected-as-absent to show the not-installed state.
 */
const WEB_SERVERS: WebServerInfo[] = [
  {
    id: "caddy",
    name: "Caddy",
    role: "Edge router — maps your project hostnames to their ports, terminates local HTTPS, and reverse-proxies to Nginx/Apache when a project picks them.",
    edge: true,
    bundled: true,
    installed: true,
    binaryPath: null,
    version: null,
    projects: [],
    isDefault: true,
  },
  {
    id: "nginx",
    name: "Nginx",
    role: "Per-project PHP backend. PortBay generates the nginx.conf (FastCGI to PHP-FPM) and Caddy reverse-proxies the hostname to it.",
    edge: false,
    bundled: false,
    installed: true,
    binaryPath: "/opt/homebrew/bin/nginx",
    version: "1.27.0",
    projects: [{ id: "billing-api", name: "Billing API" }],
    isDefault: false,
  },
  {
    id: "apache",
    name: "Apache",
    role: "Per-project PHP backend. PortBay generates the httpd.conf (mod_proxy_fcgi to PHP-FPM) and Caddy reverse-proxies the hostname to it.",
    edge: false,
    bundled: false,
    installed: false,
    binaryPath: null,
    version: null,
    projects: [],
    isDefault: false,
  },
];

const SYSTEM_METRICS: SystemMetrics = {
  cpu: { total: 18 },
  memory: {
    usedBytes: 11.4 * 1024 ** 3,
    totalBytes: 32 * 1024 ** 3,
  },
  disk: {
    usedBytes: 356 * 1024 ** 3,
    totalBytes: 994 * 1024 ** 3,
  },
};

const DEV_TOOLS: DevToolInfo[] = [
  { id: "vscode", label: "Visual Studio Code", kind: "editor" },
  { id: "cursor", label: "Cursor", kind: "editor" },
  { id: "phpstorm", label: "PhpStorm", kind: "editor" },
  { id: "codex", label: "Codex", kind: "agent" },
  { id: "warp", label: "Warp", kind: "terminal" },
  { id: "terminal", label: "Terminal", kind: "terminal" },
  { id: "finder", label: "Finder", kind: "file-manager" },
];

function log(process: string, message: string, level = "info"): string {
  return JSON.stringify({ level, process, replica: 0, message });
}

const LOGS: Record<string, string[]> = {
  "acme-storefront": [
    log("web", "> acme-storefront@0.8.0 dev"),
    log("web", "> next dev --turbo"),
    log("web", "▲ Next.js 15.3.1"),
    log("web", "Local:        https://acme-storefront.test"),
    log("web", "Network:      http://192.168.1.195:3000"),
    log("web", "✓ Starting..."),
    log("web", "✓ Ready in 812ms"),
    log("web", "GET / 200 in 42ms"),
    log("web", "GET /_next/static/chunks/main.js 304 in 3ms"),
    log("web", "Compiled /products/[slug] in 184ms"),
    log("web", "GET /products/lighthouse-tee 200 in 57ms"),
  ],
  "billing-api": [
    log("php-fpm", "PHP 8.3.6 Development Server started"),
    log("nginx", "nginx/1.27.0 started; upstream php-fpm listening on 127.0.0.1:9074"),
    log("laravel", "INFO  Route cache loaded"),
    log("laravel", "INFO  GET /health 200 9ms"),
    log("laravel", "INFO  POST /api/invoices 201 88ms"),
    log("laravel", "WARN  Slow query detected: invoices.index took 421ms", "warn"),
    log("laravel", "INFO  Queue worker processing SendInvoiceEmail"),
  ],
  "dashboard-ui": [
    log("web", "> dashboard-ui@0.4.0 dev"),
    log("web", "> vite --host 127.0.0.1"),
    log("web", "VITE v6.4.2  ready in 318 ms"),
    log("web", "➜  Local:   https://dashboard.test"),
    log("web", "hmr update /src/routes/+page.svelte"),
  ],
  "marketing-site": [
    log("static", "Serving /Users/dev/Sites/marketing-site on https://marketing.test"),
    log("caddy", "certificate loaded for marketing.test"),
    log("caddy", "GET / 200 18ms"),
    log("caddy", "GET /assets/hero.webp 200 24ms"),
  ],
  "legacy-importer": [
    log("node", "> node server.js"),
    log("node", "Legacy Importer listening on http://127.0.0.1:4000"),
    log("node", "WARN  Deprecated API token format detected", "warn"),
    log("node", "GET /import/run 500 1204ms", "error"),
    log("node", "ERROR Migration step failed: missing customer_id on row 1842", "error"),
  ],
  "untrusted-demo": [
    log("sandbox", "Sandbox profile applied: loopback_only, ephemeral filesystem"),
    log("node", "> npm start"),
    log("node", "Demo server listening on https://sandbox.test"),
    log("sandbox", "Blocked outbound network attempt to api.example.com:443", "warn"),
    log("node", "GET / 200 31ms"),
  ],
};

// ── SSH / Cloudflare fixture helpers ─────────────────────────────────────────

const DAY_MS = 86_400_000;
const NOW = Date.now();


// ── SSH / remote connections (demo) ──────────────────────────────────────────
// A small fictional fleet for "Acme". Hostnames use the RFC-2606 `.example`
// reserved TLD; every key path, account, fingerprint, and IP is invented.
// ⛔ DUMMY DATA ONLY — never a real host, key, or credential.

const SSH_SECS = Math.floor(NOW / 1000);
const sshAgoSecs = (mins: number): number => SSH_SECS - mins * 60;
const sshAgoMs = (mins: number): number => NOW - mins * 60_000;

function sftpFile(
  dir: string,
  name: string,
  size: number,
  mode: number,
  daysAgo: number,
): SftpEntry {
  return {
    name,
    path: `${dir}/${name}`.replace("//", "/"),
    isDir: false,
    isSymlink: false,
    size,
    permissions: mode,
    mtimeSecs: Math.floor((NOW - daysAgo * DAY_MS) / 1000),
  };
}
function sftpDir(dir: string, name: string, daysAgo: number): SftpEntry {
  return {
    name,
    path: `${dir}/${name}`.replace("//", "/"),
    isDir: true,
    isSymlink: false,
    size: 4096,
    permissions: 0o755,
    mtimeSecs: Math.floor((NOW - daysAgo * DAY_MS) / 1000),
  };
}

/** Build the marker-delimited stdout the host-snapshot command parses. */
function snapshotOut(
  user: string,
  os: string,
  uptimeLine: string,
  memTotal: number,
  memUsed: number,
  dfLine: string,
): string {
  return [
    "###USER",
    user,
    "###OS",
    os,
    "###UP",
    uptimeLine,
    "###MEM",
    "              total        used        free      shared  buff/cache   available",
    `Mem:        ${memTotal}        ${memUsed}        ${Math.max(0, memTotal - memUsed - 1800)}         512        7454        ${memTotal - memUsed}`,
    "Swap:           0           0           0",
    "###DISK",
    "Filesystem      Size  Used Avail Use% Mounted on",
    dfLine,
  ].join("\n");
}

const NO_AGENT: AgentInfo = {
  hasCurl: true,
  hasWget: true,
  hasClaude: false,
  hasCodex: false,
  hasOllama: false,
  hasLlm: false,
  ollamaModels: [],
  port: 11434,
};

const SSH_CONNECTIONS: SshConnectionView[] = [
  {
    id: "acme-prod-web",
    name: "acme-prod-web",
    sshHost: "web1.acme.example",
    sshPort: 22,
    sshUser: "deploy",
    authKind: "key",
    keyPath: "~/.ssh/acme_deploy_ed25519",
    proxyJump: null,
    identityId: "id-acme-deploy",
    proxy: null,
    tags: ["production", "web"],
    color: "#e5484d",
    notes: "Primary web node behind the load balancer.",
    detectedOs: "Ubuntu 22.04.4 LTS",
    environment: "ubuntu",
    stage: "production",
    region: "us-east-1",
    provider: "AWS EC2",
    createdAt: sshAgoSecs(60 * 24 * 90),
    lastUsed: sshAgoSecs(18),
    tunnelCount: 1,
    inUse: false,
  },
  {
    id: "acme-staging",
    name: "acme-staging",
    sshHost: "stage.acme.example",
    sshPort: 22,
    sshUser: "deploy",
    authKind: "key",
    keyPath: "~/.ssh/acme_deploy_ed25519",
    proxyJump: null,
    identityId: "id-acme-deploy",
    proxy: null,
    tags: ["staging"],
    color: "#f5a623",
    notes: null,
    detectedOs: "Debian GNU/Linux 12 (bookworm)",
    environment: "ubuntu",
    stage: "staging",
    region: "us-east-1",
    provider: "AWS EC2",
    createdAt: sshAgoSecs(60 * 24 * 74),
    lastUsed: sshAgoSecs(180),
    tunnelCount: 1,
    inUse: false,
  },
  {
    id: "acme-db-primary",
    name: "acme-db-primary",
    sshHost: "db1.internal.acme.example",
    sshPort: 22,
    sshUser: "dbadmin",
    authKind: "key",
    keyPath: "~/.ssh/acme_deploy_ed25519",
    proxyJump: "bastion.acme.example",
    identityId: "id-acme-deploy",
    proxy: null,
    tags: ["database", "production"],
    color: "#3b82f6",
    notes: "Reachable only through the bastion. MySQL on 3306.",
    detectedOs: "Ubuntu 22.04.4 LTS",
    environment: "ubuntu",
    stage: "production",
    region: "us-east-1",
    provider: "AWS EC2",
    createdAt: sshAgoSecs(60 * 24 * 88),
    lastUsed: sshAgoSecs(42),
    tunnelCount: 1,
    inUse: true,
  },
  {
    id: "cpanel-shared",
    name: "cpanel-shared",
    sshHost: "premium42.webhostbox.example",
    sshPort: 2222,
    sshUser: "acmeco",
    authKind: "password",
    keyPath: null,
    proxyJump: null,
    identityId: null,
    proxy: null,
    tags: ["cpanel", "legacy"],
    color: null,
    notes: "Old marketing site on shared cPanel. Password auth only.",
    detectedOs: "CloudLinux 8.9",
    environment: "cpanel",
    stage: "production",
    region: null,
    provider: "Shared cPanel",
    createdAt: sshAgoSecs(60 * 24 * 210),
    lastUsed: sshAgoSecs(60 * 24 * 6),
    tunnelCount: 0,
    inUse: false,
  },
  {
    id: "edge-cache",
    name: "edge-cache",
    sshHost: "edge.acme.example",
    sshPort: 22,
    sshUser: "root",
    authKind: "agent",
    keyPath: null,
    proxyJump: null,
    identityId: "id-ops-agent",
    proxy: null,
    tags: ["cache", "edge"],
    color: "#22c55e",
    notes: null,
    detectedOs: "Alpine Linux 3.19",
    environment: "ubuntu",
    stage: "production",
    region: "eu-west-1",
    provider: "Hetzner Cloud",
    createdAt: sshAgoSecs(60 * 24 * 40),
    lastUsed: sshAgoSecs(60 * 5),
    tunnelCount: 1,
    inUse: false,
  },
  {
    id: "research-box",
    name: "research-box",
    sshHost: "gpu.lab.acme.example",
    sshPort: 22,
    sshUser: "researcher",
    authKind: "key",
    keyPath: "~/.ssh/id_ed25519",
    proxyJump: null,
    identityId: "id-personal",
    proxy: null,
    tags: ["research", "gpu"],
    color: "#a855f7",
    notes: "Ollama host — drives the on-box AI agent demo.",
    detectedOs: "Ubuntu 22.04.4 LTS (CUDA 12.4)",
    environment: "ubuntu",
    stage: "research",
    region: "us-west-2",
    provider: "Lambda Labs",
    createdAt: sshAgoSecs(60 * 24 * 21),
    lastUsed: sshAgoSecs(60 * 24 * 2),
    tunnelCount: 0,
    inUse: false,
  },
];

const SSH_IDENTITIES: SshIdentityView[] = [
  {
    id: "id-acme-deploy",
    name: "Acme Deploy Key",
    sshUser: "deploy",
    authKind: "key",
    keyPath: "~/.ssh/acme_deploy_ed25519",
    connectionCount: 3,
    inUse: true,
  },
  {
    id: "id-personal",
    name: "Personal ed25519",
    sshUser: "researcher",
    authKind: "key",
    keyPath: "~/.ssh/id_ed25519",
    connectionCount: 1,
    inUse: true,
  },
  {
    id: "id-ops-agent",
    name: "Ops (ssh-agent)",
    sshUser: "root",
    authKind: "agent",
    keyPath: null,
    connectionCount: 1,
    inUse: true,
  },
];

const SSH_PROBES: Record<string, ProbeResult> = {
  "acme-prod-web": {
    reachable: true,
    latencyMs: 24,
    health: "healthy",
    fingerprint: "SHA256:9q3Rk0m2Xc7pVz1bNf8sQwYtJ4Lh6Ua2Dg5Re0Ki3o",
    trust: "trusted",
  },
  "acme-staging": {
    reachable: true,
    latencyMs: 142,
    health: "degraded",
    fingerprint: "SHA256:2Hh7Tn5Pq8Lr1Wd4Yf0Zb6Mc3Vx9Ks2Ja5Ge8Ui1No",
    trust: "trusted",
  },
  "acme-db-primary": {
    reachable: true,
    latencyMs: 38,
    health: "healthy",
    fingerprint: "SHA256:5Kf2Lp9Qr3Wn7Xd1Yb8Zc4Mh6Vs0Ja2Ge5Ui8No1Tp",
    trust: "trusted",
  },
  "cpanel-shared": {
    reachable: true,
    latencyMs: 88,
    health: "healthy",
    fingerprint: "SHA256:7Np3Qr5Wf9Xd2Yb6Zc1Mh8Vs4Ja0Ge7Ui3No5Tp2Lk",
    trust: "trusted",
  },
  "edge-cache": {
    reachable: true,
    latencyMs: 11,
    health: "healthy",
    fingerprint: "SHA256:3Vx8Ja2Ge5Ui1No4Tp7Lk0Qr5Wf9Xd2Yb6Zc1Mh8Sa",
    trust: "trusted",
  },
  "research-box": {
    reachable: false,
    latencyMs: null,
    health: "down",
    fingerprint: null,
    trust: "trusted",
  },
};

const SSH_TUNNELS: SshTunnelRuntimeStatus[] = [
  {
    id: "tun-db-mysql",
    connectionId: "acme-db-primary",
    name: "Acme DB (MySQL)",
    sshHost: "db1.internal.acme.example",
    sshPort: 22,
    sshUser: "dbadmin",
    authKind: "key",
    keyPath: "~/.ssh/acme_deploy_ed25519",
    localHost: "127.0.0.1",
    localPort: 3307,
    remoteHost: "127.0.0.1",
    remotePort: 3306,
    forwardKind: "local",
    proxyJump: "bastion.acme.example",
    keepAlive: true,
    autoReconnect: true,
    state: "live",
    running: true,
    startedAtMs: sshAgoMs(42),
    command:
      "ssh -N -L 127.0.0.1:3307:127.0.0.1:3306 -J bastion.acme.example dbadmin@db1.internal.acme.example",
  },
  {
    id: "tun-prod-redis",
    connectionId: "acme-prod-web",
    name: "Prod Redis",
    sshHost: "web1.acme.example",
    sshPort: 22,
    sshUser: "deploy",
    authKind: "key",
    keyPath: "~/.ssh/acme_deploy_ed25519",
    localHost: "127.0.0.1",
    localPort: 6380,
    remoteHost: "127.0.0.1",
    remotePort: 6379,
    forwardKind: "local",
    proxyJump: null,
    keepAlive: true,
    autoReconnect: true,
    state: "live",
    running: true,
    startedAtMs: sshAgoMs(18),
    command: "ssh -N -L 127.0.0.1:6380:127.0.0.1:6379 deploy@web1.acme.example",
  },
  {
    id: "tun-staging-webhook",
    connectionId: "acme-staging",
    name: "Staging webhook (reverse)",
    sshHost: "stage.acme.example",
    sshPort: 22,
    sshUser: "deploy",
    authKind: "key",
    keyPath: "~/.ssh/acme_deploy_ed25519",
    localHost: "127.0.0.1",
    localPort: 3000,
    remoteHost: "0.0.0.0",
    remotePort: 9000,
    forwardKind: "reverse",
    proxyJump: null,
    keepAlive: true,
    autoReconnect: false,
    state: "live",
    running: true,
    startedAtMs: sshAgoMs(180),
    command: "ssh -N -R 0.0.0.0:9000:127.0.0.1:3000 deploy@stage.acme.example",
  },
  {
    id: "tun-edge-socks",
    connectionId: "edge-cache",
    name: "Edge SOCKS proxy",
    sshHost: "edge.acme.example",
    sshPort: 22,
    sshUser: "root",
    authKind: "agent",
    keyPath: null,
    localHost: "127.0.0.1",
    localPort: 1080,
    remoteHost: "",
    remotePort: 0,
    forwardKind: "socks",
    proxyJump: null,
    keepAlive: false,
    autoReconnect: true,
    state: "down",
    running: false,
    startedAtMs: null,
    command: "ssh -N -D 127.0.0.1:1080 root@edge.acme.example",
  },
];

const PROD_HOME = "/home/deploy";
const PROD_APP = `${PROD_HOME}/apps/acme-storefront`;

const SSH_HOSTS: Record<string, DemoSshHost> = {
  "acme-prod-web": {
    homeDir: PROD_HOME,
    snapshotStdout: snapshotOut(
      "deploy",
      "Linux 6.5.0-27-generic",
      " 10:25:41 up 41 days,  6:24,  2 users,  load average: 0.31, 0.28, 0.25",
      15990,
      6432,
      "/dev/root        78G   34G   41G  46% /",
    ),
    psStdout: [
      "USER       PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
      "deploy   48211  1.8  2.0 712044 65212 ?        Ssl  Mar20  64:21 node /home/deploy/apps/acme-storefront/server.js",
      "root      1102  0.4  0.6 124800 21044 ?        Ss   Mar20  12:04 nginx: master process /usr/sbin/nginx",
      "www-data  1140  0.9  0.8 131200 28112 ?        S    Mar20  31:50 nginx: worker process",
      "redis     8843  0.3  0.5  64200 18004 ?        Ssl  Mar20   9:11 redis-server 127.0.0.1:6379",
      "deploy   48377  0.2  0.3  98044 12110 ?        S    Mar20   2:41 pm2: God Daemon",
      "root       922  0.0  0.1  18044  6044 ?        Ss   Mar20   0:31 /usr/sbin/sshd -D",
    ].join("\n"),
    portsStdout: [
      'LISTEN 0      4096         0.0.0.0:22         0.0.0.0:*    users:(("sshd",pid=922,fd=3))',
      'LISTEN 0      511          0.0.0.0:80         0.0.0.0:*    users:(("nginx",pid=1102,fd=6))',
      'LISTEN 0      511          0.0.0.0:443        0.0.0.0:*    users:(("nginx",pid=1102,fd=7))',
      'LISTEN 0      511        127.0.0.1:3000       0.0.0.0:*    users:(("node",pid=48211,fd=18))',
      'LISTEN 0      128        127.0.0.1:6379       0.0.0.0:*    users:(("redis-server",pid=8843,fd=6))',
    ].join("\n"),
    sftp: {
      [PROD_HOME]: [
        sftpDir(PROD_HOME, "apps", 2),
        sftpDir(PROD_HOME, "logs", 0),
        sftpDir(PROD_HOME, ".ssh", 90),
        sftpFile(PROD_HOME, "deploy.sh", 1240, 0o755, 3),
        sftpFile(PROD_HOME, ".bashrc", 3526, 0o644, 90),
        sftpFile(PROD_HOME, ".env", 412, 0o600, 12),
      ],
      [`${PROD_HOME}/apps`]: [sftpDir(`${PROD_HOME}/apps`, "acme-storefront", 2)],
      [PROD_APP]: [
        sftpDir(PROD_APP, "public", 2),
        sftpDir(PROD_APP, "src", 2),
        sftpFile(PROD_APP, "package.json", 884, 0o644, 2),
        sftpFile(PROD_APP, "ecosystem.config.js", 642, 0o644, 9),
        sftpFile(PROD_APP, "README.md", 1320, 0o644, 14),
        sftpFile(PROD_APP, ".env.production", 318, 0o600, 9),
      ],
      [`${PROD_HOME}/logs`]: [
        sftpFile(`${PROD_HOME}/logs`, "deploy.log", 84210, 0o644, 0),
        sftpFile(`${PROD_HOME}/logs`, "access.log", 1284422, 0o644, 0),
      ],
    },
    files: {
      [`${PROD_HOME}/deploy.sh`]: [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "",
        "cd /home/deploy/apps/acme-storefront",
        "git pull --ff-only origin main",
        "pnpm install --frozen-lockfile",
        "pnpm build",
        "pm2 reload ecosystem.config.js --update-env",
        'echo "deployed $(git rev-parse --short HEAD) at $(date -u)"',
        "",
      ].join("\n"),
      [`${PROD_HOME}/.env`]: [
        "NODE_ENV=production",
        "PORT=3000",
        "REDIS_URL=redis://127.0.0.1:6379",
        "",
      ].join("\n"),
      [`${PROD_APP}/package.json`]: [
        "{",
        '  "name": "acme-storefront",',
        '  "version": "2.4.1",',
        '  "private": true,',
        '  "scripts": {',
        '    "build": "next build",',
        '    "start": "next start -p 3000"',
        "  },",
        '  "dependencies": {',
        '    "next": "14.2.3",',
        '    "react": "18.3.1",',
        '    "ioredis": "5.4.1"',
        "  }",
        "}",
        "",
      ].join("\n"),
      [`${PROD_APP}/ecosystem.config.js`]: [
        "module.exports = {",
        "  apps: [",
        "    {",
        '      name: "acme-storefront",',
        '      script: "server.js",',
        "      instances: 2,",
        '      exec_mode: "cluster",',
        '      env: { NODE_ENV: "production", PORT: 3000 },',
        "    },",
        "  ],",
        "};",
        "",
      ].join("\n"),
      [`${PROD_APP}/README.md`]: [
        "# Acme Storefront",
        "",
        "Production Next.js app. Deploys with `~/deploy.sh` (git pull → build →",
        "`pm2 reload`). Fronted by nginx on :443, app on :3000, Redis on :6379.",
        "",
        "Roll back: `pm2 reload ecosystem.config.js` after `git checkout <sha>`.",
        "",
      ].join("\n"),
      [`${PROD_APP}/.env.production`]: [
        "NODE_ENV=production",
        "NEXT_PUBLIC_API_BASE=https://api.acme.example",
        "REDIS_URL=redis://127.0.0.1:6379",
        "",
      ].join("\n"),
      [`${PROD_HOME}/.bashrc`]: [
        "# ~/.bashrc — Acme prod web",
        "export PATH=$HOME/.local/bin:$PATH",
        'alias ll="ls -alF"',
        'alias logs="pm2 logs acme-storefront"',
        "",
      ].join("\n"),
    },
    agent: NO_AGENT,
  },
  "acme-staging": {
    homeDir: "/home/deploy",
    snapshotStdout: snapshotOut(
      "deploy",
      "Linux 6.1.0-21-amd64",
      " 14:02:10 up 9 days,  1:12,  1 user,  load average: 1.84, 1.40, 1.12",
      7960,
      6610,
      "/dev/sda1        40G   31G  6.8G  82% /",
    ),
    psStdout: [
      "USER       PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
      "deploy    3120 22.4  6.1 982044 498212 ?       Rsl  May28  41:02 node /home/deploy/apps/acme-storefront/server.js",
      "deploy    3344 14.0  3.2 412044 261004 ?       Sl   May28  18:50 next-server (v14.2.3)",
      "root       902  0.1  0.4  18044  6044 ?        Ss   May28   0:12 /usr/sbin/sshd -D",
    ].join("\n"),
    portsStdout: [
      'LISTEN 0      4096         0.0.0.0:22         0.0.0.0:*    users:(("sshd",pid=902,fd=3))',
      'LISTEN 0      511        127.0.0.1:3000       0.0.0.0:*    users:(("node",pid=3120,fd=18))',
    ].join("\n"),
    sftp: {
      "/home/deploy": [
        sftpDir("/home/deploy", "apps", 5),
        sftpFile("/home/deploy", "deploy.sh", 1240, 0o755, 5),
        sftpFile("/home/deploy", ".env", 388, 0o600, 5),
      ],
      "/home/deploy/apps": [sftpDir("/home/deploy/apps", "acme-storefront", 5)],
    },
    files: {
      "/home/deploy/.env": ["NODE_ENV=staging", "PORT=3000", ""].join("\n"),
      "/home/deploy/deploy.sh": [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "cd ~/apps/acme-storefront && git pull && pnpm install && pnpm build && pm2 reload all",
        "",
      ].join("\n"),
    },
    agent: NO_AGENT,
  },
  "acme-db-primary": {
    homeDir: "/home/dbadmin",
    snapshotStdout: snapshotOut(
      "dbadmin",
      "Linux 6.5.0-27-generic",
      " 09:41:55 up 120 days, 14:51,  1 user,  load average: 0.62, 0.55, 0.49",
      31980,
      18420,
      "/dev/nvme0n1p1  500G  214G  286G  43% /",
    ),
    psStdout: [
      "USER       PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
      "mysql     2044  6.8 41.0 9820044 6940212 ?     Ssl  Feb02 980:14 /usr/sbin/mysqld",
      "root       880  0.0  0.0  18044  5044 ?        Ss   Feb02   1:02 /usr/sbin/sshd -D",
    ].join("\n"),
    portsStdout: [
      'LISTEN 0      4096         0.0.0.0:22         0.0.0.0:*    users:(("sshd",pid=880,fd=3))',
      'LISTEN 0      151          0.0.0.0:3306       0.0.0.0:*    users:(("mysqld",pid=2044,fd=22))',
    ].join("\n"),
    sftp: {
      "/home/dbadmin": [
        sftpDir("/home/dbadmin", "backups", 0),
        sftpFile("/home/dbadmin", "restore.sh", 902, 0o750, 30),
        sftpFile("/home/dbadmin", ".my.cnf", 148, 0o600, 88),
      ],
      "/home/dbadmin/backups": [
        sftpFile("/home/dbadmin/backups", "acme-2026-06-02.sql.gz", 48211244, 0o640, 0),
        sftpFile("/home/dbadmin/backups", "acme-2026-06-01.sql.gz", 47980022, 0o640, 1),
      ],
    },
    files: {
      "/home/dbadmin/restore.sh": [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "gunzip -c \"$1\" | mysql acme",
        'echo "restored $1"',
        "",
      ].join("\n"),
    },
    agent: NO_AGENT,
  },
  "cpanel-shared": {
    homeDir: "/home/acmeco",
    snapshotStdout: snapshotOut(
      "acmeco",
      "Linux 4.18.0-513.el8.x86_64",
      " 18:30:02 up 211 days,  4:09,  3 users,  load average: 3.21, 2.98, 2.74",
      64200,
      52110,
      "/dev/sdb1       2.0T  1.6T  410G  80% /",
    ),
    psStdout: [
      "USER       PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
      "acmeco   88102  1.1  0.2 244800 18044 ?        S    10:02   0:14 /usr/bin/php-cgi",
      "acmeco   88110  0.0  0.0  12044  4044 pts/0    Ss   18:29   0:00 -bash",
    ].join("\n"),
    portsStdout: [
      'LISTEN 0      128          0.0.0.0:2222       0.0.0.0:*    users:(("sshd",pid=701,fd=3))',
    ].join("\n"),
    sftp: {
      "/home/acmeco": [
        sftpDir("/home/acmeco", "public_html", 6),
        sftpDir("/home/acmeco", "mail", 30),
        sftpFile("/home/acmeco", ".bash_profile", 244, 0o644, 210),
      ],
      "/home/acmeco/public_html": [
        sftpFile("/home/acmeco/public_html", "index.php", 1820, 0o644, 6),
        sftpFile("/home/acmeco/public_html", ".htaccess", 420, 0o644, 40),
      ],
    },
    files: {
      "/home/acmeco/public_html/index.php": [
        "<?php",
        "// Acme marketing — legacy shared host",
        'echo "<h1>Acme</h1>";',
        "",
      ].join("\n"),
    },
    agent: NO_AGENT,
  },
  "edge-cache": {
    homeDir: "/root",
    snapshotStdout: snapshotOut(
      "root",
      "Linux 6.6.7-0-virt",
      " 21:14:39 up 40 days, 22:10,  1 user,  load average: 0.05, 0.04, 0.01",
      1980,
      612,
      "/dev/vda1        20G  4.2G   15G  22% /",
    ),
    psStdout: [
      "USER       PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
      "root       644  0.2  4.0  68044 80044 ?        Ss   Apr22  12:40 nginx: cache manager process",
      "root       420  0.0  0.6  10044 12044 ?        Ss   Apr22   0:20 /usr/sbin/sshd -D",
    ].join("\n"),
    portsStdout: [
      'LISTEN 0      511          0.0.0.0:22         0.0.0.0:*    users:(("sshd",pid=420,fd=3))',
      'LISTEN 0      511          0.0.0.0:80         0.0.0.0:*    users:(("nginx",pid=644,fd=6))',
    ].join("\n"),
    sftp: {
      "/root": [
        sftpDir("/root", "cache", 0),
        sftpFile("/root", "nginx.conf", 2210, 0o644, 14),
      ],
    },
    files: {
      "/root/nginx.conf": [
        "worker_processes auto;",
        "events { worker_connections 1024; }",
        "http {",
        "  proxy_cache_path /root/cache levels=1:2 keys_zone=edge:50m;",
        "  server { listen 80; location / { proxy_cache edge; proxy_pass http://web1.acme.example; } }",
        "}",
        "",
      ].join("\n"),
    },
    agent: NO_AGENT,
  },
  "research-box": {
    homeDir: "/home/researcher",
    snapshotStdout: snapshotOut(
      "researcher",
      "Linux 6.5.0-27-generic",
      " 03:02:11 up 2 days,  8:40,  1 user,  load average: 4.10, 3.88, 2.60",
      128000,
      41200,
      "/dev/nvme0n1p1  1.8T  640G  1.1T  37% /",
    ),
    psStdout: [
      "USER        PID %CPU %MEM     VSZ    RSS TTY      STAT START   TIME COMMAND",
      "researcher 9120 64.0 18.0 48200044 24200044 ?    Rsl  01:10  88:20 ollama runner --model qwen2.5-coder",
      "researcher 9044  2.0  1.0  812044 132044 ?       Ssl  Jun01   6:02 ollama serve",
      "root        780  0.0  0.0  18044  6044 ?         Ss   Jun01   0:08 /usr/sbin/sshd -D",
    ].join("\n"),
    portsStdout: [
      'LISTEN 0      4096         0.0.0.0:22         0.0.0.0:*    users:(("sshd",pid=780,fd=3))',
      'LISTEN 0      4096       127.0.0.1:11434      0.0.0.0:*    users:(("ollama",pid=9044,fd=8))',
      'LISTEN 0      128        127.0.0.1:8888       0.0.0.0:*    users:(("python3",pid=9320,fd=12))',
    ].join("\n"),
    sftp: {
      "/home/researcher": [
        sftpDir("/home/researcher", "notebooks", 1),
        sftpDir("/home/researcher", "models", 2),
        sftpFile("/home/researcher", "train.py", 2840, 0o644, 1),
        sftpFile("/home/researcher", "requirements.txt", 210, 0o644, 2),
      ],
    },
    files: {
      "/home/researcher/requirements.txt": [
        "torch==2.3.0",
        "transformers==4.41.0",
        "accelerate==0.30.1",
        "datasets==2.19.1",
        "",
      ].join("\n"),
      "/home/researcher/train.py": [
        "import torch",
        "from transformers import AutoModelForCausalLM, AutoTokenizer",
        "",
        'MODEL = "Qwen/Qwen2.5-Coder-7B"',
        "",
        "def main():",
        "    tok = AutoTokenizer.from_pretrained(MODEL)",
        "    model = AutoModelForCausalLM.from_pretrained(MODEL, torch_dtype=torch.bfloat16)",
        '    print("loaded", model.num_parameters(), "params")',
        "",
        'if __name__ == "__main__":',
        "    main()",
        "",
      ].join("\n"),
    },
    agent: {
      hasCurl: true,
      hasWget: true,
      hasClaude: false,
      hasCodex: false,
      hasOllama: true,
      hasLlm: false,
      ollamaModels: ["llama3.1:8b", "qwen2.5-coder:7b", "nomic-embed-text"],
      port: 11434,
    },
  },
};

const CF_TUNNELS: TunnelStatus[] = [
  {
    projectId: "acme-storefront",
    upstreamUrl: "https://acme-storefront.test",
    publicUrl: "https://acme-storefront-demo.trycloudflare.com",
    running: true,
    startedAtMs: sshAgoMs(26),
    custom: false,
  },
  {
    projectId: "marketing-site",
    upstreamUrl: "https://marketing.test",
    publicUrl: "https://preview.acme.example",
    running: true,
    startedAtMs: sshAgoMs(150),
    custom: true,
  },
];

const CF_NAMED_TUNNELS: DetectedTunnel[] = [
  {
    uuid: "b6f1c2d4-7a90-4e3b-9c21-0f8a5e6d4b22",
    credentialsFile: "~/.cloudflared/b6f1c2d4-7a90-4e3b-9c21-0f8a5e6d4b22.json",
    suggestedHostname: "preview.acme.example",
  },
];

/** The canonical fixture bag. Deep-cloned by the mock before mutation. */
export const DEMO_FIXTURES: DemoFixtures = {
  projects: PROJECTS,
  sshConnections: SSH_CONNECTIONS,
  sshIdentities: SSH_IDENTITIES,
  sshTunnels: SSH_TUNNELS,
  sshProbes: SSH_PROBES,
  sshHosts: SSH_HOSTS,
  cfTunnels: CF_TUNNELS,
  cfNamedTunnels: CF_NAMED_TUNNELS,
  groups: GROUPS,
  sidecars: SIDECARS,
  entitlement: ENTITLEMENT,
  requests: REQUESTS,
  runtimes: RUNTIMES,
  databaseEngines: DATABASE_ENGINES,
  databaseInstances: DATABASE_INSTANCES,
  dnsRecords: DNS_RECORDS,
  dnsPreflight: DNS_PREFLIGHT,
  resolverStatus: RESOLVER_STATUS,
  webServers: WEB_SERVERS,
  metrics: SYSTEM_METRICS,
  devTools: DEV_TOOLS,
  logs: LOGS,
};
