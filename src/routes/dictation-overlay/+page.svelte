<!--
  Notch dictation HUD — the recording overlay for BOTH dictation surfaces
  (dictate-anywhere and the in-app local-engine session; the stt commands
  drive the same state events for in-app sessions).

  Runs in its own transparent, always-on-top webview window (label
  `dictation-overlay`; native behavior in src-tauri/src/overlay_window.rs —
  clickable while visible but never activates the app, so the dictation
  target keeps focus). It follows three event streams and draws the
  FluidVoice-style notch expansion —

    anywhere://state   session transitions (arming → live → processing →
                       polishing → done/error → hidden) + target app + notch
                       geometry
    stt://partial      live transcript hypothesis (streaming models), and the
                       polished text forming during a "polish everywhere" pass
    stt://level        mic RMS for the waveform (every ~150 ms)

  The shape is a 1:1 port of DynamicNotchKit's NotchShape (expanded r-top
  15 / r-bottom 20): a black shape growing out of the camera housing, top
  corners flaring outward to meet the screen edge. The expansion follows
  the Dynamic Island's one rule — the black surface is "hardware", so it
  never fades, blurs or transform-scales; it only changes shape. Hidden IS
  the physical notch outline (pixel-aligned, invisible against it); showing
  morphs width/height/corner-radii out of that outline on a single physical
  spring (bouncy open / critically-damped close, the DynamicNotchKit +
  boring.notch recipe), while the content melts in separately, trailing the
  surface. The expanded recording row is the VoiceInput reference design:
  the target app's icon, a 12-bar frequency animation driven by real mic
  RMS, the mm:ss elapsed clock, and a stop control (rotating square →
  invokes `dictation_overlay_stop`).
  Below it, the scrolling transcript preview (10 pt medium, white .75, max
  180×60, auto-scroll to bottom).

  `preferences.dictation.overlay_position` swaps the wrapper for a floating
  bottom pill (same content, same phases — the option for Macs without a
  notch); the noise floor and preview tail length ride the arming
  transition's `settings` payload.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { Spring } from "svelte/motion";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  type Phase = "hidden" | "arming" | "live" | "processing" | "polishing" | "done" | "error";
  /** Which surface invoked the session (leading-slot seam — see below). */
  type OverlayMode = "dictation" | "edit" | "rewrite";

  interface NotchGeometry {
    windowWidth: number;
    windowHeight: number;
    notchWidth: number;
    notchHeight: number;
    hasNotch: boolean;
    /** "notch" (top, camera housing) or "bottom" (floating pill) — the
     * native side already placed the window accordingly. */
    placement?: "notch" | "bottom";
  }

  /** Overlay knobs from preferences, carried on arming transitions only
   * (kept across the session, like the geometry). */
  interface OverlaySettings {
    /** Raw mic-RMS floor below which the waveform stays flat. */
    noiseFloor: number;
    /** Preview keeps the last N chars of the partial (head-truncated). */
    previewChars: number;
  }

  interface OverlayState {
    phase: Phase;
    appName?: string | null;
    appIcon?: string | null;
    notch?: NotchGeometry | null;
    error?: string | null;
    /** Hands-free (double-tap) session — stopping it is press-or-click. */
    toggle?: boolean;
    mode?: OverlayMode;
    settings?: OverlaySettings | null;
  }

  let phase = $state<Phase>("hidden");
  let appIcon = $state<string | null>(null);
  let appName = $state<string | null>(null);
  let errorText = $state<string | null>(null);
  let toggleMode = $state(false);
  let partial = $state("");
  /** Normalized 0..1 waveform amplitude — adaptive (see the envelope
   * below); replaces a raw level so the bars auto-calibrate to the room. */
  let amplitude = $state(0);
  let mode = $state<OverlayMode>("dictation");
  /** Seeds the adaptive envelope's starting floor; real value arrives with
   * arming. The old fixed noise threshold, kept as a seed only (the knob
   * is no longer user-facing — the waveform self-calibrates). */
  let noiseFloor = $state(0.01);
  let previewChars = $state(150);

  // ---- Elapsed clock (reference design's timer) ----------------------------
  /** Seconds since the mic went live — starts at the live transition, never
   * at arming (counting through the model load would claim recording time
   * that captured nothing, same rule as micSession's clock). */
  let seconds = $state(0);
  let clock: ReturnType<typeof setInterval> | null = null;

  function startClock() {
    stopClock();
    seconds = 0;
    clock = setInterval(() => {
      seconds += 1;
    }, 1000);
  }
  function stopClock() {
    if (clock) clearInterval(clock);
    clock = null;
  }
  const timeLabel = $derived(
    `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`,
  );

  /** The stop control — same semantics as releasing Fn (hold), tapping Fn
   * (toggle) or clicking the in-app mic button: finish and deliver. */
  function requestStop() {
    void invoke("dictation_overlay_stop").catch(() => {});
  }

  /** Last known geometry — sent with the arming transition, kept across
   * the session (later transitions carry `notch: null`). */
  let notch = $state<NotchGeometry>({
    windowWidth: 420,
    windowHeight: 200,
    notchWidth: 200,
    notchHeight: 32,
    hasNotch: false,
    placement: "notch",
  });

  /** Which variant to draw — the geometry's placement (the native window
   * is already where this says). */
  const placement = $derived(notch.placement === "bottom" ? "bottom" : "notch");

  // ---- Shape math (DynamicNotchKit NotchShape) ----------------------------
  /** Top corners flare OUTWARD (concave, meeting the screen edge); bottom
   * corners are normal convex rounds. Expanded radii. */
  const R_TOP = 15;
  const R_BOTTOM = 20;
  /** Collapsed radii (DynamicNotchKit's compact 6/14) — the hidden outline
   * hugs the camera housing's own curvature, so at rest the shape is
   * indistinguishable from the hardware it sits on. */
  const RT_HIDDEN = 6;
  const RB_HIDDEN = 14;
  /** Content column width (FluidVoice's ~176 pt notchContentWidth). */
  const CONTENT_WIDTH = 176;

  const expanded = $derived(phase !== "hidden");
  /** The black shape's width: at least the physical notch, at least the
   * content + its side insets, plus the flare radii. */
  const shapeInner = $derived(Math.max(notch.notchWidth, CONTENT_WIDTH + 30));
  const shapeW = $derived(shapeInner + R_TOP * 2);
  /** Measured preview height (capped at 60, the FluidVoice max). */
  let previewHeight = $state(0);
  /** Height: notch spacer + control row + preview (when present). */
  const previewVisible = $derived(
    (phase === "live" && partial.trim().length > 0) ||
      phase === "processing" ||
      phase === "polishing" ||
      phase === "error",
  );
  const shapeH = $derived(notch.notchHeight + 6 + 18 + (previewVisible ? 6 + previewHeight : 0) + 10);

  // ---- Dynamic Island geometry spring ---------------------------------------
  // The island illusion's one rule: the black surface reads as HARDWARE — it
  // never fades, blurs or transform-scales; it only changes shape. So all
  // four geometry channels (width, height, both radii) ride ONE physical
  // spring, and the path is rebuilt from the spring's in-flight values every
  // frame: the notch outline itself swells into the HUD and shrinks back.
  // Parameters follow the references — DynamicNotchKit opens .bouncy(0.4) /
  // closes .smooth(0.4); boring.notch opens spring(0.42, 0.8) / closes
  // spring(0.45, 1.0). Open gets one visible micro-bounce, close none.
  const SPRING_OPEN = { stiffness: 0.2, damping: 0.62 };
  const SPRING_CLOSE = { stiffness: 0.24, damping: 1 };
  /** In-session resizes (preview growing, hover) — snappy, tiny overshoot. */
  const SPRING_RESIZE = { stiffness: 0.28, damping: 0.8 };

  /** The exact physical-notch outline — the collapsed/rest geometry. The
   * RT_HIDDEN flares stand in for the housing's own corner curvature. */
  function collapsedGeo(n: NotchGeometry) {
    return { w: n.notchWidth + RT_HIDDEN * 2, h: n.notchHeight, rt: RT_HIDDEN, rb: RB_HIDDEN };
  }

  /* svelte-ignore state_referenced_locally -- intentionally the initial
     value: the spring seeds from the default outline; real geometry arrives
     with arming and hard-resets it (see the phase handler). */
  const geo = new Spring(collapsedGeo(notch), { ...SPRING_RESIZE, precision: 0.01 });

  /** Hover micro-inflate — the island's "alive" response to the pointer
   * (a few points of real geometry, never a pixel scale). */
  let hovered = $state(false);
  const hoverBoost = $derived(expanded && hovered ? 1 : 0);

  const targetGeo = $derived(
    expanded
      ? { w: shapeW + hoverBoost * 6, h: shapeH + hoverBoost * 3, rt: R_TOP, rb: R_BOTTOM }
      : collapsedGeo(notch),
  );

  /** Which spring the next retarget rides — set by the phase handler (open /
   * close), consumed once, then back to in-session resize. Plain let: it
   * routes the effect below, it isn't rendered. */
  let springMode: "open" | "close" | "resize" = "resize";
  /** When the close started. A session landing shortly after must NOT
   * teleport the spring to the collapsed outline — the window is still on
   * screen mid-collapse, so the morph continues from wherever it is. After
   * the window has been ordered out (rAF freezes for occluded windows) a
   * fresh show hard-resets to the outline so frame 0 is exactly the notch. */
  let closingSince = 0;

  $effect(() => {
    const target = targetGeo;
    const params =
      springMode === "open" ? SPRING_OPEN : springMode === "close" ? SPRING_CLOSE : SPRING_RESIZE;
    springMode = "resize";
    geo.stiffness = params.stiffness;
    geo.damping = params.damping;
    geo.target = target;
  });

  /** NotchShape path for the spring's in-flight geometry — same command
   * structure at every size, radii clamped so overshoot can never fold the
   * curves over themselves. */
  function notchPath(w: number, h: number, rtRaw: number, rbRaw: number): string {
    const rt = Math.max(0, Math.min(rtRaw, w / 2));
    const rb = Math.max(0, Math.min(rbRaw, (w - rt * 2) / 2, h - rt));
    const f = (n: number) => n.toFixed(2);
    return (
      `M 0 0` +
      ` Q ${f(rt)} 0 ${f(rt)} ${f(rt)}` +
      ` L ${f(rt)} ${f(h - rb)}` +
      ` Q ${f(rt)} ${f(h)} ${f(rt + rb)} ${f(h)}` +
      ` L ${f(w - rt - rb)} ${f(h)}` +
      ` Q ${f(w - rt)} ${f(h)} ${f(w - rt)} ${f(h - rb)}` +
      ` L ${f(w - rt)} ${f(rt)}` +
      ` Q ${f(w - rt)} 0 ${f(w)} 0 Z`
    );
  }

  const shapePath = $derived(notchPath(geo.current.w, geo.current.h, geo.current.rt, geo.current.rb));

  /** Shadow rides the expansion progress (the radii are monotonic between
   * the states, so they double as the progress signal): collapsed stays
   * shadowless to blend with the housing; expanded gets the island's subtle
   * lift against the wallpaper (boring.notch does the same — open only). */
  const shadowAlpha = $derived(
    0.5 * Math.min(Math.max((geo.current.rt - RT_HIDDEN) / (R_TOP - RT_HIDDEN), 0), 1),
  );

  /** Content choreography: the surface leads, the content melts in ~80 ms
   * behind it (blur + scale + fade live on the inner column ONLY — never on
   * the black shape). On close the content drops out fast while the
   * critically-damped spring is still easing in, so the island always
   * empties before it visibly shrinks — Apple's ordering. */
  let contentIn = $state(false);
  $effect(() => {
    if (!expanded) {
      contentIn = false;
      return;
    }
    const t = setTimeout(() => (contentIn = true), 80);
    return () => clearTimeout(t);
  });

  // ---- Frequency bars (VoiceInput reference design) -------------------------
  /** 12 thin bars, 2 px wide with 2 px gaps, 2–13 px tall — amplitude comes
   * from the real mic RMS (every ~150 ms), per-bar variation is randomized
   * each tick so the line field reads as live frequency, not a meter. */
  const BAR_COUNT = 12;
  const BAR_MIN = 2;
  const BAR_MAX = 13;

  // ---- Adaptive waveform envelope ------------------------------------------
  /** The bars auto-calibrate to the room instead of mapping a fixed RMS
   * window (the old 0.01→0.16 mapping made ordinary speech — which peaks
   * around 0.05–0.1 — read as barely-there bars). A slow-tracking floor
   * settles on the ambient noise within a second or two; a fast-attack /
   * slow-decay peak tracks speech. So speaking fills the bars whether the
   * mic is hot in a silent booth or a busy café, and steady room noise
   * (which sits at the floor) leaves them flat — no threshold to tune. */
  const SPAN_MIN = 0.035; // smallest floor→peak range — silence can't blow up
  const PEAK_DECAY = 0.94; // per-tick (~150 ms) peak decay back toward the floor
  let envFloor = 0.008; // ambient baseline (plain working state, not reactive)
  let envPeak = 0.05; // recent speech ceiling

  /** Smoothed amplitude the bars actually draw — the rAF loop eases it
   * toward `amplitude` (the live target) so motion is continuous, not a
   * 150 ms staircase. Plain `let`; only `liveBars` (its output) is reactive. */
  let ampDisplay = 0;
  let liveBars = $state<number[]>(Array.from({ length: BAR_COUNT }, () => BAR_MIN));

  /** New session, possibly a new room: re-seed the envelope from the (kept)
   * noise-floor pref and let it re-converge. */
  function resetEnvelope() {
    envFloor = Math.max(noiseFloor, 0.004);
    envPeak = envFloor + SPAN_MIN;
    amplitude = 0;
    ampDisplay = 0;
  }

  /** Fold one mic RMS sample into the envelope and recompute the 0..1
   * amplitude target the bars ease toward. */
  function feedLevel(rms: number) {
    // Floor: attack down fast toward quieter samples, creep up slowly —
    // converges on the room's noise floor and follows it if it drifts.
    envFloor += (rms < envFloor ? 0.4 : 0.01) * (rms - envFloor);
    // Peak: snap up to louder samples, decay gently so the scale holds
    // through the gaps between words; never closer than SPAN_MIN to the
    // floor, so a dead-quiet room doesn't amplify hiss to full height.
    const floored = envFloor + SPAN_MIN;
    envPeak = rms > envPeak ? rms : floored + (envPeak - floored) * PEAK_DECAY;
    if (envPeak < floored) envPeak = floored;
    const norm = Math.min(Math.max((rms - envFloor) / (envPeak - envFloor), 0), 1);
    // Perceptual lift: linear RMS puts ordinary speech low on the scale, so
    // a quiet room barely moves the bars. A gamma < 1 raises the low-mid
    // range — speech fills the bars; the idle baseline (rAF loop) keeps them
    // alive when it drops. This is the TARGET the rAF loop eases toward.
    amplitude = Math.pow(norm, 0.55);
  }

  // The bars animate from a requestAnimationFrame loop while live (and
  // briefly on error), not from the 150 ms RMS tick — so the motion is
  // smooth and a near-silent room still reads as a gentle, breathing
  // waveform instead of a flat 2 px line. Each frame eases the drawn level
  // toward `amplitude`, then adds a small idle baseline + organic per-bar
  // motion (no per-frame Math.random — that read as jitter, not sound).
  $effect(() => {
    if (phase !== "live" && phase !== "error") {
      ampDisplay = 0;
      liveBars = Array.from({ length: BAR_COUNT }, () => BAR_MIN);
      return;
    }
    let raf = 0;
    const start = performance.now();
    const draw = (now: number) => {
      const t = (now - start) / 1000;
      // Fast attack, soft release: peaks pop, tails settle gently.
      const k = amplitude > ampDisplay ? 0.4 : 0.12;
      ampDisplay += (amplitude - ampDisplay) * k;
      // Idle baseline — the bars never fully flatten while live, so a quiet
      // room reads as "listening", not "off". Speech rises well above it.
      const IDLE = 0.16;
      const level = IDLE + (1 - IDLE) * ampDisplay;
      const mid = (BAR_COUNT - 1) / 2;
      liveBars = Array.from({ length: BAR_COUNT }, (_, i) => {
        // Two offset waves → continuous, non-uniform motion; a centre-
        // weighted envelope arcs the field like an equalizer.
        const wob = 0.5 + 0.5 * Math.sin(t * 7 + i * 0.9) * Math.cos(t * 3.1 + i * 0.55);
        const centre = 1 - Math.abs(i - mid) / mid;
        const shape = 0.5 + 0.5 * centre;
        return Math.max(BAR_MIN, BAR_MIN + (BAR_MAX - BAR_MIN) * level * shape * (0.4 + 0.6 * wob));
      });
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  });

  const barHeights = $derived(
    phase === "live" || phase === "error"
      ? liveBars
      : Array.from({ length: BAR_COUNT }, () => BAR_MIN),
  );

  const waveColor = $derived(phase === "error" ? "rgb(255, 89, 89)" : "rgba(255, 255, 255, 0.85)");

  // ---- Preview text ---------------------------------------------------------
  /** Preview tail (FluidVoice's preview char limit): keep only the last N
   * chars of the partial — head-truncated at a word edge so the newest
   * words are always visible and a long dictation never grows the DOM
   * unbounded. The top mask fade already reads as "more above". */
  const partialTail = $derived.by(() => {
    if (partial.length <= previewChars) return partial;
    const tail = partial.slice(-previewChars);
    const cut = tail.indexOf(" ");
    return cut > 0 ? tail.slice(cut + 1) : tail;
  });

  /** Map any error detail to a short, clear message for the notch — the
   * defensive layer so no edge case ever shows a raw, empty, or null/undefined
   * string. Known causes (engine missing, timeout, mic/permission, rewrite
   * provider offline) get specific copy; a genuine message passes through
   * (length-capped); anything unusable falls back to a plain failure line. */
  function friendlyError(raw: string | null): string {
    const text = (raw ?? "").trim();
    const lower = text.toLowerCase();
    // Treat empty and the stringified empties as "no detail".
    const usable = text && lower !== "null" && lower !== "undefined";
    if (!usable) return "Dictation failed — try again.";
    const has = (...needles: string[]) => needles.some((n) => lower.includes(n));
    // Paste rescue copy is already user-facing and actionable — keep it.
    if (has("paste", "copied", "⌘v")) return text;
    if (has("timed out", "timeout")) return "Timed out — try again.";
    if (has("missing", "reinstall")) return "Speech engine missing — reinstall PortBay.";
    if (has("requires_macos", "macos 14", "unsupported"))
      return "Local speech needs a newer macOS.";
    if (has("microphone", "mic ", "no audio", "audio input"))
      return "No microphone input — check permissions.";
    if (has("accessibility", "not trusted", "permission"))
      return "Grant Accessibility in System Settings.";
    // A rewrite provider being down should keep the raw transcript, but if one
    // ever surfaces here, say so plainly instead of leaking a connection error.
    if (has("ollama", "connection refused", "refused", "econnrefused", "not reachable", "rewrite"))
      return "Rewrite model offline — kept your words as spoken.";
    if (has("sidecar", "exited", "read failed", "write failed", "decode", "transcription"))
      return "Speech engine error — try again.";
    if (has("no capture session", "already")) return "Recording already stopped.";
    if (has("capture", "failed to start", "start")) return "Couldn’t start recording — try again.";
    // A real, specific message we didn't anticipate: show it, but keep the
    // notch readable (it's ~22 chars wide before wrapping).
    return text.length > 120 ? `${text.slice(0, 117)}…` : text;
  }

  /** Polishing, but the first streamed token hasn't landed yet (or the
   * provider is one-shot AFM, which never streams): show a centered
   * "Polishing" status instead of an empty preview. Once text forms, it reads
   * as a normal left-aligned preview. */
  const polishingStatus = $derived(phase === "polishing" && partialTail.trim().length === 0);

  const previewText = $derived(
    phase === "error"
      ? friendlyError(errorText)
      : phase === "processing"
        ? "Transcribing"
        : polishingStatus
          ? "Polishing"
          : partialTail,
  );

  let previewEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void previewText;
    if (previewEl) {
      previewHeight = Math.min(previewEl.scrollHeight, 60);
      previewEl.scrollTop = previewEl.scrollHeight;
    } else {
      previewHeight = 0;
    }
  });

  onMount(() => {
    const unlisteners: Promise<UnlistenFn>[] = [
      listen<OverlayState>("anywhere://state", (event) => {
        const next = event.payload;
        const wasLive = phase === "live";
        const wasHidden = phase === "hidden";
        phase = next.phase;
        if (next.notch) notch = next.notch;
        // Geometry-spring routing: fresh shows morph OUT of the physical
        // notch outline; hides morph back INTO it (see closingSince above
        // for the mid-collapse re-arm exception).
        if (next.phase !== "hidden" && wasHidden) {
          if (performance.now() - closingSince > 800) {
            void geo.set(collapsedGeo(notch), { instant: true });
          }
          springMode = "open";
        } else if (next.phase === "hidden" && !wasHidden) {
          springMode = "close";
          closingSince = performance.now();
          hovered = false;
        }
        if (next.appName !== undefined && next.appName !== null) appName = next.appName;
        if (next.appIcon !== undefined && next.appIcon !== null) appIcon = next.appIcon;
        if (next.toggle !== undefined) toggleMode = next.toggle;
        if (next.mode) mode = next.mode;
        // Settings ride the arming transition only (like the geometry).
        if (next.settings) {
          noiseFloor = next.settings.noiseFloor;
          previewChars = next.settings.previewChars;
        }
        errorText = next.error ?? null;
        // Clock runs while live only; leaving live freezes the value (the
        // processing state shows the final duration), arming/hidden reset it.
        if (next.phase === "live" && !wasLive) startClock();
        if (next.phase !== "live") stopClock();
        if (next.phase === "arming") {
          partial = "";
          resetEnvelope();
          seconds = 0;
        }
        // Entering polish: clear the last transcript hypothesis so the
        // preview starts empty and fills with the forming polished text.
        if (next.phase === "polishing") partial = "";
        if (next.phase === "hidden") {
          partial = "";
          amplitude = 0;
          seconds = 0;
          appIcon = null;
          appName = null;
          toggleMode = false;
          mode = "dictation";
        }
      }),
      listen<{ text: string }>("stt://partial", (event) => {
        // Live capture hypothesis, OR the polished text forming during a
        // "Polish dictation everywhere" pass (same channel; capture has
        // already stopped by then, so the two never overlap).
        if (phase === "live" || phase === "polishing") partial = event.payload?.text ?? "";
      }),
      listen<{ rms: number }>("stt://level", (event) => {
        if (phase === "live") feedLevel(event.payload?.rms ?? 0);
      }),
    ];
    return () => {
      for (const u of unlisteners) void u.then((f) => f());
    };
  });
