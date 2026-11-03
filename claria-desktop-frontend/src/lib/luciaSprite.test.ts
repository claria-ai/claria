import { describe, expect, it } from "vitest";
import {
  PETS_V2_ATLAS,
  PETS_V2_CLIPS,
  petsV2LookAngleFromOffset,
  selectPetsV2LookFrame,
  selectSpriteFrame,
  spriteClipDurationMs,
} from "./luciaSprite";
import type { PetsV2State, SpriteAtlas, SpriteClip } from "./luciaSprite";

describe("Pets v2 atlas contract", () => {
  it("has the exact 1536 × 2288 geometry", () => {
    expect(PETS_V2_ATLAS).toEqual({ columns: 8, rows: 11, cellWidth: 192, cellHeight: 208 });
    expect(PETS_V2_ATLAS.columns * PETS_V2_ATLAS.cellWidth).toBe(1536);
    expect(PETS_V2_ATLAS.rows * PETS_V2_ATLAS.cellHeight).toBe(2288);
  });

  const rows: [PetsV2State, number, number[]][] = [
    ["idle", 0, [280, 110, 110, 140, 140, 320]],
    ["running-right", 1, [120, 120, 120, 120, 120, 120, 120, 220]],
    ["running-left", 2, [120, 120, 120, 120, 120, 120, 120, 220]],
    ["waving", 3, [140, 140, 140, 280]],
    ["jumping", 4, [140, 140, 140, 140, 280]],
    ["failed", 5, [140, 140, 140, 140, 140, 140, 140, 240]],
    ["waiting", 6, [150, 150, 150, 150, 150, 260]],
    ["running", 7, [120, 120, 120, 120, 120, 220]],
    ["review", 8, [150, 150, 150, 150, 150, 280]],
  ];

  it.each(rows)("plays only the used %s cells at their specified timings", (state, row, durations) => {
    const clip = PETS_V2_CLIPS[state];
    expect(clip).toEqual({ row, firstColumn: 0, frameDurationsMs: durations });
    const cycleMs = durations.reduce((sum, duration) => sum + duration, 0);
    expect(spriteClipDurationMs(clip)).toBe(cycleMs);
    let startMs = 0;
    durations.forEach((duration, column) => {
      const expected = { row, column, frameIndex: column };
      expect(selectSpriteFrame(PETS_V2_ATLAS, clip, startMs)).toEqual(expected);
      expect(selectSpriteFrame(PETS_V2_ATLAS, clip, startMs + duration - 0.001)).toEqual(expected);
      expect(selectSpriteFrame(PETS_V2_ATLAS, clip, startMs + cycleMs * 3)).toEqual(expected);
      startMs += duration;
    });
    expect(selectSpriteFrame(PETS_V2_ATLAS, clip, cycleMs)).toEqual({ row, column: 0, frameIndex: 0 });
    expect(selectSpriteFrame(PETS_V2_ATLAS, clip, cycleMs * 100, { playback: "once" })).toEqual({
      row, column: durations.length - 1, frameIndex: durations.length - 1,
    });
  });
});

describe("selectSpriteFrame", () => {
  // Synthetic geometry exercises registration offsets without depending on artwork.
  const atlas: SpriteAtlas = { columns: 6, rows: 2, cellWidth: 32, cellHeight: 40 };
  const clip: SpriteClip = { row: 1, firstColumn: 2, frameDurationsMs: [25, 50, 100] };

  it.each([
    [-1, 0], [0, 0], [24.999, 0], [25, 1], [74.999, 1],
    [75, 2], [174.999, 2], [175, 0], [200, 1], [350, 0],
  ])("selects the nonuniform frame at %s ms", (elapsedMs, frameIndex) => {
    expect(selectSpriteFrame(atlas, clip, elapsedMs)).toEqual({
      row: 1, column: 2 + frameIndex, frameIndex,
    });
  });

  it("holds a one-shot's final used cell from its last frame through completion", () => {
    for (const elapsedMs of [75, 174.999, 175, 200, 1_000_000]) {
      expect(selectSpriteFrame(atlas, clip, elapsedMs, { playback: "once" })).toEqual({
        row: 1, column: 4, frameIndex: 2,
      });
    }
  });

  it("keeps reduced motion on the first used cell for either playback mode", () => {
    for (const playback of ["loop", "once"] as const) {
      for (const elapsedMs of [-1, 25, 75, 175, 1_000_000]) {
        expect(selectSpriteFrame(atlas, clip, elapsedMs, { playback, reducedMotion: true })).toEqual({
          row: 1, column: 2, frameIndex: 0,
        });
      }
    }
  });

  it("supports a single used cell", () => {
    const single: SpriteClip = { row: 0, firstColumn: 5, frameDurationsMs: [100] };
    for (const elapsedMs of [0, 100, 1_000_000]) {
      expect(selectSpriteFrame(atlas, single, elapsedMs)).toEqual({ row: 0, column: 5, frameIndex: 0 });
    }
  });

  it.each([NaN, Infinity, -Infinity])("rejects non-finite elapsed time %s", elapsedMs => {
    expect(() => selectSpriteFrame(atlas, clip, elapsedMs)).toThrow(RangeError);
  });

  it.each([[], [0], [-1], [NaN], [Infinity], [100, 0], [Number.MAX_VALUE, Number.MAX_VALUE]])(
    "rejects empty, nonpositive, or non-finite durations: %j",
    (...durations) => {
      expect(() => spriteClipDurationMs({ ...clip, frameDurationsMs: durations })).toThrow(RangeError);
    },
  );

  it.each([
    { row: -1 }, { row: 2 }, { row: 0.5 }, { row: NaN },
    { firstColumn: -1 }, { firstColumn: 4 }, { firstColumn: 1.5 }, { firstColumn: Infinity },
  ])("rejects clips outside the atlas: %j", invalid => {
    expect(() => selectSpriteFrame(atlas, { ...clip, ...invalid }, 0)).toThrow(RangeError);
  });

  it.each([
    { columns: 0 }, { rows: -1 }, { cellWidth: 0 }, { cellHeight: 0.5 }, { columns: NaN },
  ])("rejects invalid atlas geometry: %j", invalid => {
    expect(() => selectSpriteFrame({ ...atlas, ...invalid }, clip, 0)).toThrow(RangeError);
  });
});

