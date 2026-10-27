/**
 * Artwork-independent policy: inject monotonic time and a random roll, then arm
 * one timeout for nextWakeAt. No DOM, input contents, frame data or timers live
 * here. Late wakes advance one phase from now; missed cosmetics never queue.
 */
export const LUCIA_QUIET_MS = 30_000;
export const LUCIA_TWIRL_COOLDOWN_MS = 120_000;
export const LUCIA_SKATEBOARD_COOLDOWN_MS = 600_000;
export const LUCIA_BARK_COOLDOWN_MS = 3_000;
export const LUCIA_READING_DEBOUNCE_MS = 200;

export type LuciaMode = "companion" | "gaze" | "feeding";
export type LuciaFedCount = 0 | 1 | 2 | 3 | 4 | 5;
export type LuciaTimedBehavior =
  | "tailwag"
  | "ear-perk"
  | "twirl"
  | "stretch"
  | "skateboard"
  | "chase"
  | "pickup"
  | "carry-away"
  | "bark"
  | "reading-enter"
  | "reading-exit"
  | "feeding"
  | "full";

export type LuciaBehavior =
  | "inactive"
  | "idle"
  | "bored"
  | "gaze"
  | "reading"
  | "feeding-idle"
  | LuciaTimedBehavior;

export type LuciaBehaviorConfig = {
  /** Supply finite, positive durations from the final asset metadata. */
  durationsMs: Readonly<Record<LuciaTimedBehavior, number>>;
};

export type LuciaBehaviorEvent =
  | {
      type: "environment";
      active: boolean;
      visible: boolean;
      reducedMotion: boolean;
    }
  // Adapters filter actual text entry and aggregate concurrent inference jobs.
  // No keys, text, pointer coordinates, prompts or response content enter here.
  | { type: "typing" }
  | { type: "inference"; pending: boolean }
  | { type: "mode"; mode: LuciaMode }
  | { type: "sock" }
  | { type: "bark" }
  | { type: "feed" }
  | { type: "reset" }
  | { type: "wake" };

export type LuciaBehaviorState = Readonly<{
  behavior: LuciaBehavior;
  mode: LuciaMode;
  fedCount: LuciaFedCount;
  startedAt: number;
  endsAt: number | null;
  nextWakeAt: number | null;
  active: boolean;
  visible: boolean;
  reducedMotion: boolean;
  /** Coarse baseline, reset on scene resume and after a sock run. */
  lastTypingAt: number;
  nextTwirlAt: number | null;
  nextSkateboardAt: number | null;
  nextBarkAt: number;
  inferencePending: boolean;
  /** null while reading, when suspended, or when no inference is pending. */
  readingReadyAt: number | null;
}>;

export function createLuciaBehavior(now: number): LuciaBehaviorState {
  return {
    behavior: "inactive",
    mode: "companion",
    fedCount: 0,
    startedAt: now,
    endsAt: null,
    nextWakeAt: null,
    active: false,
    visible: true,
    reducedMotion: false,
    lastTypingAt: now,
    nextTwirlAt: null,
    nextSkateboardAt: null,
    nextBarkAt: now,
    inferencePending: false,
    readingReadyAt: null,
  };
}

function ownsSock(behavior: LuciaBehavior): boolean {
  return behavior === "chase" || behavior === "pickup" || behavior === "carry-away";
}

function isReading(behavior: LuciaBehavior): boolean {
  return behavior === "reading" || behavior === "reading-enter" || behavior === "reading-exit";
}

function shouldRead(state: LuciaBehaviorState, now: number): boolean {
  return state.inferencePending && (state.readingReadyAt === null || now >= state.readingReadyAt);
}

/** A deadline missed during another action is discarded, never queued. */
function discardMissedCosmetics(state: LuciaBehaviorState, now: number): LuciaBehaviorState {
  return {
    ...state,
    nextTwirlAt:
      state.nextTwirlAt !== null && state.nextTwirlAt <= now
        ? now + LUCIA_TWIRL_COOLDOWN_MS
        : state.nextTwirlAt,
    nextSkateboardAt:
      state.nextSkateboardAt !== null && state.nextSkateboardAt <= now
        ? now + LUCIA_SKATEBOARD_COOLDOWN_MS
        : state.nextSkateboardAt,
  };
}

function earlier(first: number | null, second: number | null): number | null {
  return first === null ? second : second === null ? first : Math.min(first, second);
}

