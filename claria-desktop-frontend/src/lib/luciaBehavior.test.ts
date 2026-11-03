import { describe, expect, it } from "vitest";
import {
  LUCIA_FUN_POOL,
  LUCIA_PET_BARK_WINDOW_MS,
  LUCIA_PET_FUN_WINDOW_MS,
  canSkate,
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
    "reading-play": 150,
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
    for (const type of ["typing", "sock", "pet", "feed", "wake"] as const) {
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
    expect(miss).toMatchObject({ behavior: "bored", nextTwirlAt: 240_000, nextWakeAt: 180_000 });
    expect(wake(miss, 120_000, 0)).toBe(miss);
    // The three-minute skate attempt misses; the twirl is not due yet.
    const skateMiss = wake(miss, 180_000, 0.99);
    expect(skateMiss).toMatchObject({ behavior: "bored", nextSkateboardAt: 360_000, nextWakeAt: 240_000 });
    expect(wake(skateMiss, 239_999, 0)).toBe(skateMiss);
    expect(wake(skateMiss, 240_000, 0).behavior).toBe("twirl");
  });

  it("cannot quickly repeat a successful twirl", () => {
    const twirl = wake(bored(), 120_000, 0);
    const stretch = wake(twirl);
    expect(stretch.behavior).toBe("stretch");
    const resting = wake(stretch);
    expect(resting).toMatchObject({ nextTwirlAt: 240_000, nextWakeAt: 180_000 });
    const skateMiss = wake(resting, 180_000, 0.99);
    expect(wake(skateMiss, 239_999, 0)).toBe(skateMiss);
    expect(wake(skateMiss, 240_000, 0).behavior).toBe("twirl");
  });

  it("skates at a three-minute deadline and spaces the next twirl", () => {
    const skateboard = wake(bored(), 180_000, 0);
    expect(skateboard).toMatchObject({
      behavior: "skateboard",
      nextSkateboardAt: 360_000,
      nextTwirlAt: 300_000,
      nextWakeAt: 180_050,
    });
    const resting = wake(wake(skateboard));
    expect(resting).toMatchObject({ behavior: "bored", nextWakeAt: 300_000 });
    expect(wake(resting, 180_090, 0)).toBe(resting);
  });

  it.each([
    [0.49, "skateboard"],
    [0.5, "twirl"],
    [0.62, "twirl"],
    [0.63, "bored"],
  ])("gives a skate half the rolls and rescales a miss so a due twirl keeps its chance: %s", (roll, behavior) => {
    const chosen = wake(bored(), 180_000, roll);
    expect(chosen).toMatchObject({ behavior, nextSkateboardAt: 360_000 });
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
    ["skateboard", () => wake(bored(), 180_000, 0)],
    ["bark", () => step(idle(), { type: "pet" }, 20)],
  ] as const)("preempts %s with one sock sequence", (_behavior, makeState) => {
    const from = makeState();
    const now = from.startedAt + 1;
    const chase = step(from, { type: "sock" }, now);
    expect(chase).toMatchObject({ behavior: "chase", nextWakeAt: now + 60 });
    expect(step(chase, { type: "sock" }, now + 1)).toBe(chase);
    expect(step(chase, { type: "pet" }, now + 1)).toBe(chase);
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

  it("plays a visual bark once, ignores clicks while it plays, and resumes gaze", () => {
    const bark = step(active("gaze"), { type: "pet" }, 10);
    expect(bark).toMatchObject({ behavior: "bark", nextWakeAt: 100 });
    expect(step(bark, { type: "pet" }, 11)).toBe(bark);
    const returned = wake(bark);
    expect(returned.behavior).toBe("gaze");
    expect(step(returned, { type: "pet" }, 101).behavior).toBe("bark");
  });

  it("cannot restart a bark even when its clip is long", () => {
    const config = { durationsMs: { ...CONFIG.durationsMs, bark: 5_000 } };
    const bark = step(idle(), { type: "pet" }, 10, 0, config);
    expect(step(bark, { type: "pet" }, 3_010, 0, config)).toBe(bark);
  });
});

