/**
 * Artwork-independent policy: inject monotonic time and a random roll, then arm
 * one timeout for nextWakeAt. No DOM, input contents, frame data or timers live
 * here. Late wakes advance one phase from now; missed cosmetics never queue.
 */
export const LUCIA_QUIET_MS = 30_000;
export const LUCIA_TWIRL_COOLDOWN_MS = 120_000;
// Under the shortest common auto-lock (5 minutes), so a skate attempt comes up while she is on screen.
export const LUCIA_SKATEBOARD_COOLDOWN_MS = 180_000;
export const LUCIA_SKATEBOARD_CHANCE = 0.5;
export const LUCIA_TWIRL_CHANCE = 0.25;
export const LUCIA_READING_DEBOUNCE_MS = 200;
/** Petting: the first two clicks bark; a further click within the window plays a random fun animation. */
export const LUCIA_PET_BARKS = 2;
export const LUCIA_PET_BARK_WINDOW_MS = 5_000;
export const LUCIA_PET_FUN_WINDOW_MS = 30_000;

export type LuciaMode = "companion" | "gaze" | "feeding";
export type LuciaFedCount = 0 | 1 | 2 | 3 | 4 | 5;
export type LuciaTimedBehavior =
  | "tailwag"
  | "ear-perk"
  | "twirl"
  | "stretch"
  | "board-chase"
  | "skateboard"
  | "chase"
  | "pickup"
  | "carry-away"
  | "bark"
  | "reading-enter"
  | "reading-play"
  | "reading-exit"
  | "feeding"
  | "full";

/** Explicit local-demo requests use the same action ownership and completion path. */
export type LuciaPreviewBehavior = "tailwag" | "stretch" | "ear-perk" | "twirl" | "skateboard";

/** What a pet past the barks can start; a skate begins with the board rolling in, reading with the paper. */
export const LUCIA_FUN_POOL = ["twirl", "skateboard", "reading", "tailwag", "ear-perk", "stretch"] as const;
export type LuciaFunBehavior = (typeof LUCIA_FUN_POOL)[number];

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
  | { type: "pet" }
  | { type: "feed" }
  | { type: "reset" }
  | { type: "preview"; behavior: LuciaPreviewBehavior }
  | { type: "wake" };

export type LuciaBehaviorState = Readonly<{
  behavior: LuciaBehavior;
  mode: LuciaMode;
  /** Completed fruits only; interruption cancels an uncommitted feed. */
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
  /** Clicks so far in the current pet chain; 0 when there is none. */
  petStep: number;
  /** When the next pet stops continuing the chain; null while a pet reaction plays or with no chain. */
  petWindowEndsAt: number | null;
  /** A pet reaction is playing; clicks are ignored until she is back at rest. */
  petReacting: boolean;
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
    petStep: 0,
    petWindowEndsAt: null,
    petReacting: false,
    inferencePending: false,
    readingReadyAt: null,
  };
}

/** The sock run owns Lucia from chase through carry-away; the UI and policy share this test. */
export function ownsSock(behavior: LuciaBehavior): boolean {
  return behavior === "chase" || behavior === "pickup" || behavior === "carry-away";
}

export function isReading(behavior: LuciaBehavior): boolean {
  return behavior === "reading" || behavior === "reading-enter" || behavior === "reading-play" || behavior === "reading-exit";
}

function endPetChain(state: LuciaBehaviorState): LuciaBehaviorState {
  return state.petStep === 0 && !state.petReacting ? state : { ...state, petStep: 0, petWindowEndsAt: null, petReacting: false };
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
  // A pet reaction is over once she is back at rest; only then does the next click's window open.
  const settled = state.petReacting && behavior !== "reading"
    ? {
        ...state,
        petReacting: false,
        petWindowEndsAt: now + (state.petStep === 1 ? LUCIA_PET_BARK_WINDOW_MS : LUCIA_PET_FUN_WINDOW_MS),
      }
    : state;
  return schedule({
    ...discardMissedCosmetics(settled, now),
    behavior,
    startedAt: now,
    endsAt: null,
  });
}

