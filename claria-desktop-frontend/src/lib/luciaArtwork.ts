import manifest from "../assets/lucia/manifest.json";
import type { LuciaBehaviorConfig, LuciaBehaviorState } from "./luciaBehavior";
import { selectSpriteFrame, spriteClipDurationMs, type SpriteFrame, type SpriteStripMetadata } from "./luciaSprite";
import { logFrontendEvent } from "./logBridge";

const images = import.meta.glob<string>("../assets/lucia/**/*.png", {
  eager: true, query: "?url", import: "default",
});

export function luciaImageUrl(path: string): string {
  const url = images[`../assets/lucia/${path}`];
  if (!url) throw new Error(`Missing Lucia artwork: ${path}`);
  return url;
}

export async function loadLuciaArtwork(): Promise<boolean> {
  try {
    await Promise.all(Object.entries(manifest.assets).map(async ([path, asset]) => {
      const image = new Image();
      image.src = luciaImageUrl(path);
      await image.decode();
      if (image.naturalWidth !== asset.width || image.naturalHeight !== asset.height) {
        throw new Error("Lucia artwork dimensions do not match the manifest");
      }
    }));
    return true;
  } catch {
    logFrontendEvent("error", "Lucia artwork could not be loaded; companion stopped");
    throw new Error("Lucia’s artwork could not be loaded. Disable and re-enable distraction mode to retry.");
  }
}

export type LuciaClipName = keyof typeof manifest.clips;
export const LUCIA_SCALE = 0.65;
export const LUCIA_HEAD_ANCHOR = { x: manifest.head_anchor[0], y: manifest.head_anchor[1] };
export const LUCIA_SOCK_IMAGE = "extras/sock/sock.png";
export const LUCIA_FRUIT_IMAGE = "extras/dekopon/fruit.png";

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
    skateboard: duration("skateboard") * 4,
    chase: duration("running-right") * 2,
    pickup: duration("pickup"),
    "carry-away": duration("carry-away") * 3,
    bark: duration("bark"),
    "reading-enter": duration("reading-enter"),
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
}

/** Counts are committed by policy, not by the renderer or an animation callback. */
export function selectLuciaPose(state: LuciaBehaviorState, elapsedMs: number, gaze?: SpriteFrame): LuciaPose {
  if (state.behavior === "feeding-idle" || (state.behavior === "feeding" && (state.fedCount > 0 || state.reducedMotion))) {
    return {
      image: manifest.stages.image,
      frame: { row: 0, column: state.fedCount, frameIndex: state.fedCount },
      anchor: [manifest.stages.anchor[0], manifest.stages.ground_y_by_count[state.fedCount]],
      animated: false,
    };
  }
  const name: LuciaClipName = state.behavior === "inactive" || state.behavior === "gaze"
    ? "idle" : state.behavior === "chase" ? "running-right" : state.behavior;
  const metadata = luciaClip(name);
  const source = manifest.clips[name];
  return {
    image: source.image,
    frame: state.behavior === "gaze" && gaze && !state.reducedMotion
      ? gaze
      : selectSpriteFrame(metadata.atlas, metadata.clip, elapsedMs, {
        playback: source.loop ? "loop" : "once", reducedMotion: state.reducedMotion,
      }),
    anchor: state.behavior === "gaze" ? manifest.gaze_anchor : [source.anchor[0], source.ground_y],
    animated: state.behavior !== "inactive" && state.behavior !== "gaze" && !state.reducedMotion,
  };
}

/** The pointer never enters this geometry. Travel belongs only to deliberate socks or skating. */
export function luciaPosition(state: LuciaBehaviorState, elapsedMs: number, width: number): number {
  const home = Math.min(90, width / 2);
  if (state.reducedMotion) return home;
  const pickup = width / 2;
  const progress = state.endsAt === null ? 0 : Math.min(1, Math.max(0, elapsedMs / (state.endsAt - state.startedAt)));
  switch (state.behavior) {
    case "chase": return home + (pickup - home) * progress;
    case "pickup": return pickup;
    case "carry-away": return pickup + (width + 80 - pickup) * progress;
    case "skateboard": return width + 80 - (width + 160) * progress;
    default: return home;
  }
}
