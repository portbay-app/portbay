<script setup lang="ts">
/**
 * "Was this helpful?" feedback widget for docs pages (mounted via the default
 * theme's `doc-after` slot in theme/index.ts, so it never renders on the
 * `layout: home` landing page).
 *
 * Floats fixed at the bottom of the viewport and fades in once the reader has
 * scrolled 70% of the page (pages too short to scroll count as fully read).
 * Collapsed: a pill with four rating faces. Picking one expands a textarea;
 * Send POSTs to the portbay-cloud Worker, which relays the message by email.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useData, useRoute } from "vitepress";

const ENDPOINT = "https://cloud.portbay.app/feedback/docs";
const SCROLL_THRESHOLD = 0.7;

const RATINGS = [
  { id: "very-sad", label: "Terrible" },
  { id: "sad", label: "Bad" },
  { id: "neutral", label: "Okay" },
  { id: "happy", label: "Amazing" },
] as const;

const { page } = useData();
const route = useRoute();

const value = ref("");
const feedback = ref("");
const submitting = ref(false);
const sent = ref(false);
const error = ref("");
const textareaEl = ref<HTMLTextAreaElement | null>(null);

const expanded = computed(() => value.value !== "");

const scrolledEnough = ref(false);
// Stay visible while the reader is mid-interaction, even if they scroll back up.
const visible = computed(
  () => scrolledEnough.value || expanded.value || submitting.value,
);

function measureScroll() {
  const max = document.documentElement.scrollHeight - window.innerHeight;
  scrolledEnough.value = max <= 0 || window.scrollY / max >= SCROLL_THRESHOLD;
}

onMounted(() => {
  window.addEventListener("scroll", measureScroll, { passive: true });
  window.addEventListener("resize", measureScroll, { passive: true });
  measureScroll();
});

onBeforeUnmount(() => {
  window.removeEventListener("scroll", measureScroll);
  window.removeEventListener("resize", measureScroll);
});

// Fresh state per page: clear any draft and re-measure once the new content
// has settled (client-side navigation does not remount this component).
watch(
  () => route.path,
  () => {
    value.value = "";
    feedback.value = "";
    sent.value = false;
    error.value = "";
    scrolledEnough.value = false;
    nextTick(measureScroll);
  },
);

function pick(id: string) {
  error.value = "";
  sent.value = false;
  if (value.value === id) {
    value.value = "";
    return;
  }
  value.value = id;
  nextTick(() => textareaEl.value?.focus());
}

async function send() {
  if (!feedback.value.trim() || submitting.value) return;
  submitting.value = true;
  error.value = "";
  try {
    const res = await fetch(ENDPOINT, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        rating: value.value,
        feedback: feedback.value.trim(),
        page: typeof location !== "undefined" ? location.pathname : "",
        title: page.value.title,
        website: "", // honeypot — must stay empty
      }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    value.value = "";
    feedback.value = "";
    sent.value = true;
  } catch {
    error.value = "Could not send feedback — please try again.";
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="fbw-wrap" :class="{ visible }" :inert="!visible">
    <div class="fbw" :class="{ expanded }">
      <div class="fbw-row">
        <span class="fbw-label">{{ sent ? "Thanks for your feedback!" : "Was this helpful?" }}</span>
        <div class="fbw-faces" role="group" aria-label="Rate this page">
          <button
            v-for="r in RATINGS"
            :key="r.id"
            type="button"
            class="fbw-face"
            :class="{ active: value === r.id }"
            :title="r.label"
            :aria-pressed="value === r.id"
            @click="pick(r.id)"
          >
            <!-- very-sad -->
            <svg v-if="r.id === 'very-sad'" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <path d="M16 16s-1.5-2-4-2-4 2-4 2" />
              <path d="M9 9h.01" />
              <path d="M15 9h.01" />
              <path d="M9 13v2" class="fbw-tear" />
              <path d="M15 13v2" class="fbw-tear" />
            </svg>
            <!-- sad -->
            <svg v-else-if="r.id === 'sad'" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <path d="M16 16s-1.5-2-4-2-4 2-4 2" />
              <line x1="9" y1="9" x2="9.01" y2="9" />
              <line x1="15" y1="9" x2="15.01" y2="9" />
            </svg>
            <!-- neutral -->
            <svg v-else-if="r.id === 'neutral'" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <path d="M8 13s1.5 2 4 2 4-2 4-2" />
              <line x1="9" y1="9" x2="9.01" y2="9" />
              <line x1="15" y1="9" x2="15.01" y2="9" />
            </svg>
            <!-- happy -->
            <svg v-else width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <path d="M8 13s1.5 2 4 2 4-2 4-2" />
              <path d="M9 9l.5 1.5l1.5 .5l-1.5 .5l-.5 1.5l-.5-1.5l-1.5-.5l1.5-.5z" class="fbw-spark" stroke="none" />
              <path d="M15 9l.5 1.5l1.5 .5l-1.5 .5l-.5 1.5l-.5-1.5l-1.5-.5l1.5-.5z" class="fbw-spark" stroke="none" />
            </svg>
          </button>
        </div>
      </div>

      <div class="fbw-expand" :class="{ open: expanded }">
        <div class="fbw-expand-inner">
          <div class="fbw-body">
            <span class="fbw-eyebrow">Feedback</span>
            <textarea
              ref="textareaEl"
              v-model="feedback"
              class="fbw-textarea"
              placeholder="What worked, what's missing, what's wrong?"
              rows="5"
              maxlength="4000"
            ></textarea>
            <div class="fbw-footer">
              <p class="fbw-note">{{ error || "We appreciate your input." }}</p>
              <button
                type="button"
                class="fbw-send"
                :disabled="!feedback.trim() || submitting"
                @click="send"
              >
                <span v-if="submitting" class="fbw-spinner" aria-label="Sending"></span>
                <template v-else>Send Feedback</template>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fbw-wrap {
  position: fixed;
  bottom: 24px;
  left: 50%;
  z-index: var(--vp-z-index-nav, 30);
  width: min(420px, calc(100vw - 32px));
  display: flex;
  justify-content: center;
  opacity: 0;
  transform: translate(-50%, 16px);
  pointer-events: none;
  transition:
    opacity 0.3s ease,
    transform 0.35s cubic-bezier(0.32, 0.72, 0, 1);
}

.fbw-wrap.visible {
  opacity: 1;
  transform: translate(-50%, 0);
  pointer-events: auto;
}

.fbw {
  width: fit-content;
  max-width: 100%;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-elv, var(--vp-c-bg));
  border-radius: 9999px;
  padding: 6px 14px;
  box-shadow: 0 16px 40px -16px rgba(0, 0, 0, 0.25);
  transition:
    border-radius 0.35s cubic-bezier(0.32, 0.72, 0, 1),
    width 0.35s cubic-bezier(0.32, 0.72, 0, 1);
  /* Lets `width: fit-content -> 100%` animate in browsers that support it;
     elsewhere the size change simply snaps. */
  interpolate-size: allow-keywords;
}

