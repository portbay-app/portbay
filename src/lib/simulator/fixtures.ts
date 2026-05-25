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
    services: [],
    env: {},
    autoStart: true,
    tags: ["static"],
    sandboxed: false,
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
    services: [],
    env: {},
    autoStart: false,
    tags: ["sandbox", "imported"],
    sandboxed: true,
    sandbox: { enabled: true, network: "loopback_only", ephemeral: true },
    status: "running",
    runtime: runtimeOf(48511, 64, 0.9),
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
  },
  {
    id: "redis",
    label: "Redis",
    installed: true,
    version: "7.2.4",
    defaultPort: 6379,
    clientAvailable: true,
    installHint: "",
  },
  {
    id: "postgres",
    label: "PostgreSQL",
    installed: false,
    version: "16.2",
    defaultPort: 5432,
    clientAvailable: false,
    installHint: "brew install postgresql@16",
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
};
