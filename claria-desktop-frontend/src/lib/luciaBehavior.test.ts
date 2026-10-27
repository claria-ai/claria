import { describe, expect, it } from "vitest";
import {
  createLuciaBehavior,
  reduceLuciaBehavior,
  type LuciaBehaviorConfig,
  type LuciaBehaviorEvent,
  type LuciaBehaviorState,
  type LuciaMode,
} from "./luciaBehavior";

// Deliberately arbitrary, distinct test durations; these are not asset defaults.
const CONFIG: LuciaBehaviorConfig = {
  durationsMs: {
    tailwag: 10,
    "ear-perk": 20,
    twirl: 30,
    stretch: 40,
    skateboard: 50,
    chase: 60,
    pickup: 70,
    "carry-away": 80,
    bark: 90,
    "reading-enter": 100,
    "reading-exit": 110,
    feeding: 120,
    full: 130,
  },
};
const ENVIRONMENT = { type: "environment", active: true, visible: true, reducedMotion: false } as const;

function step(
  state: LuciaBehaviorState,
  event: LuciaBehaviorEvent,
  now: number,
  roll = 0.5,
  config = CONFIG
): LuciaBehaviorState {
  return reduceLuciaBehavior(state, event, now, config, roll);
}

function wake(state: LuciaBehaviorState, now = state.nextWakeAt!, roll = 0.5): LuciaBehaviorState {
  expect(state.nextWakeAt).not.toBeNull();
  return step(state, { type: "wake" }, now, roll);
}

function active(mode: LuciaMode = "companion", reducedMotion = false): LuciaBehaviorState {
  const state = step(createLuciaBehavior(0), { ...ENVIRONMENT, reducedMotion }, 0);
  return mode === "companion" ? state : step(state, { type: "mode", mode }, 0);
}

function idle(): LuciaBehaviorState {
  return wake(active());
}

function bored(): LuciaBehaviorState {
  return wake(wake(idle()));
}

function reading(mode: LuciaMode = "companion", reducedMotion = false): LuciaBehaviorState {
  const state = step(active(mode, reducedMotion), { type: "inference", pending: true }, 10);
  const entering = wake(state, 210);
  return reducedMotion ? entering : wake(entering);
}

function finishSock(state: LuciaBehaviorState): LuciaBehaviorState {
  expect(state.behavior).toBe("chase");
  const pickup = wake(state);
  expect(pickup.behavior).toBe("pickup");
  const carry = wake(pickup);
  expect(carry.behavior).toBe("carry-away");
  return wake(carry);
}

describe("Lucia quiet activity", () => {
  it("starts inactive without a timer or input side effects", () => {
    const state = Object.freeze(createLuciaBehavior(0));
    for (const type of ["typing", "sock", "bark", "feed", "wake"] as const) {
      expect(step(state, { type }, 1_000_000, 0)).toBe(state);
    }
    expect(state.nextWakeAt).toBeNull();
  });

  it("tailwags on entrance, stretches before bored rest, and exposes sparse deadlines", () => {
    const entrance = active();
    expect(entrance).toMatchObject({ behavior: "tailwag", startedAt: 0, nextWakeAt: 10 });
    const attentive = wake(entrance);
    expect(attentive).toMatchObject({ behavior: "idle", endsAt: null, nextWakeAt: 30_000 });
    expect(wake(attentive, 29_999)).toBe(attentive);
    const stretch = wake(attentive);
    expect(stretch).toMatchObject({ behavior: "stretch", nextWakeAt: 30_040 });
    expect(wake(stretch)).toMatchObject({ behavior: "bored", nextWakeAt: 120_000 });
  });

  it.each([
    [29_999, "idle"],
    [30_000, "ear-perk"],
  ])("responds to first typing at %i ms with %s", (now, behavior) => {
    expect(step(idle(), { type: "typing" }, now).behavior).toBe(behavior);
  });

  it("measures quiet since the most recent input, not scene creation", () => {
    const early = step(idle(), { type: "typing" }, 29_999);
    const immediate = step(early, { type: "typing" }, 30_000);
    expect(immediate.behavior).toBe("idle");
    expect(immediate.lastTypingAt).toBe(30_000);
    expect(step(immediate, { type: "typing" }, 60_000).behavior).toBe("ear-perk");
  });

  it("does not restart an ear perk on repeated typing or replay it afterward", () => {
    const perk = step(idle(), { type: "typing" }, 30_000);
    const more = step(perk, { type: "typing" }, 30_001);
    expect(more).toMatchObject({ behavior: "ear-perk", startedAt: 30_000, nextWakeAt: 30_020, lastTypingAt: 30_001 });
    const done = wake(more);
    expect(done).toMatchObject({ behavior: "idle", nextWakeAt: 60_001 });
    expect(step(done, { type: "typing" }, 30_021).behavior).toBe("idle");
  });

  it("lets typing cancel ambient motion without leaving a queued action", () => {
    const twirl = wake(bored(), 120_000, 0);
    const perk = step(twirl, { type: "typing" }, 120_001);
    expect(perk.behavior).toBe("ear-perk");
    const done = wake(perk);
    expect(done).toMatchObject({ behavior: "idle", nextWakeAt: 150_001 });
  });

  it("does not twirl or skate during ongoing typing or immediately catch up afterward", () => {
    let state = idle();
    for (let now = 20_000; now <= 620_000; now += 20_000) {
      state = step(state, { type: "typing" }, now, 0);
      expect(state.behavior).toBe("idle");
      expect(wake(state, now, 0)).toBe(state);
    }
    const resting = wake(wake(state));
    expect(resting.behavior).toBe("bored");
    expect(resting.nextWakeAt).toBeGreaterThan(650_040);
  });
});

