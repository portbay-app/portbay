<!-- NotificationsPanel — category/channel matrix, DND, and sound cues. -->
<script lang="ts">
  import { onMount } from "svelte";

  import ChannelChips from "$lib/components/atoms/ChannelChips.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import Segmented from "$lib/components/atoms/Segmented.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import {
    AGENT_SOUND_EVENTS,
    NOTIFICATION_CATEGORIES,
    NOTIFICATION_CHANNELS,
    NOTIFICATION_CUES,
    type AgentSoundEvent,
    type NotificationCategory,
    type NotificationChannel,
    type NotificationCue,
    type NotificationQuietHours,
    type NotificationSeverityFloor,
  } from "$lib/notifications/prefs";
  import { previewCue } from "$lib/sound/play";
  import { notificationPrefs } from "$lib/stores/notificationPrefs.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  const severityOptions: { value: NotificationSeverityFloor; label: string }[] = [
    { value: "errors_only", label: "Errors only" },
    { value: "errors_and_warnings", label: "Errors & warnings" },
    { value: "everything", label: "Everything" },
  ];

  // The "where does it show up" channels. Sound is owned by the Sound cues
  // section below so it isn't set in two places.
  const DELIVERY_CHANNELS = NOTIFICATION_CHANNELS.filter((channel) => channel.id !== "sound").map(
    (channel) => ({ id: channel.id, label: channel.shortLabel }),
  );

  let resetArmed = $state(false);

  function setChannel(category: NotificationCategory, channel: NotificationChannel, on: boolean) {
    notificationPrefs.update((draft) => {
      draft.channels[category][channel] = on;
    });
  }

  function setSeverityFloor(value: NotificationSeverityFloor) {
    notificationPrefs.update((draft) => {
      draft.severityFloor = value;
    });
  }

  function setQuietHours(patch: Partial<NotificationQuietHours>) {
    notificationPrefs.update((draft) => {
      draft.quietHours = { ...draft.quietHours, ...patch };
    });
  }

  function setSnoozeUntil(value: number | null) {
    notificationPrefs.update((draft) => {
      draft.snoozeUntil = value;
    });
  }

  function pauseForOneHour() {
    setSnoozeUntil(Date.now() + 60 * 60 * 1_000);
  }

  function pauseUntilTomorrow() {
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(9, 0, 0, 0);
    setSnoozeUntil(tomorrow.getTime());
  }

  function setAgentEventEnabled(event: AgentSoundEvent, enabled: boolean) {
    notificationPrefs.update((draft) => {
      draft.sound.agentEvents[event].enabled = enabled;
    });
  }

  function setAgentEventCue(event: AgentSoundEvent, cue: NotificationCue) {
    notificationPrefs.update((draft) => {
      draft.sound.agentEvents[event].cue = cue;
    });
  }

  function reset() {
    if (!resetArmed) {
      resetArmed = true;
      window.setTimeout(() => (resetArmed = false), 3_000);
      return;
    }
    resetArmed = false;
    void notificationPrefs.resetToDefaults();
  }

  function snoozeLabel(): string {
    const until = notificationPrefs.value.snoozeUntil;
    if (!until || Date.now() >= until) return "Not paused";
    return `Paused until ${new Date(until).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    })}`;
  }

  onMount(() => {
    void notificationPrefs.load();
  });
</script>

<SettingsPanel
  title="Notifications"
  description="Choose which events reach the toast corner, bell history, desktop banners, and sound."