function schedule(state: LuciaBehaviorState): LuciaBehaviorState {
  if (!state.active || !state.visible) return { ...state, nextWakeAt: null };
  let nextWakeAt = state.endsAt;
  if (nextWakeAt === null && !state.reducedMotion) {
    if (state.behavior === "idle") {
      nextWakeAt = state.lastTypingAt + LUCIA_QUIET_MS;
    } else if (state.behavior === "bored") {
      nextWakeAt = earlier(state.nextTwirlAt, state.nextSkateboardAt);
    } else if (state.behavior === "gaze" && state.nextTwirlAt !== null) {
      nextWakeAt = Math.max(state.lastTypingAt + LUCIA_QUIET_MS, state.nextTwirlAt);
    }
  }
  if (state.inferencePending && !ownsSock(state.behavior) && !isReading(state.behavior)) {
    nextWakeAt = earlier(nextWakeAt, state.readingReadyAt);
  }
  return { ...state, nextWakeAt };
}

function begin(
  state: LuciaBehaviorState,
  behavior: LuciaTimedBehavior,
  now: number,
  config: LuciaBehaviorConfig
): LuciaBehaviorState {
  const duration = config.durationsMs[behavior];
  if (!Number.isFinite(duration) || duration <= 0) {
    throw new RangeError(`Lucia ${behavior} duration must be finite and positive`);
  }
  return schedule({
    ...discardMissedCosmetics(state, now),
    behavior,
    startedAt: now,
    endsAt: now + duration,
  });
}

function rest(
  state: LuciaBehaviorState,
  behavior: "idle" | "bored" | "gaze" | "reading" | "feeding-idle",
  now: number
): LuciaBehaviorState {
  return schedule({
    ...discardMissedCosmetics(state, now),
    behavior,
    startedAt: now,
    endsAt: null,
  });
}

function read(state: LuciaBehaviorState, now: number, config: LuciaBehaviorConfig): LuciaBehaviorState {
  const next = { ...state, readingReadyAt: null };
  return state.reducedMotion ? rest(next, "reading", now) : begin(next, "reading-enter", now, config);
}

function settle(state: LuciaBehaviorState, now: number, config: LuciaBehaviorConfig): LuciaBehaviorState {
  if (shouldRead(state, now)) return read(state, now, config);
  if (state.mode === "gaze") return rest(state, "gaze", now);
  if (state.mode === "feeding") return rest(state, "feeding-idle", now);
  return !state.reducedMotion && now - state.lastTypingAt >= LUCIA_QUIET_MS
    ? begin(state, "stretch", now, config)
    : rest(state, "idle", now);
}

/**
 * Priority: sock ownership, real inference, explicit interactions, ambient
 * motion. Reading waits for an owned sock; a new explicit sock may interrupt
 * reading. Feeding suppresses socks and ambient motion. Modes persist across
 * reading; feeding counts survive hiding but reset on mode exit or deactivation.
 *
 * Reduced motion uses static reading, feeding, bark and sock poses; the renderer
 * must suppress their animation. All unsolicited motion and its deadlines stop.
 * roll is in [0, 1). Only quiet attempt deadlines consult it: <0.1 chooses a due
 * skateboard, otherwise <0.25 a due twirl. Misses consume the attempt too.
 */