describe("Lucia ambient choices and anchored gaze", () => {
  it("consumes random misses once, with at least two minutes between twirl attempts", () => {
    const miss = wake(bored(), 120_000, 0.25);
    expect(miss).toMatchObject({ behavior: "bored", nextWakeAt: 240_000 });
    expect(wake(miss, 120_000, 0)).toBe(miss);
    expect(wake(miss, 239_999, 0)).toBe(miss);
    expect(wake(miss, 240_000, 0).behavior).toBe("twirl");
  });

  it("cannot quickly repeat a successful twirl", () => {
    const twirl = wake(bored(), 120_000, 0);
    const stretch = wake(twirl);
    expect(stretch.behavior).toBe("stretch");
    const resting = wake(stretch);
    expect(resting.nextWakeAt).toBe(240_000);
    expect(wake(resting, 239_999, 0)).toBe(resting);
    expect(wake(resting, 240_000, 0).behavior).toBe("twirl");
  });

  it("selects one rare skateboard at a ten-minute deadline and spaces the next twirl", () => {
    const skateboard = wake(bored(), 600_000, 0);
    expect(skateboard).toMatchObject({
      behavior: "skateboard",
      nextSkateboardAt: 1_200_000,
      nextTwirlAt: 720_000,
      nextWakeAt: 600_050,
    });
    const resting = wake(wake(skateboard));
    expect(resting).toMatchObject({ behavior: "bored", nextWakeAt: 720_000 });
    expect(wake(resting, 600_090, 0)).toBe(resting);
  });

  it("only allows the lower-probability skateboard roll and consumes its miss", () => {
    const twirl = wake(bored(), 600_000, 0.1);
    expect(twirl).toMatchObject({ behavior: "twirl", nextSkateboardAt: 1_200_000 });
  });

  it("shares the twirl schedule across modes without moving the seated gaze anchor", () => {
    const twirl = wake(bored(), 120_000, 0);
    const gaze = step(twirl, { type: "mode", mode: "gaze" }, 120_001);
    expect(gaze).toMatchObject({ behavior: "gaze", nextTwirlAt: 240_000, nextWakeAt: 240_000 });
    const nextTwirl = wake(gaze, 240_000, 0);
    expect(nextTwirl.behavior).toBe("twirl");
    expect(wake(nextTwirl).behavior).toBe("gaze");
  });

  it("never selects a skateboard or spins on its overdue deadline in gaze mode", () => {
    const gaze = active("gaze");
    const twirl = wake(gaze, 600_000, 0);
    expect(twirl.behavior).toBe("twirl");
    const returned = wake(twirl);
    expect(returned).toMatchObject({ behavior: "gaze", nextWakeAt: 720_000 });
  });

  it("requires quiet for a gaze twirl too", () => {
    const gaze = step(active("gaze"), { type: "typing" }, 119_999);
    const settled = wake(gaze);
    expect(settled).toMatchObject({ behavior: "gaze", nextWakeAt: 240_019 });
    expect(wake(settled, 120_000, 0)).toBe(settled);
  });
});

