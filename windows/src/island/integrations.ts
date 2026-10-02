// Integration events → island state. Port of the `handle…` methods in the Swift
// pollers: a genuinely new item flips the pill to finished/error, badges it when
// the pill isn't focused, plays a sound, and clears itself after 60 s.

import { onEvent, Bridge, type IntegrationUpdate } from "../core/bridge";
import { Sound } from "../core/sound";
import { KEYLESS_INTEGRATION_IDS, State } from "../core/state";
import type { Island } from "./island";

/** Which Credential Manager key backs each pill. */
const KEY_FOR: Record<string, string> = {
  integration_stripe: "stripe-api-key",
  integration_github: "github-token",
  integration_vercel: "vercel-token",
  integration_n8n: "n8n-api-key",
  integration_resend: "resend-api-key",
  integration_notion: "notion-api-key",
  integration_calcom: "calcom-api-key",
};

const clearTimers = new Map<string, number>();

// Mood-reaction bookkeeping — sustained/edge state that doesn't fit the
// per-poll `event` shape above (CPU needs to stay hot a while, a PR list
// needs diffing against what was there last time).
const now = () => performance.now() / 1000;
const CPU_HOT_THRESHOLD = 85;
const CPU_HOT_SUSTAIN_S = 8;
const LOW_BATTERY_PERCENT = 15;
const LOW_BATTERY_YAWN_COOLDOWN_S = 45;
let hotSince: number | null = null;
let wasCharging: boolean | null = null;
let lastLowBatteryYawn = 0;
let seenPrIds: Set<string> | null = null;

export function registerIntegrationHandlers(island: Island) {
  void onEvent<IntegrationUpdate>("integration", (update) => handle(island, update));
  void refreshConfigured();
}

/** Asks Rust which keys exist so the idle cards can say so. */
export async function refreshConfigured() {
  for (const [id, key] of Object.entries(KEY_FOR)) {
    const present = (await Bridge.secretPresent(key)) ?? false;
    const info = State.integrations[id] ?? { data: {}, error: null, loaded: false, configured: false };
    State.integrations[id] = { ...info, configured: present };
  }
  const hooks = State.settings.hooksInstalled;
  const claude = State.integrations.integration_claude ?? {
    data: {}, error: null, loaded: false, configured: false,
  };
  State.integrations.integration_claude = { ...claude, configured: hooks };
  // System vitals and now-playing read local OS state, not a stored key.
  for (const id of KEYLESS_INTEGRATION_IDS) {
    const info = State.integrations[id] ?? { data: {}, error: null, loaded: false, configured: false };
    State.integrations[id] = { ...info, configured: true };
  }
  State.notify();
}

function handle(island: Island, update: IntegrationUpdate) {
  if (State.paused) return;

  const previous = State.integrations[update.id];
  State.integrations[update.id] = {
    data: update.error ? (previous?.data ?? {}) : update.data,
    error: update.error,
    loaded: update.error ? (previous?.loaded ?? false) : true,
    configured: previous?.configured ?? true,
  };

  const event = update.event;
  if (event) {
    const task = State.tasks.find((t) => t.id === update.id);
    if (task) {
      task.state = event.success ? "finished" : "error";
      task.steps = event.detail ? [event.label, event.detail] : [event.label];
      task.stepIndex = task.steps.length - 1;
      if (State.focusId !== update.id) {
        task.pillBadge = event.success ? "finished" : "error";
      }
      Sound.play(event.success ? "finish" : "error");
      // Same as the Swift pollers: show the compact island so the badge is seen,
      // but never steal the screen for a successful deploy.
      island.reveal();
      if (update.id === "integration_github") {
        island.playEmote(event.success ? "proud" : "annoyed");
      }

      const existing = clearTimers.get(update.id);
      if (existing != null) window.clearTimeout(existing);
      clearTimers.set(
        update.id,
        window.setTimeout(() => {
          clearTimers.delete(update.id);
          const t = State.tasks.find((x) => x.id === update.id);
          if (!t || (t.state !== "finished" && t.state !== "error")) return;
          t.state = "idle";
          t.steps = [];
          t.stepIndex = 0;
          t.pillBadge = null;
          State.notify();
        }, 60_000),
      );
    }
  }

  if (update.id === "integration_github" && !update.error) {
    reactToPullRequests(island, update.data.pullRequests);
  }
  if (update.id === "integration_vitals" && !update.error) {
    reactToVitals(island, update.data);
  }

  State.notify();
}

/** A PR waiting on your review is the one thing here that actually needs you —
 *  worth a look up from Wato, silent on the first load like every other
 *  "is this new" check in this app. */
function reactToPullRequests(island: Island, raw: unknown) {
  const list = Array.isArray(raw) ? (raw as Record<string, unknown>[]) : [];
  const ids = new Set(list.map((pr) => String(pr.id ?? "")));
  if (seenPrIds && [...ids].some((id) => id && !seenPrIds!.has(id))) {
    island.playEmote("surprised");
  }
  seenPrIds = ids;
}

/** CPU pegged for a while, or the battery situation changing — ambient mood,
 *  not an alert, so no sound and no badge. */
function reactToVitals(island: Island, raw: unknown) {
  const d = raw as Record<string, unknown>;
  const t = now();

  const cpu = Number(d.cpuPercent ?? 0);
  if (cpu >= CPU_HOT_THRESHOLD) {
    if (hotSince == null) hotSince = t;
  } else {
    hotSince = null;
  }
  island.setHot(hotSince != null && t - hotSince > CPU_HOT_SUSTAIN_S);

  const charging = d.batteryCharging as boolean | null | undefined;
  const battery = d.batteryPercent as number | null | undefined;
  if (charging != null) {
    if (wasCharging === false && charging === true) island.playEmote("happy");
    wasCharging = charging;
  }
  if (
    battery != null && battery < LOW_BATTERY_PERCENT && charging === false &&
    t - lastLowBatteryYawn > LOW_BATTERY_YAWN_COOLDOWN_S
  ) {
    lastLowBatteryYawn = t;
    island.playEmote("yawn", 2.4);
  }
}