describe("selectPetsV2LookFrame", () => {
  it.each([
    [0, 9, 0], [22.5, 9, 1], [45, 9, 2], [67.5, 9, 3],
    [90, 9, 4], [112.5, 9, 5], [135, 9, 6], [157.5, 9, 7],
    [180, 10, 0], [202.5, 10, 1], [225, 10, 2], [247.5, 10, 3],
    [270, 10, 4], [292.5, 10, 5], [315, 10, 6], [337.5, 10, 7],
  ])("maps clockwise %s° to row %s, column %s", (angleDegrees, row, column) => {
    expect(selectPetsV2LookFrame(angleDegrees)).toEqual({ row, column, frameIndex: column });
  });

  it.each([
    [11.249, 9, 0], [11.25, 9, 1], [168.749, 9, 7], [168.75, 10, 0],
    [348.749, 10, 7], [348.75, 9, 0], [360, 9, 0], [450, 9, 4], [-90, 10, 4],
  ])("snaps and wraps %s° without leaving the two look rows", (angleDegrees, row, column) => {
    expect(selectPetsV2LookFrame(angleDegrees)).toEqual({ row, column, frameIndex: column });
  });

  it("uses an explicit neutral fallback to idle, while 000° means up", () => {
    expect(selectPetsV2LookFrame(null, 0)).toEqual({ row: 0, column: 0, frameIndex: 0 });
    expect(selectPetsV2LookFrame(null, 280)).toEqual({ row: 0, column: 1, frameIndex: 1 });
    expect(selectPetsV2LookFrame(null, 1100)).toEqual({ row: 0, column: 0, frameIndex: 0 });
    expect(selectPetsV2LookFrame(0, 280)).toEqual({ row: 9, column: 0, frameIndex: 0 });
  });

  it("uses idle's first frame for reduced motion, including directed gaze", () => {
    for (const angleDegrees of [null, 0, 90, 180, 270]) {
      expect(selectPetsV2LookFrame(angleDegrees, 780, { reducedMotion: true })).toEqual({
        row: 0, column: 0, frameIndex: 0,
      });
    }
  });

  it.each([NaN, Infinity, -Infinity])("rejects an invalid angle %s instead of silently selecting up", angle => {
    expect(() => selectPetsV2LookFrame(angle)).toThrow(RangeError);
  });
});

describe("petsV2LookAngleFromOffset", () => {
  it.each([
    [{ x: 0, y: -100 }, 0, 9, 0],
    [{ x: 100, y: 0 }, 90, 9, 4],
    [{ x: 0, y: 100 }, 180, 10, 0],
    [{ x: -100, y: 0 }, 270, 10, 4],
  ] as const)("maps pointer offset %j to clockwise %s°", (offset, angle, row, column) => {
    const original = { ...offset };
    expect(petsV2LookAngleFromOffset(offset)).toBe(angle);
    expect(selectPetsV2LookFrame(petsV2LookAngleFromOffset(offset))).toEqual({
      row, column, frameIndex: column,
    });
    expect(offset).toEqual(original);
  });

  it("uses a circular inclusive deadzone and falls back to animated idle", () => {
    for (const offset of [{ x: 0, y: 0 }, { x: 0, y: -24 }, { x: 14.4, y: 19.2 }]) {
      expect(petsV2LookAngleFromOffset(offset)).toBeNull();
      expect(selectPetsV2LookFrame(petsV2LookAngleFromOffset(offset), 280)).toEqual({
        row: 0, column: 1, frameIndex: 1,
      });
    }
    expect(petsV2LookAngleFromOffset({ x: 0, y: -24.001 })).toBe(0);
    expect(petsV2LookAngleFromOffset({ x: 20, y: -20 })).toBe(45);
  });

  it("allows the deadzone to follow the displayed pet size", () => {
    expect(petsV2LookAngleFromOffset({ x: 40, y: 0 }, 48)).toBeNull();
    expect(petsV2LookAngleFromOffset({ x: 40, y: 0 }, 12)).toBe(90);
    expect(petsV2LookAngleFromOffset({ x: 0, y: 0 }, 0)).toBeNull();
    expect(petsV2LookAngleFromOffset({ x: -0.001, y: 0 }, 0)).toBe(270);
  });

  it.each([{ x: NaN, y: 0 }, { x: 0, y: Infinity }, { x: -Infinity, y: 0 }])(
    "rejects a non-finite pointer offset: %j",
    offset => expect(() => petsV2LookAngleFromOffset(offset)).toThrow(RangeError),
  );

  it.each([-1, NaN, Infinity])("rejects invalid deadzone %s", deadzone => {
    expect(() => petsV2LookAngleFromOffset({ x: 0, y: 0 }, deadzone)).toThrow(RangeError);
  });
});