describe("Lucia sock ownership and explicit barking", () => {
  it.each([
    ["tailwag", () => active()],
    ["stretch", () => wake(idle())],
    ["ear-perk", () => step(idle(), { type: "typing" }, 30_000)],
    ["twirl", () => wake(bored(), 120_000, 0)],
    ["skateboard", () => wake(bored(), 600_000, 0)],
    ["bark", () => step(idle(), { type: "bark" }, 20)],
  ] as const)("preempts %s with one sock sequence", (_behavior, makeState) => {
    const from = makeState();
    const now = from.startedAt + 1;
    const chase = step(from, { type: "sock" }, now);
    expect(chase).toMatchObject({ behavior: "chase", nextWakeAt: now + 60 });
    expect(step(chase, { type: "sock" }, now + 1)).toBe(chase);
    expect(step(chase, { type: "bark" }, now + 1)).toBe(chase);
    const done = finishSock(chase);
    expect(done).toMatchObject({ behavior: "idle", lastTypingAt: now + 210, nextWakeAt: now + 30_210 });
  });

  it("consumes typing suppressed by sock ownership without replay", () => {
    const config = { durationsMs: { ...CONFIG.durationsMs, chase: 40_000 } };
    let state = step(idle(), { type: "sock" }, 20, 0, config);
    state = step(state, { type: "typing" }, 30_020);
    expect(state).toMatchObject({ behavior: "chase", lastTypingAt: 30_020 });
    const done = finishSock(state);
    expect(step(done, { type: "typing" }, done.startedAt + 1).behavior).toBe("idle");
  });

  it("advances a very late wake only one phase, with a fresh full duration", () => {
    const chase = step(idle(), { type: "sock" }, 20);
    const pickup = wake(chase, 1_000_000, 0);
    expect(pickup).toMatchObject({ behavior: "pickup", startedAt: 1_000_000, nextWakeAt: 1_000_070 });
    expect(wake(pickup, 1_000_000, 0)).toBe(pickup);
  });

  it("returns to the latest chosen mode after sock completion", () => {
    const chase = step(idle(), { type: "sock" }, 20);
    const changed = step(chase, { type: "mode", mode: "gaze" }, 21);
    expect(changed.behavior).toBe("chase");
    expect(finishSock(changed).behavior).toBe("gaze");
  });

  it("plays a visual bark once, respects its cooldown, and resumes gaze", () => {
    const bark = step(active("gaze"), { type: "bark" }, 10);
    expect(bark).toMatchObject({ behavior: "bark", nextWakeAt: 100, nextBarkAt: 3_010 });
    expect(step(bark, { type: "bark" }, 11)).toBe(bark);
    const returned = wake(bark);
    expect(returned.behavior).toBe("gaze");
    expect(step(returned, { type: "bark" }, 3_009)).toBe(returned);
    expect(step(returned, { type: "bark" }, 3_010).behavior).toBe("bark");
  });

  it("cannot restart a bark even when its clip outlasts the cooldown", () => {
    const config = { durationsMs: { ...CONFIG.durationsMs, bark: 5_000 } };
    const bark = step(idle(), { type: "bark" }, 10, 0, config);
    expect(step(bark, { type: "bark" }, 3_010, 0, config)).toBe(bark);
  });
});