.fbw.expanded {
  width: 100%;
  border-radius: 24px;
}

.fbw-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
}

.fbw-label {
  margin-left: 6px;
  font-size: 14px;
  font-weight: 500;
  color: var(--vp-c-text-2);
  white-space: nowrap;
  user-select: none;
}

.fbw-faces {
  display: flex;
  align-items: center;
  gap: 4px;
}

.fbw-face {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 7px;
  border-radius: 9999px;
  color: var(--vp-c-text-3);
  transition: color 0.2s, background-color 0.2s, transform 0.15s;
}

.fbw-face:hover {
  color: var(--vp-c-text-1);
  background: var(--vp-c-default-soft);
}

.fbw-face:active {
  transform: scale(0.88);
}

.fbw-face.active {
  color: #fff;
  background: var(--vp-c-brand-1);
}

.fbw-face:focus-visible {
  outline: 2px solid var(--vp-c-brand-1);
  outline-offset: 1px;
}

.fbw-tear {
  stroke: #3b82f6;
}

.fbw-face.active .fbw-tear {
  stroke: #fff;
}

.fbw-spark {
  fill: #f97316;
}

.fbw-face.active .fbw-spark {
  fill: #fff;
}

/* Height-auto expansion via the 0fr -> 1fr grid trick. */
.fbw-expand {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.35s cubic-bezier(0.32, 0.72, 0, 1);
}

.fbw-expand.open {
  grid-template-rows: 1fr;
}

.fbw-expand-inner {
  overflow: hidden;
}

.fbw-body {
  padding: 14px 2px 8px;
}

.fbw-eyebrow {
  display: block;
  margin-bottom: 8px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--vp-c-text-3);
  user-select: none;
}

.fbw-textarea {
  width: 100%;
  resize: none;
  border: 1px solid var(--vp-c-divider);
  border-radius: 14px;
  background: var(--vp-c-bg-soft);
  padding: 12px 14px;
  font-family: inherit;
  font-size: 14px;
  line-height: 1.6;
  color: var(--vp-c-text-1);
  transition: border-color 0.2s;
}

.fbw-textarea:focus {
  outline: none;
  border-color: var(--vp-c-brand-1);
}

.fbw-textarea::placeholder {
  color: var(--vp-c-text-3);
}

.fbw-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 10px;
  padding-top: 12px;
  border-top: 1px solid var(--vp-c-divider);
}

.fbw-note {
  margin: 0;
  font-size: 11px;
  font-weight: 500;
  color: var(--vp-c-text-3);
}

.fbw-send {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 120px;
  border-radius: 12px;
  background: var(--vp-c-brand-1);
  color: #fff;
  padding: 7px 18px;
  font-size: 13px;
  font-weight: 600;
  transition: background-color 0.2s, transform 0.15s, opacity 0.2s;
}

.fbw-send:hover:not(:disabled) {
  background: var(--vp-c-brand-2);
}

.fbw-send:active:not(:disabled) {
  transform: scale(0.95);
}

.fbw-send:disabled {
  opacity: 0.35;
  pointer-events: none;
}

.fbw-spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border-radius: 9999px;
  border: 2px solid rgba(255, 255, 255, 0.25);
  border-top-color: #fff;
  animation: fbw-spin 0.8s linear infinite;
}

@keyframes fbw-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
