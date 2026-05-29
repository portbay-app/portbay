<!--
  Icon — thin wrapper over @lucide/svelte exposing a `name` prop.

  Why a wrapper at all (lucide can be imported directly)? Two reasons:
    1. Bundle discipline: every icon we ship is listed in `iconMap`, so
       tree-shaking is honest — adding an icon is one explicit line, not a
       reflexive import scattered across components.
    2. A single place to enforce stroke width / size defaults consistent
       with the design system. Lucide's defaults (24×24, stroke-width 2)
       look heavy at PortBay's density.

  Add a new icon by importing it from `@lucide/svelte` and adding it to
  `iconMap`. Don't import lucide icons directly anywhere else.
-->
<script lang="ts" module>
  export type IconName =
    | "play"
    | "stop-circle"
    | "square"
    | "rotate-cw"
    | "external-link"
    | "folder"
    | "pencil"
    | "globe"
    | "settings"
    | "search"
    | "plus"
    | "refresh-cw"
    | "x"
    | "check"
    | "chevron-right"
    | "chevron-down"
    | "home"
    | "server"
    | "link"
    | "file-text"
    | "terminal"
    | "circle-alert"
    | "circle-check"
    | "circle-stop"
    | "info"
    | "package"
    | "zap"
    | "star"
    | "file-code"
    | "rocket"
    | "sparkles"
    | "arrow-right"
    | "bell"
    | "sun"
    | "moon"
    | "more-horizontal"
    | "lock"
    | "users"
    | "user"
    | "activity"
    | "layers"
    | "list"
    | "grid-2x2"
    | "chevrons-up-down"
    | "chevron-up"
    | "log-out"
    | "database"
    | "cloud"
    | "copy"
    | "eye"
    | "shield"
    | "server-cog"
    | "sliders-horizontal"
    | "power"
    | "panel-left-close"
    | "panel-left-open";
</script>

<script lang="ts">
  import {
    Play,
    StopCircle,
    Square,
    RotateCw,
    ExternalLink,
    Folder,
    Pencil,
    Globe,
    Settings,
    Search,
    Plus,
    RefreshCw,
    X,
    Check,
    ChevronRight,
    ChevronDown,
    Home,
    Server,
    Link as LinkIcon,
    FileText,
    Terminal,
    CircleAlert,
    CircleCheck,
    CircleStop,
    Info,
    Package,
    Zap,
    Star,
    FileCode,
    Rocket,
    Sparkles,
    ArrowRight,
    Bell,
    Sun,
    Moon,
    MoreHorizontal,
    Lock,
    Users,
    User,
    Activity,
    Layers,
    List,
    Grid2x2,
    ChevronsUpDown,
    ChevronUp,
    LogOut,
    Database,
    Cloud,
    Copy,
    Eye,
    Shield,
    ServerCog,
    SlidersHorizontal,
    Power,
    PanelLeftClose,
    PanelLeftOpen,
    type Icon as LucideIconComponent,
  } from "@lucide/svelte";

  interface Props {
    name: IconName;
    /** Pixel size. Defaults to 16 — design-system standard for inline use. */
    size?: number;
    /** Stroke width override. Defaults to 1.75 — lucide's 2 reads heavy. */
    strokeWidth?: number;
    class?: string;
  }
  let {
    name,
    size = 16,
    strokeWidth = 1.75,
    class: cls = "",
  }: Props = $props();

  const iconMap: Record<IconName, typeof LucideIconComponent> = {
    play: Play,
    "stop-circle": StopCircle,
    square: Square,
    "rotate-cw": RotateCw,
    "external-link": ExternalLink,
    folder: Folder,
    pencil: Pencil,
    globe: Globe,
    settings: Settings,
    search: Search,
    plus: Plus,
    "refresh-cw": RefreshCw,
    x: X,
    check: Check,
    "chevron-right": ChevronRight,
    "chevron-down": ChevronDown,
    home: Home,
    server: Server,
    link: LinkIcon,
    "file-text": FileText,
    terminal: Terminal,
    "circle-alert": CircleAlert,
    "circle-check": CircleCheck,
    "circle-stop": CircleStop,
    info: Info,
    package: Package,
    zap: Zap,
    star: Star,
    "file-code": FileCode,
    rocket: Rocket,
    sparkles: Sparkles,
    "arrow-right": ArrowRight,
    bell: Bell,
    sun: Sun,
    moon: Moon,
    "more-horizontal": MoreHorizontal,
    lock: Lock,
    users: Users,
    user: User,
    activity: Activity,
    layers: Layers,
    list: List,
    "grid-2x2": Grid2x2,
    "chevrons-up-down": ChevronsUpDown,
    "chevron-up": ChevronUp,
    "log-out": LogOut,
    database: Database,
    cloud: Cloud,
    copy: Copy,
    eye: Eye,
    shield: Shield,
    "server-cog": ServerCog,
    "sliders-horizontal": SlidersHorizontal,
    power: Power,
    "panel-left-close": PanelLeftClose,
    "panel-left-open": PanelLeftOpen,
  };

  const Component = $derived(iconMap[name]);
</script>

<Component {size} {strokeWidth} class={cls} aria-hidden="true" />