describe("Lucia reading follows real aggregate inference", () => {
  it("debounces short requests without enter or exit animation", () => {
    const pending = step(idle(), { type: "inference", pending: true }, 100);
    expect(pending.nextWakeAt).toBe(300);
    expect(wake(pending, 299)).toBe(pending);
    const done = step(pending, { type: "inference", pending: false }, 299);
    expect(done).toMatchObject({ behavior: "idle", nextWakeAt: 30_000, readingReadyAt: null });
    expect(wake(done, 300)).toBe(done);
  });

  it("enters at the exact debounce deadline, then loops without a timer until completion", () => {
    const pending = step(idle(), { type: "inference", pending: true }, 100);
    expect(step(pending, { type: "inference", pending: true }, 200)).toBe(pending);
    const entering = wake(pending);
    expect(entering).toMatchObject({ behavior: "reading-enter", nextWakeAt: 400 });
    expect(step(entering, { type: "inference", pending: true }, 301)).toBe(entering);
    const loop = wake(entering);
    expect(loop).toMatchObject({ behavior: "reading", nextWakeAt: null, endsAt: null });
    expect(step(loop, { type: "wake" }, 1_000_000, 0)).toBe(loop);
    const exiting = step(loop, { type: "inference", pending: false }, 1_000);
    expect(exiting).toMatchObject({ behavior: "reading-exit", nextWakeAt: 1_110 });
    expect(wake(exiting).behavior).toBe("idle");
  });

  it("reads before advancing a cosmetic whose deadline overlaps the debounce", () => {
    const config = { durationsMs: { ...CONFIG.durationsMs, twirl: 1_000 } };
    const twirl = step(bored(), { type: "wake" }, 120_000, 0, config);
    const pending = step(twirl, { type: "inference", pending: true }, 120_010, 0, config);
    expect(pending.nextWakeAt).toBe(120_210);
    expect(step(pending, { type: "wake" }, 120_210, 0, config).behavior).toBe("reading-enter");
  });

  it("waits for sock ownership, then reads if the request still exists", () => {
    const chase = step(idle(), { type: "sock" }, 20);
    const pending = step(chase, { type: "inference", pending: true }, 21);
    expect(pending.nextWakeAt).toBe(80);
    expect(finishSock(pending).behavior).toBe("reading-enter");
  });

  it("does not replay a request that completed during the sock sequence", () => {
    const chase = step(idle(), { type: "sock" }, 20);
    const pending = step(chase, { type: "inference", pending: true }, 21);
    const done = step(pending, { type: "inference", pending: false }, 22);
    expect(finishSock(done)).toMatchObject({ behavior: "idle", inferencePending: false });
  });

  it("lets an explicit sock interrupt reading and then returns to the live request", () => {
    const chase = step(reading(), { type: "sock" }, 400);
    expect(finishSock(chase)).toMatchObject({ behavior: "reading-enter", inferencePending: true });
  });

  it("suppresses bark and ear perks during reading and returns to the latest mode", () => {
    const loop = reading();
    expect(step(loop, { type: "bark" }, 400)).toBe(loop);
    const typed = step(loop, { type: "typing" }, 30_000);
    expect(typed).toMatchObject({ behavior: "reading", lastTypingAt: 30_000 });
    const changed = step(typed, { type: "mode", mode: "gaze" }, 30_001);
    expect(changed.behavior).toBe("reading");
    const exiting = step(changed, { type: "inference", pending: false }, 30_002);
    expect(wake(exiting)).toMatchObject({ behavior: "gaze", mode: "gaze" });
  });

  it("coalesces new work during reading exit without replaying the entrance", () => {
    const exiting = step(reading(), { type: "inference", pending: false }, 400);
    const resumed = step(exiting, { type: "inference", pending: true }, 401);
    expect(resumed).toMatchObject({ behavior: "reading", nextWakeAt: null, readingReadyAt: null });
  });

  it("exits when a request ends during the reading entrance", () => {
    const pending = step(idle(), { type: "inference", pending: true }, 100);
    const entering = wake(pending);
    expect(step(entering, { type: "inference", pending: false }, 301).behavior).toBe("reading-exit");
  });
});