export function reduceLuciaBehavior(
  state: LuciaBehaviorState,
  event: LuciaBehaviorEvent,
  now: number,
  config: LuciaBehaviorConfig,
  roll = 0.5
): LuciaBehaviorState {
  if (event.type === "environment") {
    if (
      state.active === event.active &&
      state.visible === event.visible &&
      state.reducedMotion === event.reducedMotion
    ) return state;
    const environment = {
      active: event.active,
      visible: event.visible,
      reducedMotion: event.reducedMotion,
    };
    if (!event.active || !event.visible) {
      return {
        ...createLuciaBehavior(now),
        ...environment,
        mode: state.mode,
        fedCount: event.active ? state.fedCount : 0,
        inferencePending: state.inferencePending,
        nextBarkAt: state.nextBarkAt,
      };
    }
    const resumed = !state.active || !state.visible;
    const fresh = {
      ...state,
      ...environment,
      lastTypingAt: now,
      nextTwirlAt: event.reducedMotion ? null : now + LUCIA_TWIRL_COOLDOWN_MS,
      nextSkateboardAt: event.reducedMotion ? null : now + LUCIA_SKATEBOARD_COOLDOWN_MS,
      readingReadyAt: resumed && state.inferencePending ? now + LUCIA_READING_DEBOUNCE_MS : state.readingReadyAt,
    };
    if (!resumed && ownsSock(state.behavior)) return schedule(fresh);
    if (!resumed && isReading(state.behavior) && state.inferencePending) {
      return rest({ ...fresh, readingReadyAt: null }, "reading", now);
    }
    return resumed && !event.reducedMotion && state.mode !== "feeding"
      ? begin(fresh, "tailwag", now, config)
      : settle(fresh, now, config);
  }

  // Inference remains external truth even while the scene is hidden or disabled.
  if (event.type === "inference") {
    if (state.inferencePending === event.pending) return state;
    const next = {
      ...state,
      inferencePending: event.pending,
      readingReadyAt: event.pending && state.active && state.visible ? now + LUCIA_READING_DEBOUNCE_MS : null,
    };
    if (!state.active || !state.visible) return next;
    if (event.pending && state.behavior === "reading-exit") {
      return rest({ ...next, readingReadyAt: null }, "reading", now);
    }
    if (!event.pending && isReading(state.behavior)) {
      return state.reducedMotion ? settle(next, now, config) : begin(next, "reading-exit", now, config);
    }
    return schedule(next);
  }

  if (event.type === "mode") {
    if (event.mode === state.mode) return state;
    const next = { ...state, mode: event.mode, fedCount: 0 as const, lastTypingAt: now };
    if (!state.active || !state.visible || isReading(state.behavior)) return next;
    if (ownsSock(state.behavior) && event.mode !== "feeding") return next;
    return settle(next, now, config);
  }

  if (!state.active || !state.visible) return state;

  if (event.type === "reset") {
    if (state.mode !== "feeding") return state;
    const next = { ...state, fedCount: 0 as const };
    return isReading(state.behavior) ? next : settle(next, now, config);
  }

  if (event.type === "sock") {
    return state.mode === "feeding" || ownsSock(state.behavior)
      ? state
      : begin({ ...state, lastTypingAt: now }, "chase", now, config);
  }

  if (event.type === "typing") {
    const wasQuiet = now - state.lastTypingAt >= LUCIA_QUIET_MS;
    const next = { ...state, lastTypingAt: now };
    // Suppression consumes this activity: ear perks never replay after a task.
    if (ownsSock(state.behavior) || isReading(state.behavior)) return schedule(next);
    if (shouldRead(next, now)) return read(next, now, config);
    if (state.reducedMotion || state.mode === "feeding" || state.behavior === "bark") return schedule(next);
    if (wasQuiet && state.behavior !== "ear-perk") return begin(next, "ear-perk", now, config);
    if (
      state.behavior === "bored" || state.behavior === "stretch" ||
      state.behavior === "twirl" || state.behavior === "skateboard"
    ) return settle(next, now, config);
    return schedule(next);
  }

  if (event.type === "bark" || event.type === "feed") {
    if (ownsSock(state.behavior) || isReading(state.behavior)) return state;
    if (shouldRead(state, now)) return read(state, now, config);
    if (event.type === "bark") {
      if (state.mode === "feeding" || state.behavior === "bark" || now < state.nextBarkAt) return state;
      return begin({ ...state, nextBarkAt: now + LUCIA_BARK_COOLDOWN_MS }, "bark", now, config);
    }
    if (state.mode !== "feeding" || state.behavior !== "feeding-idle") return state;
    return state.fedCount === 5
      ? begin(state, "full", now, config)
      : begin({ ...state, fedCount: (state.fedCount + 1) as LuciaFedCount }, "feeding", now, config);
  }

  if (state.nextWakeAt === null || now < state.nextWakeAt) return state;
  if (!ownsSock(state.behavior) && !isReading(state.behavior) && shouldRead(state, now)) {
    return read(state, now, config);
  }

  switch (state.behavior) {
    case "chase":
      return begin(state, "pickup", now, config);
    case "pickup":
      return begin(state, "carry-away", now, config);
    case "carry-away":
      return settle({ ...state, lastTypingAt: now }, now, config);
    case "tailwag":
    case "ear-perk":
    case "bark":
    case "reading-exit":
    case "feeding":
    case "full":
    case "twirl":
    case "skateboard":
      return settle(state, now, config);
    case "idle":
      return begin(state, "stretch", now, config);
    case "stretch":
      return rest(state, "bored", now);
    case "reading-enter":
      return rest(state, "reading", now);
    case "bored":
    case "gaze": {
      const skateboardDue = state.mode === "companion" && state.nextSkateboardAt !== null && now >= state.nextSkateboardAt;
      const twirlDue = state.nextTwirlAt !== null && now >= state.nextTwirlAt;
      const next = discardMissedCosmetics(state, now);
      if (skateboardDue && roll < 0.1) {
        // Keep a twirl well away from this larger surprise traversal.
        return begin({ ...next, nextTwirlAt: now + LUCIA_TWIRL_COOLDOWN_MS }, "skateboard", now, config);
      }
      if (twirlDue && roll < 0.25) return begin(next, "twirl", now, config);
      return schedule(next);
    }
    case "inactive":
    case "reading":
    case "feeding-idle":
      return state;
  }
}
