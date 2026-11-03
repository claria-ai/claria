import manifest from "../assets/lucia/manifest.json";
import type { LuciaBehaviorConfig, LuciaBehaviorState } from "./luciaBehavior";
import { selectSpriteFrame, spriteClipDurationMs, spriteFrameRemainingMs, type SpriteFrame, type SpriteStripMetadata } from "./luciaSprite";
import { logFrontendEvent } from "./logBridge";

const images = import.meta.glob<string>("../assets/lucia/**/*.png", {
  eager: true, query: "?url", import: "default",
});

export function luciaImageUrl(path: string): string {
  const url = images[`../assets/lucia/${path}`];
  if (!url) throw new Error(`Missing Lucia artwork: ${path}`);
  return url;
}

export type LuciaArtwork = ReadonlyMap<string, HTMLImageElement>;

let artworkLoad: Promise<LuciaArtwork> | null = null;

/**
 * Keep decoded sources alive; changing CSS URLs can still paint a blank frame.
 * Decoded once per app session (switching clients remounts the scene); a failed
 * load is forgotten so re-enabling distraction mode retries it.
 */
export function loadLuciaArtwork(): Promise<LuciaArtwork> {
  artworkLoad ??= decodeLuciaArtwork().catch((error: unknown) => {
    artworkLoad = null;
    throw error;
  });
  return artworkLoad;
}

async function decodeLuciaArtwork(): Promise<LuciaArtwork> {
  try {
    return new Map(await Promise.all(Object.entries(manifest.assets).map(async ([path, asset]) => {
      const image = new Image();
      image.src = luciaImageUrl(path);
      await image.decode();
      if (image.naturalWidth !== asset.width || image.naturalHeight !== asset.height) {
        throw new Error("Lucia artwork dimensions do not match the manifest");
      }
      return [path, image] as const;
    })));
  } catch {
    logFrontendEvent("error", "Lucia artwork could not be loaded; companion stopped");
    throw new Error("Lucia’s artwork could not be loaded. Disable and re-enable distraction mode to retry.");
  }
}

export type LuciaClipName = keyof typeof manifest.clips;
export const LUCIA_SCALE = 0.65;
export const LUCIA_HEAD_ANCHOR = { x: manifest.head_anchor[0], y: manifest.head_anchor[1] };
export const LUCIA_SOCK_IMAGE = "extras/sock/sock.png";
/**
 * Cropped and scaled to the board drawn in skateboard-left frame 0: deck x 17–163
 * (146 px) with its wheels on the clip's ground line, y 200. Drawn at LUCIA_SCALE.
 */
export const LUCIA_SKATEBOARD_IMAGE = "extras/play/skateboard/board.png";
/** The share of the board chase Lucia watches before she runs after it. */
export const LUCIA_BOARD_CHASE_RUN_START = 0.3;

export function luciaClip(name: LuciaClipName): SpriteStripMetadata {
  const clip = manifest.clips[name];
  const image = manifest.assets[clip.image as keyof typeof manifest.assets];
  return {
    atlas: {
      columns: image.width / manifest.cell_width,
      rows: image.height / manifest.cell_height,
      cellWidth: manifest.cell_width,
      cellHeight: manifest.cell_height,
    },
    clip: { row: clip.row, firstColumn: clip.first_column, frameDurationsMs: clip.frame_durations_ms },
    baselineY: clip.ground_y,
  };
}
const duration = (name: LuciaClipName) => spriteClipDurationMs(luciaClip(name).clip);

/** Looped entrance/carry/skate strips get a bounded number of cycles in policy. */
export const LUCIA_BEHAVIOR_CONFIG: LuciaBehaviorConfig = {
  durationsMs: {
    tailwag: duration("tailwag"),
    "ear-perk": duration("ear-perk"),
    twirl: duration("twirl"),
    stretch: duration("stretch"),
    "board-chase": duration("running-right") * 4,
    skateboard: duration("skateboard") * 4,
    chase: duration("running-right") * 2,
    pickup: duration("pickup"),
    "carry-away": duration("carry-away") * 3,
    bark: duration("bark"),
    "reading-enter": duration("reading-enter"),
    "reading-play": duration("reading") * 3,
    "reading-exit": duration("reading-exit"),
    feeding: duration("feeding"),
    full: duration("full"),
  },
};

export interface LuciaPose {
  image: string;
  frame: SpriteFrame;
  anchor: readonly number[];
  animated: boolean;
  /** When the frame next changes; null if it holds. Lets the renderer sleep between frames. */
  nextFrameInMs: number | null;
}