describe("Lucia feeding session", () => {
  it("accepts at most five fruits, with one discrete stage per completed interaction", () => {
    let state = active("feeding");
    expect(state).toMatchObject({ behavior: "feeding-idle", fedCount: 0, nextWakeAt: null });
    for (let count = 1; count <= 5; count++) {
      const now = (count - 1) * 200 + 10;
      const feeding = step(state, { type: "feed" }, now);
      expect(feeding).toMatchObject({ behavior: "feeding", fedCount: count });
      expect(step(feeding, { type: "feed" }, now + 1)).toBe(feeding);
      state = wake(feeding);
      expect(state).toMatchObject({ behavior: "feeding-idle", fedCount: count, nextWakeAt: null });
    }
    const full = step(state, { type: "feed" }, 1_010);
    expect(full).toMatchObject({ behavior: "full", fedCount: 5 });
    expect(step(full, { type: "feed" }, 1_011)).toBe(full);
    state = wake(full);
    expect(step(state, { type: "feed" }, 1_200)).toMatchObject({ behavior: "full", fedCount: 5 });
  });

  it("suppresses socks, barking, typing reactions, and ambient timers", () => {
    let state = active("feeding");
    for (const type of ["sock", "bark", "wake"] as const) {
      expect(step(state, { type }, 1_000_000, 0)).toBe(state);
    }
    state = step(state, { type: "typing" }, 1_000_000, 0);
    expect(state).toMatchObject({ behavior: "feeding-idle", fedCount: 0, nextWakeAt: null });
  });

  it("reset cancels the current feeding transition and clears the session", () => {
    const feeding = step(active("feeding"), { type: "feed" }, 10);
    const reset = step(feeding, { type: "reset" }, 11);
    expect(reset).toMatchObject({ behavior: "feeding-idle", fedCount: 0, nextWakeAt: null });
    expect(step(reset, { type: "wake" }, 130)).toBe(reset);
  });

  it("clears the count on mode exit and cancels incompatible ownership on entry", () => {
    const feeding = step(active("feeding"), { type: "feed" }, 10);
    const gaze = step(feeding, { type: "mode", mode: "gaze" }, 11);
    expect(gaze).toMatchObject({ behavior: "gaze", fedCount: 0 });
    const chase = step(gaze, { type: "sock" }, 12);
    const returned = step(chase, { type: "mode", mode: "feeding" }, 13);
    expect(returned).toMatchObject({ behavior: "feeding-idle", fedCount: 0, nextWakeAt: null });
  });

  it("preserves accepted fruit through reading and restores the stage afterward", () => {
    const feeding = step(active("feeding"), { type: "feed" }, 10);
    const pending = step(feeding, { type: "inference", pending: true }, 11);
    const entering = wake(wake(pending));
    expect(entering).toMatchObject({ behavior: "reading-enter", fedCount: 1 });
    const loop = wake(entering);
    expect(step(loop, { type: "feed" }, 400)).toBe(loop);
    const exiting = step(loop, { type: "inference", pending: false }, 401);
    expect(wake(exiting)).toMatchObject({ behavior: "feeding-idle", fedCount: 1 });
  });

  it("keeps the five-fruit cap across hiding and resuming", () => {
    let state = active("feeding");
    for (let count = 1; count <= 5; count++) {
      state = wake(step(state, { type: "feed" }, count * 200));
    }
    const hidden = step(state, { ...ENVIRONMENT, visible: false }, 1_200);
    const resumed = step(hidden, ENVIRONMENT, 1_000_000);
    expect(resumed).toMatchObject({ behavior: "feeding-idle", fedCount: 5, nextWakeAt: null });
    expect(step(resumed, { type: "feed" }, 1_000_001)).toMatchObject({ behavior: "full", fedCount: 5 });
  });

  it("allows a reset during reading without cancelling the live request", () => {
    const feeding = step(active("feeding"), { type: "feed" }, 10);
    const pending = step(feeding, { type: "inference", pending: true }, 11);
    const loop = wake(wake(wake(pending)));
    const reset = step(loop, { type: "reset" }, 400);
    expect(reset).toMatchObject({ behavior: "reading", fedCount: 0, inferencePending: true });
    const exiting = step(reset, { type: "inference", pending: false }, 401);
    expect(wake(exiting)).toMatchObject({ behavior: "feeding-idle", fedCount: 0 });
  });
});

