/**
 * ideLayout — persisted layout state for the VS Code–style host workspace
 * (`SshWorkspace`). Holds which activity-bar view is open, the sidebar/panel
 * sizes + visibility, and which bottom-panel tab is active. Mirrors the role of
 * oxideterm's `useIdeStore`, scoped to one workspace at a time.
 *
 * Layout is a per-machine preference, so it persists to `localStorage` (not the
 * registry) and is restored on next launch. Sizes are clamped on write so a bad
 * stored value can't wedge the layout.
 */

/** Left-rail activity views (rendered in the primary sidebar). The Agent lives
 * in its own right-hand aux panel (see `agentVisible`/`agentWidth`), not here. */
export type ActivityView = "explorer" | "deploy" | "tunnels" | "sftp";
export type PanelTab =
  | "terminal"
  | "logs"
  | "processes"
  | "gpu"
  | "ports"
  | "problems"
  | "jobs";

const VALID_VIEWS: ActivityView[] = ["explorer", "deploy", "tunnels", "sftp"];

interface LayoutState {
  activeView: ActivityView;
  sidebarVisible: boolean;
  sidebarWidth: number;
  panelVisible: boolean;
  panelHeight: number;
  panelTab: PanelTab;
  /** The right-hand Agent aux panel (VS Code secondary sidebar). */
  agentVisible: boolean;
  agentWidth: number;
}

const STORAGE_KEY = "portbay.ide.layout";

const SIDEBAR_MIN = 180;
const SIDEBAR_MAX = 520;
const PANEL_MIN = 120;
const PANEL_MAX = 640;
const AGENT_MIN = 300;
const AGENT_MAX = 620;

const DEFAULTS: LayoutState = {
  activeView: "explorer",
  sidebarVisible: true,
  sidebarWidth: 264,
  panelVisible: true,
  panelHeight: 260,
  panelTab: "terminal",
  agentVisible: false,
  agentWidth: 380,
};

const clamp = (n: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, Math.round(n)));

function load(): LayoutState {
  if (typeof localStorage === "undefined") return { ...DEFAULTS };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw) as Partial<LayoutState>;
    return {
      ...DEFAULTS,
      ...parsed,
      // A stored "agent" (its old activity-view id) or any unknown value falls
      // back to Explorer now that Agent is a right-hand panel, not a rail view.
      activeView: VALID_VIEWS.includes(parsed.activeView as ActivityView)
        ? (parsed.activeView as ActivityView)
        : DEFAULTS.activeView,
      sidebarWidth: clamp(parsed.sidebarWidth ?? DEFAULTS.sidebarWidth, SIDEBAR_MIN, SIDEBAR_MAX),
      panelHeight: clamp(parsed.panelHeight ?? DEFAULTS.panelHeight, PANEL_MIN, PANEL_MAX),
      agentWidth: clamp(parsed.agentWidth ?? DEFAULTS.agentWidth, AGENT_MIN, AGENT_MAX),
    };
  } catch {
    return { ...DEFAULTS };
  }
}

function createIdeLayout() {
  const state = $state<LayoutState>(load());

  function persist() {
    if (typeof localStorage === "undefined") return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch {
      /* storage full / disabled — layout just won't persist */
    }
  }

  return {
    get activeView() {
      return state.activeView;
    },
    get sidebarVisible() {
      return state.sidebarVisible;
    },
    get sidebarWidth() {
      return state.sidebarWidth;
    },
    get panelVisible() {
      return state.panelVisible;
    },
    get panelHeight() {
      return state.panelHeight;
    },
    get panelTab() {
      return state.panelTab;
    },
    get agentVisible() {
      return state.agentVisible;
    },
    get agentWidth() {
      return state.agentWidth;
    },

    /** Select an activity view; clicking the active one toggles the sidebar. */
    selectView(view: ActivityView) {
      if (state.activeView === view && state.sidebarVisible) {
        state.sidebarVisible = false;
      } else {
        state.activeView = view;
        state.sidebarVisible = true;
      }
      persist();
    },
    toggleSidebar() {
      state.sidebarVisible = !state.sidebarVisible;
      persist();
    },
    setSidebarWidth(px: number) {
      state.sidebarWidth = clamp(px, SIDEBAR_MIN, SIDEBAR_MAX);
      persist();
    },
    togglePanel() {
      state.panelVisible = !state.panelVisible;
      persist();
    },
    setPanelHeight(px: number) {
      state.panelHeight = clamp(px, PANEL_MIN, PANEL_MAX);
      persist();
    },
    /** Show a panel tab, making the panel visible if it was collapsed. */
    showPanelTab(tab: PanelTab) {
      state.panelTab = tab;
      state.panelVisible = true;
      persist();
    },
    /** Toggle the right-hand Agent aux panel. */
    toggleAgent() {
      state.agentVisible = !state.agentVisible;
      persist();
    },
    setAgentWidth(px: number) {
      state.agentWidth = clamp(px, AGENT_MIN, AGENT_MAX);
      persist();
    },
  };
}

export const ideLayout = createIdeLayout();