/** Counts are committed by policy, not by the renderer or an animation callback. */
export function selectLuciaPose(state: LuciaBehaviorState, elapsedMs: number, gaze?: SpriteFrame): LuciaPose {
  const feedingClip = manifest.feeding_by_count[state.fedCount] as LuciaClipName;
  if (state.behavior === "feeding" && !feedingClip) throw new RangeError("Lucia cannot feed beyond five fruits");
  if (state.behavior === "feeding-idle" || (state.behavior === "feeding" && state.reducedMotion)) {
    return {
      image: manifest.stages.image,
      frame: { row: 0, column: state.fedCount, frameIndex: state.fedCount },
      anchor: [manifest.stages.anchor[0], manifest.stages.ground_y_by_count[state.fedCount]],
      animated: false,
      nextFrameInMs: null,
    };
  }
  // The board chase watches the board roll in (idle), then runs after it.
  const runStartMs = state.behavior === "board-chase" && state.endsAt !== null
    ? (state.endsAt - state.startedAt) * LUCIA_BOARD_CHASE_RUN_START : 0;
  const name: LuciaClipName = state.behavior === "inactive" || state.behavior === "gaze"
    ? "idle" : state.behavior === "chase" ? "running-right"
      : state.behavior === "board-chase" ? (elapsedMs < runStartMs ? "idle" : "running-right")
        : state.behavior === "reading-play" ? "reading"
          : state.behavior === "feeding" ? feedingClip : state.behavior;
  const clipElapsedMs = name === "running-right" ? elapsedMs - runStartMs : elapsedMs;
  const metadata = luciaClip(name);
  const source = manifest.clips[name];
  // Gaze frames come from the pointer, not the clock.
  const animated = !state.reducedMotion && state.behavior !== "inactive" && state.behavior !== "gaze";
  const playback = { playback: source.loop ? "loop" : "once", reducedMotion: !animated } as const;
  const frame = state.behavior === "gaze" && gaze && !state.reducedMotion
    ? gaze
    : selectSpriteFrame(metadata.atlas, metadata.clip, clipElapsedMs, playback);
  // Gaze's neutral pose is idle cell 0: register it on idle's measured ground, not the gaze rows'.
  const anchor = state.behavior === "gaze" && frame.row !== manifest.clips.idle.row
    ? manifest.gaze_anchor : [source.anchor[0], source.ground_y];
  return {
    image: source.image,
    frame,
    anchor,
    animated,
    nextFrameInMs: animated ? spriteFrameRemainingMs(metadata.clip, clipElapsedMs, playback) : null,
  };
}

function behaviorProgress(state: LuciaBehaviorState, elapsedMs: number): number {
  return state.endsAt === null ? 0 : Math.min(1, Math.max(0, elapsedMs / (state.endsAt - state.startedAt)));
}

const BOARD_HALF_WIDTH = (manifest.assets[LUCIA_SKATEBOARD_IMAGE].width * LUCIA_SCALE) / 2;

/** The pointer never enters this geometry. Travel belongs only to deliberate socks or skating. */
export function luciaPosition(state: LuciaBehaviorState, elapsedMs: number, width: number): number {
  const home = Math.min(90, width / 2);
  if (state.reducedMotion) return home;
  const pickup = width / 2;
  const progress = behaviorProgress(state, elapsedMs);
  const run = (progress - LUCIA_BOARD_CHASE_RUN_START) / (1 - LUCIA_BOARD_CHASE_RUN_START);
  switch (state.behavior) {
    case "board-chase": return run <= 0 ? home : home + (width + 80 - home) * run;
    case "chase": return home + (pickup - home) * progress;
    case "pickup": return pickup;
    case "carry-away": return pickup + (width + 80 - pickup) * progress;
    case "skateboard": return width + 80 - (width + 160) * progress;
    default: return home;
  }
}

/** How far ahead of Lucia's centre the board's centre rolls once she is chasing it. */
const BOARD_LEAD = 70;

/**
 * The board's centre during a board chase: it rolls in from the left edge past
 * Lucia, and once she starts running it stays BOARD_LEAD ahead of her, off the right edge.
 */
export function luciaBoardPosition(state: LuciaBehaviorState, elapsedMs: number, width: number): number {
  const home = Math.min(90, width / 2);
  const progress = behaviorProgress(state, elapsedMs);
  if (progress >= LUCIA_BOARD_CHASE_RUN_START) return luciaPosition(state, elapsedMs, width) + BOARD_LEAD;
  return -BOARD_HALF_WIDTH + (home + BOARD_LEAD + BOARD_HALF_WIDTH) * (progress / LUCIA_BOARD_CHASE_RUN_START);
}