function read(state: LuciaBehaviorState, now: number, config: LuciaBehaviorConfig): LuciaBehaviorState {
  const next = { ...endPetChain(state), readingReadyAt: null };
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

/** Start an explicitly requested animation; multi-phase ones begin with their first phase. */
function play(state: LuciaBehaviorState, behavior: LuciaFunBehavior, now: number, config: LuciaBehaviorConfig): LuciaBehaviorState {
  if (behavior === "skateboard") return begin(state, "board-chase", now, config);
  if (behavior === "reading") return begin(state, "reading-enter", now, config);
  // Playing a typing reaction has the same quiet reset as actual typing.
  return begin(behavior === "ear-perk" ? { ...state, lastTypingAt: now } : state, behavior, now, config);
}

/**
 * Priority: sock ownership, real inference, explicit interactions, ambient
 * motion. Reading waits for an owned sock; a new explicit sock may interrupt
 * reading. Feeding suppresses socks and ambient motion. Modes persist across
 * reading; feeding counts survive hiding but reset on mode exit or deactivation.
 *
 * Reduced motion uses static reading, feeding, bark and sock poses; the renderer
 * must suppress their animation. All unsolicited motion and its deadlines stop.
 * roll is in [0, 1). Quiet attempt deadlines consult it: <LUCIA_SKATEBOARD_CHANCE
 * chooses a due skateboard, and the rescaled remainder <LUCIA_TWIRL_CHANCE a due
 * twirl. Misses consume the attempt too. A pet past the barks uses it to pick
 * from LUCIA_FUN_POOL.
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
        // Play deadlines survive a blur, or frequent window switching would postpone them forever.
        nextTwirlAt: state.nextTwirlAt,
        nextSkateboardAt: state.nextSkateboardAt,
      };
    }
    const resumed = !state.active || !state.visible;
    // Keep an existing deadline; one that lapsed while away comes up after the next quiet spell.
    const deadline = (current: number | null, cooldownMs: number) => event.reducedMotion
      ? null
      : current === null ? now + cooldownMs : current <= now ? now + LUCIA_QUIET_MS : current;
    const fresh = {
      ...state,
      ...environment,
      lastTypingAt: now,
      nextTwirlAt: deadline(state.nextTwirlAt, LUCIA_TWIRL_COOLDOWN_MS),
      nextSkateboardAt: deadline(state.nextSkateboardAt, LUCIA_SKATEBOARD_COOLDOWN_MS),
      readingReadyAt: resumed && state.inferencePending ? now + LUCIA_READING_DEBOUNCE_MS : state.readingReadyAt,
    };
    if (!resumed && ownsSock(state.behavior)) return schedule(fresh);
    if (!resumed && isReading(state.behavior) && state.inferencePending) {
      return rest({ ...fresh, readingReadyAt: null }, "reading", now);
    }
    // A motion-preference change lets a deliberate action finish (a feed in progress still
    // counts); turning reduced motion on still stops unsolicited cosmetics at once.
    const deliberate = state.behavior === "feeding" || state.behavior === "full" || state.behavior === "bark";
    if (!resumed && state.endsAt !== null && (deliberate || !event.reducedMotion)) return schedule(fresh);
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
    // Real inference takes over a requested read, or one that is putting the paper away.
    if (event.pending && (state.behavior === "reading-exit" || state.behavior === "reading-play")) {
      return rest({ ...endPetChain(next), readingReadyAt: null }, "reading", now);
    }
    if (!event.pending && isReading(state.behavior)) {
      return state.reducedMotion ? settle(next, now, config) : begin(next, "reading-exit", now, config);
    }
    return schedule(next);
  }

  if (event.type === "mode") {
    if (event.mode === state.mode) return state;
    const next = { ...endPetChain(state), mode: event.mode, fedCount: 0 as const, lastTypingAt: now };
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

  if (event.type === "preview") {
    if (state.mode !== "companion" || state.reducedMotion || state.inferencePending
      || ownsSock(state.behavior) || isReading(state.behavior) || state.behavior === "bark") return state;
    return play(endPetChain(state), event.behavior, now, config);
  }

  if (event.type === "sock") {
    return state.mode === "feeding" || ownsSock(state.behavior)
      ? state
      : begin({ ...endPetChain(state), lastTypingAt: now }, "chase", now, config);
  }

  if (event.type === "typing") {
    const wasQuiet = now - state.lastTypingAt >= LUCIA_QUIET_MS;
    const next = { ...endPetChain(state), lastTypingAt: now };
    // A requested read is put away when she is typed at; real inference keeps the paper up.
    if ((state.behavior === "reading-play" || state.behavior === "reading-enter") && !state.inferencePending) {
      return begin(next, "reading-exit", now, config);
    }
    // Suppression consumes this activity: ear perks never replay after a task.
    if (ownsSock(state.behavior) || isReading(state.behavior)) return schedule(next);
    if (shouldRead(next, now)) return read(next, now, config);
    if (state.reducedMotion || state.mode === "feeding" || state.behavior === "bark") return schedule(next);
    if (wasQuiet && state.behavior !== "ear-perk") return begin(next, "ear-perk", now, config);
    if (
      state.behavior === "bored" || state.behavior === "stretch" ||
      state.behavior === "twirl" || state.behavior === "board-chase" || state.behavior === "skateboard"
    ) return settle(next, now, config);
    return schedule(next);
  }

  if (event.type === "pet" || event.type === "feed") {
    if (ownsSock(state.behavior) || isReading(state.behavior)) return state;
    if (shouldRead(state, now)) return read(state, now, config);
    if (event.type === "pet") {
      if (state.mode === "feeding" || state.petReacting) return state;
      const continues = state.petWindowEndsAt !== null && now <= state.petWindowEndsAt;
      const petStep = continues ? Math.min(state.petStep + 1, LUCIA_PET_BARKS + 1) : 1;
      // Petting counts as activity, so finishing a reaction rests instead of starting a stretch.
      const next = { ...state, petStep, petWindowEndsAt: null, petReacting: true, lastTypingAt: now };
      if (petStep <= LUCIA_PET_BARKS || state.reducedMotion) return begin(next, "bark", now, config);
      const fun = LUCIA_FUN_POOL[Math.min(LUCIA_FUN_POOL.length - 1, Math.floor(roll * LUCIA_FUN_POOL.length))];
      return play(next, fun, now, config);
    }
    if (state.mode !== "feeding" || state.behavior !== "feeding-idle" || state.inferencePending) return state;
    return state.fedCount === 5 ? state : begin(state, "feeding", now, config);
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
    case "full":
    case "twirl":
    case "skateboard":
      return settle(state, now, config);
    case "board-chase":
      return begin(state, "skateboard", now, config);
    case "reading-play":
      return state.inferencePending ? rest(state, "reading", now) : begin(state, "reading-exit", now, config);
    case "feeding": {
      const next = { ...state, fedCount: Math.min(5, state.fedCount + 1) as LuciaFedCount };
      return next.fedCount === 5 ? begin(next, "full", now, config) : settle(next, now, config);
    }
    case "idle":
      return begin(state, "stretch", now, config);
    case "stretch":
      return rest(state, "bored", now);
    case "reading-enter":
      // Inference holds the paper up until it finishes; a requested read turns a few pages.
      return state.inferencePending ? rest(state, "reading", now) : begin(state, "reading-play", now, config);
    case "bored":
    case "gaze": {
      const skateboardDue = state.mode === "companion" && state.nextSkateboardAt !== null && now >= state.nextSkateboardAt;
      const twirlDue = state.nextTwirlAt !== null && now >= state.nextTwirlAt;
      const next = discardMissedCosmetics(state, now);
      if (skateboardDue && roll < LUCIA_SKATEBOARD_CHANCE) {
        // Keep a twirl well away from this larger surprise traversal.
        return play({ ...next, nextTwirlAt: now + LUCIA_TWIRL_COOLDOWN_MS }, "skateboard", now, config);
      }
      // A missed skate roll is rescaled, so a due twirl keeps its own chance.
      const twirlRoll = skateboardDue ? (roll - LUCIA_SKATEBOARD_CHANCE) / (1 - LUCIA_SKATEBOARD_CHANCE) : roll;
      if (twirlDue && twirlRoll < LUCIA_TWIRL_CHANCE) return begin(next, "twirl", now, config);
      return schedule(next);
    }
    case "inactive":
    case "reading":
    case "feeding-idle":
      return state;
  }
}