describe("Lucia visibility, activation, and reduced motion", () => {
  it.each([
    { active: true, visible: false },
    { active: false, visible: true },
  ])("resets transient behavior when unavailable: %j", (environment) => {
    const chase = step(bored(), { type: "sock" }, 40_000);
    const suspended = step(chase, { ...ENVIRONMENT, ...environment }, 40_001);
    expect(suspended).toMatchObject({ behavior: "inactive", nextWakeAt: null, nextTwirlAt: null, nextSkateboardAt: null });
    expect(step(suspended, { type: "wake" }, 1_000_000, 0)).toBe(suspended);
    const resumed = step(suspended, ENVIRONMENT, 1_000_000, 0);
    expect(resumed).toMatchObject({ behavior: "tailwag", nextWakeAt: 1_000_010, nextTwirlAt: 1_120_000, lastTypingAt: 1_000_000 });
    expect(wake(resumed)).toMatchObject({ behavior: "idle", nextWakeAt: 1_030_000 });
  });

  it("resumes a still-pending request with a fresh debounce, without hidden backlog", () => {
    const hidden = step(reading("gaze"), { ...ENVIRONMENT, visible: false }, 400);
    expect(hidden).toMatchObject({ inferencePending: true, readingReadyAt: null, nextWakeAt: null });
    const resumed = step(hidden, ENVIRONMENT, 1_000_000);
    expect(resumed).toMatchObject({ mode: "gaze", readingReadyAt: 1_000_200 });
    const gaze = wake(resumed);
    expect(gaze).toMatchObject({ behavior: "gaze", nextWakeAt: 1_000_200 });
    expect(wake(gaze).behavior).toBe("reading-enter");
  });

  it("accepts completion while hidden so reading cannot flash on resume", () => {
    const hidden = step(reading(), { ...ENVIRONMENT, visible: false }, 400);
    const completed = step(hidden, { type: "inference", pending: false }, 401);
    const resumed = step(completed, ENVIRONMENT, 1_000_000);
    expect(wake(resumed)).toMatchObject({ behavior: "idle", inferencePending: false, nextWakeAt: 1_030_000 });
  });

  it("preserves accepted fruit and bark cooldown while hidden, then restores the stage", () => {
    const bark = step(idle(), { type: "bark" }, 10);
    const feeding = step(step(bark, { type: "mode", mode: "feeding" }, 11), { type: "feed" }, 12);
    const hidden = step(feeding, { ...ENVIRONMENT, visible: false }, 13);
    expect(hidden).toMatchObject({ mode: "feeding", fedCount: 1, nextBarkAt: 3_010 });
    const resumed = step(hidden, ENVIRONMENT, 1_000_000);
    expect(resumed).toMatchObject({ behavior: "feeding-idle", fedCount: 1, nextWakeAt: null });
    const deactivated = step(resumed, { ...ENVIRONMENT, active: false }, 1_000_001);
    expect(deactivated).toMatchObject({ behavior: "inactive", fedCount: 0 });
  });

  it("has no unsolicited behavior or idle deadlines with reduced motion", () => {
    const state = active("companion", true);
    expect(state).toMatchObject({ behavior: "idle", nextWakeAt: null, nextTwirlAt: null, nextSkateboardAt: null });
    const typed = step(state, { type: "typing" }, 30_000, 0);
    expect(typed).toMatchObject({ behavior: "idle", nextWakeAt: null });
    expect(step(typed, { type: "wake" }, 1_000_000, 0)).toBe(typed);
  });

  it.each([
    [120_000, "twirl"],
    [600_000, "skateboard"],
  ])("cancels %s ms %s immediately when reduced motion is enabled", (now, behavior) => {
    const motion = wake(bored(), now, 0);
    expect(motion.behavior).toBe(behavior);
    const stopped = step(motion, { ...ENVIRONMENT, reducedMotion: true }, now + 1);
    expect(stopped).toMatchObject({ behavior: "idle", nextWakeAt: null });
  });

  it("keeps only semantic deadlines for explicit static sock and bark poses", () => {
    const reduced = active("companion", true);
    const bark = step(reduced, { type: "bark" }, 10);
    expect(wake(bark)).toMatchObject({ behavior: "idle", nextWakeAt: null });
    const chase = step(reduced, { type: "sock" }, 20);
    expect(finishSock(chase)).toMatchObject({ behavior: "idle", nextWakeAt: null });
  });

  it("enters and leaves static reading based on real activity without motion phases", () => {
    const loop = reading("gaze", true);
    expect(loop).toMatchObject({ behavior: "reading", nextWakeAt: null });
    expect(step(loop, { type: "inference", pending: false }, 400)).toMatchObject({ behavior: "gaze", nextWakeAt: null });
  });

  it("can reduce motion during reading without ending the live request", () => {
    const loop = step(reading(), { ...ENVIRONMENT, reducedMotion: true }, 400);
    expect(loop).toMatchObject({ behavior: "reading", inferencePending: true, nextWakeAt: null });
  });
});

describe("Lucia metadata durations", () => {
  it("uses the supplied phase duration without assuming frames or geometry", () => {
    const config = { durationsMs: { ...CONFIG.durationsMs, tailwag: 1_234 } };
    const state = step(createLuciaBehavior(10_000), ENVIRONMENT, 10_000, 0, config);
    expect(state).toMatchObject({ behavior: "tailwag", nextWakeAt: 11_234 });
  });

  it.each([0, -1, Infinity, NaN])("rejects invalid duration %s before scheduling", (duration) => {
    const config = { durationsMs: { ...CONFIG.durationsMs, tailwag: duration } };
    expect(() => step(createLuciaBehavior(0), ENVIRONMENT, 0, 0, config)).toThrow(RangeError);
  });
});
