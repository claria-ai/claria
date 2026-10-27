/** Geometry and timing only; artwork and its measured registration are supplied separately. */
export interface SpriteAtlas {
  readonly columns: number;
  readonly rows: number;
  readonly cellWidth: number;
  readonly cellHeight: number;
}

/** A contiguous sequence of used cells in one atlas row. */
export interface SpriteClip {
  readonly row: number;
  readonly firstColumn: number;
  readonly frameDurationsMs: readonly number[];
}

export interface SpritePoint {
  readonly x: number;
  readonly y: number;
}

/** Fill these measurements from inspected, registered artwork before rendering it. */
export interface SpriteStripMetadata {
  readonly atlas: SpriteAtlas;
  readonly clip: SpriteClip;
  /** Resting ground contact in cell pixels; keep it fixed through airborne poses. */
  readonly baselineY: number;
  /** Cell-local anchor positions, with one entry per used frame for each named anchor. */
  readonly anchors?: Readonly<Record<string, readonly SpritePoint[]>>;
}

export interface SpriteFrame {
  readonly row: number;
  readonly column: number;
  /** Index within the row's clip, for looking up per-frame anchors. */
  readonly frameIndex: number;
}

export interface SpritePlaybackOptions {
  readonly playback?: "loop" | "once";
  /** Freeze on the clip's first used frame. */
  readonly reducedMotion?: boolean;
}

/** Pets v2: 1536 × 2288 pixels, with no gutters between its 192 × 208 cells. */
export const PETS_V2_ATLAS = {
  columns: 8,
  rows: 11,
  cellWidth: 192,
  cellHeight: 208,
} as const satisfies SpriteAtlas;

// Pets v2 animation-rows.md is the contract: trailing cells are transparent,
// and the longer last-frame holds are part of each animation's timing.
export const PETS_V2_CLIPS = {
  idle: { row: 0, firstColumn: 0, frameDurationsMs: [280, 110, 110, 140, 140, 320] },
  "running-right": { row: 1, firstColumn: 0, frameDurationsMs: [120, 120, 120, 120, 120, 120, 120, 220] },
  "running-left": { row: 2, firstColumn: 0, frameDurationsMs: [120, 120, 120, 120, 120, 120, 120, 220] },
  waving: { row: 3, firstColumn: 0, frameDurationsMs: [140, 140, 140, 280] },
  jumping: { row: 4, firstColumn: 0, frameDurationsMs: [140, 140, 140, 140, 280] },
  failed: { row: 5, firstColumn: 0, frameDurationsMs: [140, 140, 140, 140, 140, 140, 140, 240] },
  waiting: { row: 6, firstColumn: 0, frameDurationsMs: [150, 150, 150, 150, 150, 260] },
  running: { row: 7, firstColumn: 0, frameDurationsMs: [120, 120, 120, 120, 120, 220] },
  review: { row: 8, firstColumn: 0, frameDurationsMs: [150, 150, 150, 150, 150, 280] },
} as const satisfies Record<string, SpriteClip>;

export type PetsV2State = keyof typeof PETS_V2_CLIPS;

export function spriteClipDurationMs(clip: SpriteClip): number {
  if (clip.frameDurationsMs.length === 0) {
    throw new RangeError("A sprite clip must contain at least one used frame.");
  }
  let total = 0;
  for (const duration of clip.frameDurationsMs) {
    if (!Number.isFinite(duration) || duration <= 0) {
      throw new RangeError("Sprite frame durations must be finite and positive.");
    }
    total += duration;
  }
  if (!Number.isFinite(total)) {
    throw new RangeError("A sprite clip's total duration must be finite.");
  }
  return total;
}

function validateClipBounds(atlas: SpriteAtlas, clip: SpriteClip): void {
  for (const size of [atlas.columns, atlas.rows, atlas.cellWidth, atlas.cellHeight]) {
    if (!Number.isSafeInteger(size) || size <= 0) {
      throw new RangeError("Sprite atlas dimensions must be positive integers.");
    }
  }
  if (!Number.isSafeInteger(clip.row) || clip.row < 0 || clip.row >= atlas.rows
    || !Number.isSafeInteger(clip.firstColumn) || clip.firstColumn < 0
    || clip.firstColumn + clip.frameDurationsMs.length > atlas.columns) {
    throw new RangeError("The sprite clip's used cells must fit inside its atlas.");
  }
}

/**
 * Frame intervals include their start and exclude their end. Loops restart at
 * the exact cycle boundary; one-shots hold the final used cell after finishing.
 * Negative elapsed time means playback has not started. Non-finite time is an error.
 */
export function selectSpriteFrame(
  atlas: SpriteAtlas,
  clip: SpriteClip,
  elapsedMs: number,
  { playback = "loop", reducedMotion = false }: SpritePlaybackOptions = {},
): SpriteFrame {
  validateClipBounds(atlas, clip);
  const durationMs = spriteClipDurationMs(clip);
  if (!Number.isFinite(elapsedMs)) {
    throw new RangeError("Sprite elapsed time must be finite.");
  }

  let frameIndex = 0;
  if (!reducedMotion) {
    const elapsed = Math.max(0, elapsedMs);
    let remainingMs = playback === "loop" ? elapsed % durationMs : elapsed;
    while (frameIndex < clip.frameDurationsMs.length - 1
      && remainingMs >= clip.frameDurationsMs[frameIndex]) {
      remainingMs -= clip.frameDurationsMs[frameIndex];
      frameIndex += 1;
    }
  }
  return { row: clip.row, column: clip.firstColumn + frameIndex, frameIndex };
}

/**
 * Clockwise from up: 0° up, 90° right, 180° down, 270° left. Snap to the nearest
 * 22.5° pose, with exact half-steps choosing clockwise. `null` explicitly means
 * the neutral pointer deadzone and plays idle. Reduced motion always uses idle's
 * first frame, including when a direction was supplied.
 */
export function selectPetsV2LookFrame(
  angleDegrees: number | null,
  elapsedMs = 0,
  { reducedMotion = false }: Pick<SpritePlaybackOptions, "reducedMotion"> = {},
): SpriteFrame {
  if (angleDegrees !== null && !Number.isFinite(angleDegrees)) {
    throw new RangeError("A sprite look angle must be finite or null for neutral.");
  }
  if (angleDegrees === null || reducedMotion) {
    return selectSpriteFrame(PETS_V2_ATLAS, PETS_V2_CLIPS.idle, elapsedMs, { reducedMotion });
  }
  const normalized = ((angleDegrees % 360) + 360) % 360;
  const direction = Math.round(normalized / 22.5) % 16;
  const column = direction % PETS_V2_ATLAS.columns;
  return { row: 9 + Math.floor(direction / PETS_V2_ATLAS.columns), column, frameIndex: column };
}

/**
 * Pointer offset from the fixed head center, in display pixels: right is +x and
 * down is +y. The inclusive deadzone returns neutral; this never moves the pet.
 */
export function petsV2LookAngleFromOffset(offset: SpritePoint, deadzonePx = 24): number | null {
  if (!Number.isFinite(offset.x) || !Number.isFinite(offset.y)) {
    throw new RangeError("Sprite pointer offsets must be finite.");
  }
  if (!Number.isFinite(deadzonePx) || deadzonePx < 0) {
    throw new RangeError("The sprite pointer deadzone must be finite and nonnegative.");
  }
  if (Math.hypot(offset.x, offset.y) <= deadzonePx) return null;
  return (Math.atan2(offset.x, -offset.y) * 180 / Math.PI + 360) % 360;
}