/** Two barks in, with the fun window open; returns the state and when it opened. */
function funReady(mode: LuciaMode = "companion", reducedMotion = false): LuciaBehaviorState {
  const first = wake(step(active(mode, reducedMotion), { type: "pet" }, 1_000));
  return wake(step(first, { type: "pet" }, first.petWindowEndsAt!));
}

describe("Lucia pet chain", () => {
  it("barks twice, then plays, opening each window when she is back at rest", () => {
    const first = step(idle(), { type: "pet" }, 1_000, 0);
    expect(first).toMatchObject({ behavior: "bark", petStep: 1, petReacting: true, petWindowEndsAt: null, endsAt: 1_090 });
    expect(step(first, { type: "pet" }, 1_050, 0)).toBe(first);
    const rested = wake(first);
    expect(rested).toMatchObject({ behavior: "idle", petReacting: false, petWindowEndsAt: 1_090 + LUCIA_PET_BARK_WINDOW_MS });
    const second = step(rested, { type: "pet" }, rested.petWindowEndsAt!, 0);
    expect(second).toMatchObject({ behavior: "bark", petStep: 2 });
    const afterSecond = wake(second);
    expect(afterSecond.petWindowEndsAt).toBe(second.endsAt! + LUCIA_PET_FUN_WINDOW_MS);
    const fun = step(afterSecond, { type: "pet" }, afterSecond.petWindowEndsAt!, 0);
    expect(fun).toMatchObject({ behavior: "twirl", petStep: 3, petReacting: true });
    const afterFun = wake(fun);
    expect(afterFun.petWindowEndsAt).toBe(fun.endsAt! + LUCIA_PET_FUN_WINDOW_MS);
    expect(step(afterFun, { type: "pet" }, afterFun.petWindowEndsAt!, 0.99)).toMatchObject({ behavior: "stretch", petStep: 3 });
  });

  it("starts over with a bark once a window lapses", () => {
    const first = wake(step(idle(), { type: "pet" }, 1_000));
    expect(step(first, { type: "pet" }, first.petWindowEndsAt! + 1)).toMatchObject({ behavior: "bark", petStep: 1 });
    const ready = funReady();
    expect(step(ready, { type: "pet" }, ready.petWindowEndsAt! + 1, 0)).toMatchObject({ behavior: "bark", petStep: 1 });
  });

  it("does not stretch on the heels of a pet: petting counts as activity", () => {
    const rested = wake(step(idle(), { type: "pet" }, 40_000));
    expect(rested).toMatchObject({ behavior: "idle", lastTypingAt: 40_000, nextWakeAt: 70_000 });
  });

  it.each(LUCIA_FUN_POOL.map((fun, index) => [fun, index] as const))("rolls %s from the fun pool", (fun, index) => {
    const ready = funReady();
    const roll = (index + 0.5) / LUCIA_FUN_POOL.length;
    const started = { reading: "reading-enter" }[fun as string] ?? fun;
    expect(step(ready, { type: "pet" }, ready.startedAt + 1, roll).behavior).toBe(started);
  });

  it.each([
    ["reading", 0.4, ["reading-enter", "reading-play", "reading-exit"]],
    ["stretch", 0.99, ["stretch"]],
  ] as const)("opens the next window only once a multi-phase %s is over", (_fun, roll, phases) => {
    const ready = funReady();
    let state = step(ready, { type: "pet" }, ready.startedAt + 1, roll);
    for (const phase of phases) {
      expect(state).toMatchObject({ behavior: phase, petReacting: true, petWindowEndsAt: null });
      expect(step(state, { type: "pet" }, state.startedAt + 1, 0)).toBe(state);
      state = wake(state);
    }
    expect(state).toMatchObject({ behavior: phases[0] === "stretch" ? "bored" : "idle", petReacting: false, petWindowEndsAt: state.startedAt + LUCIA_PET_FUN_WINDOW_MS });
  });

  it.each([
    ["a sock", { type: "sock" }],
    ["a mode change", { type: "mode", mode: "gaze" }],
    ["hiding", { ...ENVIRONMENT, visible: false }],
  ] satisfies Array<[string, LuciaBehaviorEvent]>)("ends the chain on %s", (_label, event) => {
    const ready = funReady();
    const ended = step(ready, event, ready.startedAt + 1);
    expect(ended).toMatchObject({ petStep: 0, petWindowEndsAt: null, petReacting: false });
  });

  it("ends the chain when real inference starts reading", () => {
    const pending = step(funReady(), { type: "inference", pending: true }, 10_000);
    expect(wake(pending)).toMatchObject({ behavior: "reading-enter", petStep: 0, petReacting: false });
  });

  it("ends the chain when real inference takes over a requested read while the paper comes up", () => {
    const ready = funReady();
    const entering = step(ready, { type: "pet" }, ready.startedAt + 1, 0.4);
    expect(entering.behavior).toBe("reading-enter");
    const live = wake(step(entering, { type: "inference", pending: true }, entering.startedAt + 10));
    expect(live).toMatchObject({ behavior: "reading", petStep: 0, petReacting: false, readingReadyAt: null, nextWakeAt: null });
  });

  it("lets typing interrupt a reaction without ending the chain", () => {
    const ready = funReady();
    const twirl = step(ready, { type: "pet" }, ready.startedAt + 1, 0);
    const typed = step(twirl, { type: "typing" }, twirl.startedAt + 1);
    expect(typed).toMatchObject({ behavior: "idle", petStep: 3, petReacting: false, petWindowEndsAt: twirl.startedAt + 1 + LUCIA_PET_FUN_WINDOW_MS });
    expect(step(typed, { type: "pet" }, typed.startedAt + 1, 0).behavior).toBe("twirl");
  });

  it("blocks a demo preview while a pet reaction plays", () => {
    const twirl = step(funReady(), { type: "pet" }, 20_000, 0);
    expect(step(twirl, { type: "preview", behavior: "tailwag" }, 20_001)).toBe(twirl);
  });

  it("only barks, statically, with reduced motion", () => {
    const ready = funReady("companion", true);
    const bark = step(ready, { type: "pet" }, ready.startedAt + 1, 0);
    expect(bark).toMatchObject({ behavior: "bark", petStep: 3 });
    expect(wake(bark)).toMatchObject({ behavior: "idle", nextWakeAt: null });
  });

  it("works in Watch mode and is ignored in Feed mode", () => {
    const ready = funReady("gaze");
    expect(step(ready, { type: "pet" }, ready.startedAt + 1, 0).behavior).toBe("twirl");
    // A petted stretch sits back up to keep watching instead of lying down for good.
    const stretch = step(ready, { type: "pet" }, ready.startedAt + 1, 0.99);
    expect(stretch.behavior).toBe("stretch");
    expect(wake(stretch)).toMatchObject({ behavior: "gaze", petReacting: false });
    const feeding = active("feeding");
    expect(step(feeding, { type: "pet" }, 10)).toBe(feeding);
  });

  it("lets real inference take over a requested read, and typing put it away", () => {
    const ready = funReady();
    const turning = wake(step(ready, { type: "pet" }, ready.startedAt + 1, 0.4));
    expect(turning.behavior).toBe("reading-play");
    const typed = step(turning, { type: "typing" }, turning.startedAt + 1);
    expect(typed).toMatchObject({ behavior: "reading-exit", petStep: 3, petReacting: true });
    expect(wake(typed)).toMatchObject({ behavior: "idle", petReacting: false });
    const live = step(turning, { type: "inference", pending: true }, turning.startedAt + 1);
    expect(live).toMatchObject({ behavior: "reading", nextWakeAt: null, petStep: 0, petReacting: false });
    expect(step(live, { type: "inference", pending: false }, turning.startedAt + 2).behavior).toBe("reading-exit");
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
    expect(step(loop, { type: "pet" }, 400)).toBe(loop);
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
      expect(feeding).toMatchObject({ behavior: "feeding", fedCount: count - 1 });
      expect(step(feeding, { type: "feed" }, now + 1)).toBe(feeding);
      state = wake(feeding);
      if (count === 5) {
        expect(state).toMatchObject({ behavior: "full", fedCount: 5 });
        state = wake(state);
      }
      expect(state).toMatchObject({ behavior: "feeding-idle", fedCount: count, nextWakeAt: null });
    }
    expect(step(state, { type: "feed" }, 1_010)).toBe(state);
    expect(step(state, { type: "wake" }, 1_200)).toBe(state);
  });

  it.each([
    { type: "reset" },
    { ...ENVIRONMENT, visible: false },
    { ...ENVIRONMENT, active: false },
    { type: "mode", mode: "gaze" },
  ] satisfies LuciaBehaviorEvent[])("cancels an uncompleted feed without increment: %j", (event) => {
    const feeding = step(active("feeding"), { type: "feed" }, 10);
    const interrupted = step(feeding, event, 11);
    expect(interrupted.fedCount).toBe(0);
    expect(step(interrupted, { type: "wake" }, 130).fedCount).toBe(0);
  });

  it("inference interruption cannot commit a fruit or replay it after reading", () => {
    const feeding = step(active("feeding"), { type: "feed" }, 0, 0.5, {
      durationsMs: { ...CONFIG.durationsMs, feeding: 500 },
    });
    const pending = step(feeding, { type: "inference", pending: true }, 1);
    const interrupted = wake(pending, 201);
    expect(interrupted).toMatchObject({ behavior: "reading-enter", fedCount: 0 });
    const done = step(wake(interrupted), { type: "inference", pending: false }, 400);
    expect(wake(done)).toMatchObject({ behavior: "feeding-idle", fedCount: 0 });
  });

  it("rejects feeds even during the inference debounce", () => {
    const pending = step(active("feeding"), { type: "inference", pending: true }, 1);
    expect(step(pending, { type: "feed" }, 100)).toBe(pending);
  });

  it("suppresses socks, barking, typing reactions, and ambient timers", () => {
    let state = active("feeding");
    for (const type of ["sock", "pet", "wake"] as const) {
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
    expect(step(resumed, { type: "feed" }, 1_000_001)).toBe(resumed);
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
  ])("resets transient behavior but keeps play deadlines when unavailable: %j", (environment) => {
    const chase = step(bored(), { type: "sock" }, 40_000);
    const suspended = step(chase, { ...ENVIRONMENT, ...environment }, 40_001);
    expect(suspended).toMatchObject({ behavior: "inactive", nextWakeAt: null, nextTwirlAt: 120_000, nextSkateboardAt: 180_000 });
    expect(step(suspended, { type: "wake" }, 1_000_000, 0)).toBe(suspended);
    const resumed = step(suspended, ENVIRONMENT, 1_000_000, 0);
    // Lapsed while away: due after the next quiet spell, not a fresh full cooldown.
    expect(resumed).toMatchObject({ behavior: "tailwag", nextWakeAt: 1_000_010, nextTwirlAt: 1_030_000, nextSkateboardAt: 1_030_000, lastTypingAt: 1_000_000 });
    const settled = wake(resumed);
    expect(settled).toMatchObject({ behavior: "idle", nextWakeAt: 1_030_000 });
    // The stretch that starts then discards the lapsed deadlines to a fresh cooldown.
    expect(wake(settled)).toMatchObject({ behavior: "stretch", nextTwirlAt: 1_150_000, nextSkateboardAt: 1_210_000 });
  });

  it("does not postpone play when the window keeps losing and regaining focus", () => {
    let state = bored();
    const twirlAt = state.nextTwirlAt!;
    for (let now = 41_000; now < twirlAt; now += 20_000) {
      state = step(step(state, { ...ENVIRONMENT, visible: false }, now), ENVIRONMENT, now + 1_000);
    }
    expect(state.nextTwirlAt).toBe(twirlAt);
  });

  it("lets a feed in progress finish when the motion preference changes", () => {
    const feeding = step(active("feeding"), { type: "feed" }, 10);
    const changed = step(feeding, { ...ENVIRONMENT, reducedMotion: true }, 20);
    expect(changed).toMatchObject({ behavior: "feeding", endsAt: feeding.endsAt });
    expect(wake(changed)).toMatchObject({ behavior: "feeding-idle", fedCount: 1 });
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

  it("preserves accepted fruit while hidden, then restores the stage", () => {
    const bark = step(idle(), { type: "pet" }, 10);
    const feeding = step(step(bark, { type: "mode", mode: "feeding" }, 11), { type: "feed" }, 12);
    const completed = wake(feeding);
    const hidden = step(completed, { ...ENVIRONMENT, visible: false }, 200);
    expect(hidden).toMatchObject({ mode: "feeding", fedCount: 1, petStep: 0 });
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
    [180_000, "skateboard"],
  ])("cancels %s ms %s immediately when reduced motion is enabled", (now, behavior) => {
    const motion = wake(bored(), now, 0);
    expect(motion.behavior).toBe(behavior);
    const stopped = step(motion, { ...ENVIRONMENT, reducedMotion: true }, now + 1);
    expect(stopped).toMatchObject({ behavior: "idle", nextWakeAt: null });
  });

  it("keeps only semantic deadlines for explicit static sock and bark poses", () => {
    const reduced = active("companion", true);
    const bark = step(reduced, { type: "pet" }, 10);
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


describe("Lucia's skateboard button", () => {
  it("sends her across the rail, counts as activity, and stands in for the next ambient skate", () => {
    const ride = step(bored(), { type: "skate" }, 100_000);
    expect(ride).toMatchObject({ behavior: "skateboard", lastTypingAt: 100_000, nextSkateboardAt: 280_000, nextWakeAt: 100_050 });
    expect(step(ride, { type: "skate" }, 100_001)).toBe(ride);
    expect(wake(ride)).toMatchObject({ behavior: "idle", nextWakeAt: 130_000 });
  });

  it("ends a pet chain and interrupts a pet reaction", () => {
    const bark = step(idle(), { type: "pet" }, 1_000);
    expect(step(bark, { type: "skate" }, 1_001)).toMatchObject({ behavior: "skateboard", petStep: 0, petReacting: false });
  });

  it("works in Watch mode and returns her to watching", () => {
    const ride = step(active("gaze"), { type: "skate" }, 1_000);
    expect(ride.behavior).toBe("skateboard");
    expect(wake(ride).behavior).toBe("gaze");
  });

  it.each([
    ["Feed mode", () => active("feeding")],
    ["reduced motion", () => active("companion", true)],
    ["a pending request", () => step(idle(), { type: "inference", pending: true }, 100)],
    ["reading", () => reading()],
    ["a sock run", () => step(idle(), { type: "sock" }, 100)],
    ["hidden", () => step(idle(), { ...ENVIRONMENT, visible: false }, 100)],
  ] as const)("is ignored during %s", (_case, makeState) => {
    const state = makeState();
    expect(step(state, { type: "skate" }, 101)).toBe(state);
  });

  it("shares its gate with the button", () => {
    expect(canSkate(idle())).toBe(true);
    expect(canSkate(wake(bored(), 180_000, 0))).toBe(false);
    expect(canSkate(active("feeding"))).toBe(false);
  });
});

describe("Lucia explicit local-demo previews", () => {
  it.each(["tailwag", "stretch", "ear-perk", "twirl"] as const)("uses the existing %s duration and wake path without changing play cadence", (behavior) => {
    const previous = idle();
    const preview = step(previous, { type: "preview", behavior }, 1000);
    expect(preview).toMatchObject({ behavior, endsAt: 1000 + CONFIG.durationsMs[behavior], nextTwirlAt: previous.nextTwirlAt, nextSkateboardAt: previous.nextSkateboardAt });
    expect(wake(preview).behavior).toBe(behavior === "stretch" ? "bored" : "idle");
  });

  it("previews a skate without moving the ambient skate deadline", () => {
    const previous = idle();
    const preview = step(previous, { type: "preview", behavior: "skateboard" }, 1000);
    expect(preview).toMatchObject({ behavior: "skateboard", endsAt: 1050, nextSkateboardAt: previous.nextSkateboardAt });
    expect(wake(preview).behavior).toBe("idle");
  });

  it("cannot interrupt owned work or bypass mode, environment and motion guards", () => {
    const pending = step(idle(), { type: "inference", pending: true }, 100);
    const states = [createLuciaBehavior(0), active("companion", true), active("feeding"), active("gaze"),
      step(idle(), { ...ENVIRONMENT, visible: false }, 100), pending, reading(),
      step(idle(), { type: "sock" }, 100), step(idle(), { type: "pet" }, 100)];
    for (const state of states) expect(step(state, { type: "preview", behavior: "twirl" }, 101)).toBe(state);
  });
});