</script>

<!-- The HUD's content — identical for both placements (same phases, same
     controls); only the black wrapper differs. `topInset` is the physical
     notch spacer (0 for the bottom pill). -->
{#snippet hudContent(topInset: number)}
  <div
    class="content"
    class:in={contentIn}
    style:padding-top="{topInset}px"
    style:width="{CONTENT_WIDTH}px"
  >
    <div class="row">
      <!-- Leading slot: the stop control while recording (the reference
           design's rotating square, where the mic/dot used to sit);
           the target-app cue takes over once the keys are done.
           SEAM: future animated per-mode icons mount here, keyed off
           data-mode (dictation | edit | rewrite). White palette only —
           mode never changes colors. -->
      <span class="leading" data-mode={mode}>
        {#if phase === "live" || phase === "arming"}
          <button
            class="stop"
            type="button"
            onclick={requestStop}
            title={toggleMode ? "Stop dictation (or tap Fn)" : "Stop dictation"}
            aria-label="Stop dictation"
          >
            <span class="stop-square"></span>
          </button>
        {:else if appIcon}
          <img class="app-icon" src={appIcon} alt={appName ?? "Target app"} />
        {:else}
          <span class="app-dot" style:background={waveColor}></span>
        {/if}
      </span>
      <div
        class="wave"
        class:processing={phase === "processing" || phase === "arming" || phase === "polishing"}
        aria-hidden="true"
      >
        <!-- Processing/arming: no overlay — the bars themselves pulse in a
             staggered wave (the old shimmer sweep read as a white box). -->
        {#each barHeights as h, i (i)}
          <span
            class="bar"
            style:height="{h}px"
            style:background={waveColor}
            style:animation-delay="{i * 70}ms"
            style:box-shadow={phase === "live" || phase === "error"
              ? `0 0 1.5px ${waveColor.replace("0.85", "0.35")}`
              : "none"}
          ></span>
        {/each}
      </div>
      {#if phase === "live" || phase === "processing"}
        <span class="clock">{timeLabel}</span>
      {/if}
    </div>

    {#if previewVisible && previewText}
      <div
        class="preview"
        class:status={phase === "processing" || polishingStatus}
        class:error={phase === "error"}
        bind:this={previewEl}
        role="status"
        aria-live="polite"
      >
        {previewText}{#if phase === "processing" || polishingStatus}<span class="ellipsis"></span>{/if}
      </div>
    {/if}
  </div>
{/snippet}

<!-- Transparent stage; the shape pins to the top-center (the notch) or
     floats as a pill at the bottom-center, per the placement preference
     (the native window is already positioned to match). -->
<div class="stage" class:bottom={placement === "bottom"}>
  {#if placement === "bottom"}
    <div class="pill" class:expanded>
      {@render hudContent(0)}
    </div>
  {:else}
    <!-- The drop-shadow lives on a host ABOVE the clip: a filter on the
         clipped element itself would have its shadow cut away with the
         overflow, while the host shadows the already-clipped composite, so
         the lift follows the morphing outline exactly. -->
    <div
      class="shadow-host"
      style:filter={shadowAlpha > 0.004
        ? `drop-shadow(0 3px 9px rgba(0, 0, 0, ${shadowAlpha.toFixed(3)}))`
        : "none"}
    >
      <!-- Hover handlers drive the decorative micro-inflate only — the
           real interactive element (the stop button) lives inside. -->
      <div
        class="notch"
        class:expanded
        class:ghost={!notch.hasNotch && !expanded}
        style:width="{geo.current.w}px"
        style:height="{geo.current.h}px"
        style:clip-path='path("{shapePath}")'
        role="presentation"
        onmouseenter={() => (hovered = true)}
        onmouseleave={() => (hovered = false)}
      >
        <!-- Content sits below the physical notch, inside the flare insets. -->
        {@render hudContent(notch.notchHeight)}
      </div>
    </div>
  {/if}
</div>

<style>
  .stage {
    position: fixed;
    inset: 0;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    overflow: hidden;
    background: transparent;
    user-select: none;
    cursor: default;
    /* The window accepts mouse events while visible; DOM-wise only the
       notch shape (the stop button inside it) is interactive. */
    pointer-events: none;
  }

  .shadow-host {
    pointer-events: none;
  }

  /* The black surface — NO transform, NO blur, NO opacity animation. It is
     "hardware": the geometry spring drives width/height/clip-path per frame
     and the shape morphs out of (and back into) the physical notch outline.
     Anything that fades or scales here breaks the island illusion. */
  .notch {
    background: #000;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    overflow: hidden;
    opacity: 1;
    transition: opacity 0.15s ease-out;
  }

  .notch.expanded {
    pointer-events: auto;
  }

  /* Screens without a camera housing have nothing for the collapsed outline
     to blend into — fade the surface at the TAIL of the collapse (the morph
     still plays in full; this only hides the final notch-shaped sliver).
     Never applied on notched screens. */
  .notch.ghost {
    opacity: 0;
    transition: opacity 0.2s ease-in 0.26s;
  }

  /* Bottom placement: the same content in a floating pill anchored to the
     stage's bottom-center (the native window already sits ~50 pt above the
     visible frame's bottom edge). Same window discipline — never key,
     click-through except the shape — and the same enter/exit spring, just
     rising from below instead of growing out of the housing. */
  .stage.bottom {
    align-items: flex-end;
  }

  .pill {
    background: #000;
    border-radius: 18px;
    padding: 4px 16px 2px;
    transform-origin: bottom center;
    transform: translateY(14px) scale(0.9);
    opacity: 0;
    filter: blur(8px);
    transition:
      transform 0.4s cubic-bezier(0.32, 0.72, 0, 1),
      opacity 0.3s ease-out,
      filter 0.35s ease-out;
  }

  .pill.expanded {
    transform: translateY(0) scale(1);
    opacity: 1;
    filter: blur(0);
    pointer-events: auto;
  }

  /* The content layer carries ALL the fade/blur/scale (DynamicNotchKit's
     expanded-content transition: blur(10) + scale(y 0.6, anchor top) +
     opacity) — it melts in trailing the surface and drops out fast before
     the surface shrinks, so the island always empties before collapsing. */
  .content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding-bottom: 4px;
    opacity: 0;
    filter: blur(10px);
    transform: scaleY(0.6);
    transform-origin: top center;
    transition:
      opacity 0.14s ease-in,
      filter 0.14s ease-in,
      transform 0.14s ease-in;
  }

  .content.in {
    opacity: 1;
    filter: blur(0);
    transform: none;
    transition:
      opacity 0.26s cubic-bezier(0.22, 1, 0.36, 1),
      filter 0.26s cubic-bezier(0.22, 1, 0.36, 1),
      transform 0.32s cubic-bezier(0.3, 1.25, 0.6, 1);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 18px;
    margin-top: 4px;
  }

  /* The leading slot — sized by its child (stop control / app icon / dot).
     Per-mode animated icons mount inside this span, keyed off data-mode. */
  .leading {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  /* Phase swaps inside the row (stop control → app icon, clock appearing)
     melt in the same way the island swaps content — a quick blur-and-scale
     materialize, never a hard pop. */
  .leading > *,
  .clock {
    animation: content-swap-in 0.22s cubic-bezier(0.22, 1, 0.36, 1);
  }

  @keyframes content-swap-in {
    from {
      opacity: 0;
      filter: blur(4px);
      transform: scale(0.7);
    }
  }

  .app-icon {
    width: 16px;
    height: 16px;
    border-radius: 3px;
    object-fit: contain;
  }

  .app-dot {
    width: 8px;
    height: 8px;
    border-radius: 9999px;
    opacity: 0.9;
  }

  .wave {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 2px;
    width: 50px;
    height: 18px;
    overflow: hidden;
  }

  .bar {
    width: 2px;
    border-radius: 9999px;
    /* The sidecar emits a level every ~150 ms; easing across that cadence
       reads as continuous motion (the reference animates 1 s loops; real
       RMS ticks beat canned randomness). */
    transition:
      height 0.15s ease-out,
      background 0.2s ease;
  }

  .clock {
    min-width: 27px;
    font-size: 9px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: rgba(255, 255, 255, 0.6);
  }

  /* The stop control — the reference design's rotating square. */
  .stop {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: none;
    border-radius: 9999px;
    background: transparent;
    cursor: pointer;
  }

  .stop:hover {
    background: rgba(255, 255, 255, 0.14);
  }

  .stop-square {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.9);
    animation: stop-spin 2s ease-in-out infinite;
  }

  .stop:hover .stop-square {
    animation-play-state: paused;
    transform: scale(1.15);
  }

  @keyframes stop-spin {
    0% {
      transform: rotate(0deg);
    }
    50% {
      transform: rotate(180deg);
    }
    100% {
      transform: rotate(360deg);
    }
  }

  /* Processing/arming: the bars themselves carry the activity — a wave of
     brightness travels across them via the per-bar animation-delay (the old
     gradient sweep overlay read as a white box over the visualizer). */
  .wave.processing .bar {
    opacity: 0.16;
    animation: bar-pulse 1.05s ease-in-out infinite;
  }

  @keyframes bar-pulse {
    0%,
    100% {
      opacity: 0.16;
    }
    50% {
      opacity: 0.9;
    }
  }

  .preview {
    max-height: 60px;
    width: 100%;
    overflow: hidden;
    font-size: 10px;
    font-weight: 500;
    line-height: 1.45;
    text-align: left;
    color: rgba(255, 255, 255, 0.75);
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, black 8px);
    mask-image: linear-gradient(to bottom, transparent 0, black 8px);
  }

  .preview.status {
    text-align: center;
    color: rgba(255, 255, 255, 0.6);
    animation: status-pulse 1.05s ease-in-out infinite;
  }

  .preview.error {
    text-align: center;
    color: rgb(255, 128, 128);
  }

  @keyframes status-pulse {
    0%,
    100% {
      opacity: 0.55;
    }
    50% {
      opacity: 1;
    }
  }

  .ellipsis::after {
    content: "…";
  }
</style>
