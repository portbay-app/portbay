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

/** The canonical fixture bag. Deep-cloned by the mock before mutation. */
export const DEMO_FIXTURES: DemoFixtures = {
  projects: PROJECTS,
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