>
  <div class="space-y-7">
    <section class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Severity floor</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            A coarse filter before category and channel rules run.
          </p>
        </div>
        <Segmented
          value={notificationPrefs.value.severityFloor}
          options={severityOptions}
          label="Notification severity floor"
          onchange={setSeverityFloor}
        />
      </div>

      <div class="py-3">
        <div class="mb-1 flex items-center justify-between gap-3">
          <div>
            <span class="text-[13px] font-medium text-fg">Channels by category</span>
            <p class="text-[12px] text-fg-subtle mt-0.5">
              Pick where each kind of event shows up. Sound is set under Sound cues.
            </p>
          </div>
          {#if notificationPrefs.saving}
            <span class="text-[11px] text-fg-subtle">Saving…</span>
          {/if}
        </div>

        <div class="divide-y divide-border/60">
          {#each NOTIFICATION_CATEGORIES as category (category.id)}
            <div
              class="flex items-center justify-between gap-3 py-2.5 max-[640px]:flex-col max-[640px]:items-start"
            >
              <div class="min-w-0">
                <span class="block text-[13px] font-medium text-fg">{category.label}</span>
                <span class="block text-[12px] text-fg-subtle mt-0.5">{category.description}</span>
              </div>
              <ChannelChips
                label={category.label}
                channels={DELIVERY_CHANNELS}
                value={notificationPrefs.value.channels[category.id]}
                onchange={(channel, on) => setChannel(category.id, channel, on)}
              />
            </div>
          {/each}
        </div>
      </div>
    </section>

    <section class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <div class="min-w-0">
          <span class="text-[13px] font-medium text-fg">Quiet hours</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Suppresses interruptive channels during the selected window.
          </p>
        </div>
        <Toggle
          checked={notificationPrefs.value.quietHours.enabled}
          label="Enable quiet hours"
          onchange={(enabled) => setQuietHours({ enabled })}
        />
      </div>

      <div class="flex flex-wrap items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Window</span>
        <div class="flex items-center gap-2">
          <input
            type="time"
            value={notificationPrefs.value.quietHours.start}
            aria-label="Quiet hours start"
            onchange={(e) => setQuietHours({ start: e.currentTarget.value })}
            class="h-8 rounded-md bg-bg border border-border px-2 text-[12px] text-fg focus:outline-none focus:border-accent/60"
          />
          <span class="text-[12px] text-fg-subtle">to</span>
          <input
            type="time"
            value={notificationPrefs.value.quietHours.end}
            aria-label="Quiet hours end"
            onchange={(e) => setQuietHours({ end: e.currentTarget.value })}
            class="h-8 rounded-md bg-bg border border-border px-2 text-[12px] text-fg focus:outline-none focus:border-accent/60"
          />
        </div>
      </div>

      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Always interrupt for errors</span>
        <Toggle
          checked={notificationPrefs.value.quietHours.exemptErrors}
          label="Always interrupt for errors"
          onchange={(exemptErrors) => setQuietHours({ exemptErrors })}
        />
      </div>

      <div class="flex flex-wrap items-center justify-between gap-3 py-2.5 last:pb-0">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Manual pause</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">{snoozeLabel()}</p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onclick={pauseForOneHour}
            class="h-8 px-3 rounded-md border border-border text-[12px] text-fg hover:bg-surface-2 transition-colors"
          >
            Pause 1h
          </button>
          <button
            type="button"
            onclick={pauseUntilTomorrow}
            class="h-8 px-3 rounded-md border border-border text-[12px] text-fg hover:bg-surface-2 transition-colors"
          >
            Until tomorrow
          </button>
          <button
            type="button"
            onclick={() => setSnoozeUntil(null)}
            class="h-8 px-3 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
          >
            Clear
          </button>
        </div>
      </div>
    </section>

    <section class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <div class="min-w-0">
          <span class="text-[13px] font-medium text-fg">Sound cues</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Toggle a sound for each agent event independently — a completed card, an
            execution error, or a new comment. Each obeys quiet hours and manual pause.
          </p>
        </div>
        <Toggle
          checked={notificationPrefs.value.sound.volumeFollowsOs}
          label="Follow system volume"
          onchange={(volumeFollowsOs) =>
            notificationPrefs.update((draft) => {
              draft.sound.volumeFollowsOs = volumeFollowsOs;
            })}
        />
      </div>

      {#each AGENT_SOUND_EVENTS as event (event.id)}
        {@const setting = notificationPrefs.value.sound.agentEvents[event.id]}
        <div
          class="flex flex-wrap items-center justify-between gap-3 py-2.5 max-[640px]:items-start"
        >
          <div class="min-w-0">
            <span class="text-[13px] text-fg">{event.label}</span>
            <p class="text-[12px] text-fg-subtle mt-0.5">
              {setting.enabled ? event.description : "Silent"}
            </p>
          </div>
          <div class="flex items-center gap-2">
            <select
              aria-label={`${event.label} sound cue`}
              value={setting.cue}
              onchange={(e) => setAgentEventCue(event.id, e.currentTarget.value as NotificationCue)}
              disabled={!setting.enabled}
              class="h-8 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {#each NOTIFICATION_CUES as cue (cue.id)}
                <option value={cue.id}>{cue.label}</option>
              {/each}
            </select>
            <button
              type="button"
              title="Preview sound"
              aria-label={`Preview ${event.label} sound`}
              onclick={() => previewCue(setting.cue)}
              disabled={!setting.enabled}
              class="inline-flex items-center justify-center w-8 h-8 rounded-md border border-border text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-fg-muted"
            >
              <Icon name="play" size={13} />
            </button>
            <Toggle
              checked={setting.enabled}
              label={`${event.label} sound`}
              onchange={(on) => setAgentEventEnabled(event.id, on)}
            />
          </div>
        </div>
      {/each}
    </section>

    <section class="flex items-center justify-between gap-3 border-t border-border/60 pt-4">
      <div>
        <span class="text-[13px] font-medium text-fg">Defaults</span>
        <p class="text-[12px] text-fg-subtle mt-0.5">
          Restores PortBay's shipped routing and sound mapping.
        </p>
      </div>
      <button
        type="button"
        onclick={reset}
        class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md border text-[12px] transition-colors
               {resetArmed
          ? 'border-status-stopped/50 text-status-stopped hover:bg-status-stopped/10'
          : 'border-border text-fg-muted hover:text-fg hover:bg-surface-2'}"
      >
        <Icon name="rotate-cw" size={12} />
        {resetArmed ? "Confirm reset" : "Reset"}
      </button>
    </section>
  </div>
</SettingsPanel>
