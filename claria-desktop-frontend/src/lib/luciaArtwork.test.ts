import { createHash } from "node:crypto";
import { inflateSync } from "node:zlib";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import manifest from "../assets/lucia/manifest.json";
import { createLuciaBehavior, type LuciaBehaviorState } from "./luciaBehavior";
import { LUCIA_BEHAVIOR_CONFIG, loadLuciaArtwork, luciaClip, luciaPosition, selectLuciaPose, type LuciaClipName } from "./luciaArtwork";
import { PETS_V2_ATLAS, PETS_V2_CLIPS, selectPetsV2LookFrame, selectSpriteFrame, spriteClipDurationMs } from "./luciaSprite";

const root = join(dirname(fileURLToPath(import.meta.url)), "../assets/lucia");
const state = (overrides: Partial<LuciaBehaviorState>): LuciaBehaviorState => ({
  ...createLuciaBehavior(0), active: true, behavior: "idle", ...overrides,
});

/** Decode the shipped RGBA PNGs so registration tests check pixels, not a second metadata copy. */
function rgbaPixels(path: string) {
  const png = readFileSync(join(root, path));
  const width = png.readUInt32BE(16);
  const height = png.readUInt32BE(20);
  expect([png[24], png[25], png[28]]).toEqual([8, 6, 0]);
  const chunks: Buffer[] = [];
  for (let offset = 8; offset < png.length;) {
    const length = png.readUInt32BE(offset);
    if (png.toString("ascii", offset + 4, offset + 8) === "IDAT") chunks.push(png.subarray(offset + 8, offset + 8 + length));
    offset += length + 12;
  }
  const raw = inflateSync(Buffer.concat(chunks));
  const stride = width * 4;
  const pixels = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    expect(filter).toBeLessThanOrEqual(4);
    for (let x = 0; x < stride; x++) {
      const i = y * stride + x;
      const left = x >= 4 ? pixels[i - 4] : 0;
      const above = y > 0 ? pixels[i - stride] : 0;
      const corner = x >= 4 && y > 0 ? pixels[i - stride - 4] : 0;
      const prediction = left + above - corner;
      const distances = [Math.abs(prediction - left), Math.abs(prediction - above), Math.abs(prediction - corner)];
      const paeth = distances[0] <= distances[1] && distances[0] <= distances[2] ? left : distances[1] <= distances[2] ? above : corner;
      const correction = [0, left, above, Math.floor((left + above) / 2), paeth][filter];
      pixels[i] = (raw[y * (stride + 1) + x + 1] + correction) & 255;
    }
  }
  return { pixels, width };
}

function opaqueBottom(image: ReturnType<typeof rgbaPixels>, row: number, column: number) {
  for (let y = 207; y >= 0; y--) {
    for (let x = 0; x < 192; x++) {
      if (image.pixels[((row * 208 + y) * image.width + column * 192 + x) * 4 + 3] >= manifest.registration.alpha_threshold) return y + 1;
    }
  }
  throw new Error("Blank runtime cell");
}

describe("Lucia imported synthetic artwork", () => {
  it("registers actual opaque contact instead of the supplied transparent-padding origins", () => {
    const images = new Map<string, ReturnType<typeof rgbaPixels>>();
    const image = (path: string) => {
      if (!images.has(path)) images.set(path, rgbaPixels(path));
      return images.get(path)!;
    };
    for (const clip of Object.values(manifest.clips)) {
      const contacts = clip.frame_durations_ms.map((_, i) => opaqueBottom(image(clip.image), clip.row, clip.first_column + i));
      expect(clip.ground_y).toBe(Math.max(...contacts));
    }
    // Regression: the lying strip's source origin leaves 42 pixels of empty space below its paws.
    expect(manifest.clips.bored.anchor[1]).toBe(202);
    expect(manifest.clips.bored.ground_y).toBe(161);
    expect(selectLuciaPose(state({ behavior: "bored" }), 0).anchor[1]).toBe(161);
    expect(manifest.clips["running-right"].ground_y).toBe(181);
    for (const count of manifest.stages.counts) {
      expect(manifest.stages.ground_y_by_count[count]).toBe(opaqueBottom(image(manifest.stages.image), 0, count));
    }
    for (let i = 0; i < 16; i++) {
      expect(opaqueBottom(image(manifest.core_image), 9 + Math.floor(i / 8), i % 8)).toBe(manifest.gaze_anchor[1]);
    }
  });

  it("ships only checksummed runtime PNGs and their manifest, not previews or source QA", () => {
    expect(manifest.style).toBe("approved synthetic-v3");
    const files = readdirSync(root, { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) => relative(root, join(entry.parentPath, entry.name)).split(sep).join("/")).sort();
    expect(files).toEqual([...Object.keys(manifest.assets), "manifest.json"].sort());
    for (const [path, asset] of Object.entries(manifest.assets)) {
      expect(path).not.toMatch(/(^\/|\.\.|previews|reference|qa)/);
      const png = readFileSync(join(root, path));
      expect(createHash("sha256").update(png).digest("hex")).toBe(asset.sha256);
      expect(png.subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
      expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([asset.width, asset.height]);
      expect(png[25]).toBe(6); // RGBA, not flattened against a background.
    }
  });

  it("keeps every used rectangle, timing and anchor inside its registered sheet", () => {
    for (const name of Object.keys(manifest.clips) as LuciaClipName[]) {
      const metadata = luciaClip(name);
      const source = manifest.clips[name];
      expect(source.anchor[0]).toBeGreaterThanOrEqual(0);
      expect(source.anchor[0]).toBeLessThan(metadata.atlas.cellWidth);
      expect(metadata.baselineY).toBeGreaterThanOrEqual(0);
      expect(metadata.baselineY).toBeLessThan(metadata.atlas.cellHeight);
      let elapsed = 0;
      for (let i = 0; i < source.frame_durations_ms.length; i++) {
        const frame = selectSpriteFrame(metadata.atlas, metadata.clip, elapsed, { playback: "once" });
        expect(frame).toEqual({ row: source.row, column: source.first_column + i, frameIndex: i });
        elapsed += source.frame_durations_ms[i];
      }
      expect(spriteClipDurationMs(metadata.clip)).toBe(elapsed);
    }
    expect(luciaClip("idle").atlas).toEqual(PETS_V2_ATLAS);
    expect(luciaClip("idle").clip).toEqual(PETS_V2_CLIPS.idle);
    expect(luciaClip("running-right").clip).toEqual(PETS_V2_CLIPS["running-right"]);
    expect(manifest.gaze_degrees.map((angle) => selectPetsV2LookFrame(angle).row)).toEqual([...Array(8).fill(9), ...Array(8).fill(10)]);
    expect(LUCIA_BEHAVIOR_CONFIG.durationsMs.tailwag).toBe(840);
    expect(LUCIA_BEHAVIOR_CONFIG.durationsMs.pickup).toBe(1180);
  });
});

describe("Lucia policy-to-art mapping", () => {
  it("holds original idle cell zero at a fixed anchor instead of looping source width changes", () => {
    const original = selectLuciaPose(state({}), 0);
    expect(original).toMatchObject({ image: manifest.core_image, frame: { row: 0, column: 0 }, anchor: [96, 201], animated: false });
    for (const elapsed of [279, 280, 390, 500, 640, 780, 1100, 30_000]) {
      expect(selectLuciaPose(state({}), elapsed)).toEqual(original);
      expect(luciaPosition(state({}), elapsed, 1280)).toBe(90);
    }
  });

  it("maps every current feeding count to its own non-looping six-frame strip", () => {
    expect(manifest.feeding_by_count).toEqual(["feeding", "feeding-size-1", "feeding-size-2", "feeding-size-3", "feeding-size-4"]);
    expect(manifest.stages.counts).toEqual([0, 1, 2, 3, 4, 5]);
    for (const fedCount of [0, 1, 2, 3, 4] as const) {
      const name = manifest.feeding_by_count[fedCount] as LuciaClipName;
      const image = `extras/dekopon/${fedCount === 0 ? "feeding" : `feeding-size-${fedCount}`}/feeding-strip.png`;
      expect(spriteClipDurationMs(luciaClip(name).clip)).toBe(LUCIA_BEHAVIOR_CONFIG.durationsMs.feeding);
      for (const [elapsed, column] of [[0, 0], [149, 0], [150, 1], [300, 2], [450, 3], [600, 4], [750, 5], [1049, 5], [1050, 5], [20_000, 5]]) {
        expect(selectLuciaPose(state({ behavior: "feeding", fedCount }), elapsed)).toMatchObject({
          image, frame: { row: 0, column }, anchor: [96, manifest.clips[name].ground_y], animated: true,
        });
        expect(selectLuciaPose(state({ behavior: "feeding", fedCount, reducedMotion: true }), elapsed)).toMatchObject({
          image: manifest.stages.image, frame: { column: fedCount }, animated: false,
        });
      }
      expect(selectLuciaPose(state({ behavior: "feeding-idle", fedCount }), 0).frame.column).toBe(fedCount);
    }
    expect(() => selectLuciaPose(state({ behavior: "feeding", fedCount: 5 }), 0)).toThrow("beyond five");
    expect(selectLuciaPose(state({ behavior: "full", fedCount: 5 }), 0).image).toContain("full-strip");
  });

  it("keeps later feeding silhouettes at least as wide as their starting stage with fixed contact", () => {
    const stages = rgbaPixels(manifest.stages.image);
    const width = (image: ReturnType<typeof rgbaPixels>, column: number) => {
      const occupied: number[] = [];
      for (let x = 0; x < 192; x++) {
        for (let y = 0; y < 208; y++) {
          if (image.pixels[(y * image.width + column * 192 + x) * 4 + 3] >= manifest.registration.alpha_threshold) {
            occupied.push(x);
            break;
          }
        }
      }
      return occupied.at(-1)! - occupied[0] + 1;
    };
    for (const count of [1, 2, 3, 4]) {
      const source = manifest.clips[manifest.feeding_by_count[count] as LuciaClipName];
      const image = rgbaPixels(source.image);
      for (let frame = 0; frame < 6; frame++) {
        expect(opaqueBottom(image, 0, frame)).toBe(source.ground_y);
        expect(width(image, frame)).toBeGreaterThanOrEqual(width(stages, count));
      }
      expect(width(image, 5)).toBeLessThanOrEqual(width(stages, count + 1));
    }
  });

  it("anchors every gaze direction and has no pointer-driven travel", () => {
    const gazeState = state({ behavior: "gaze", mode: "gaze" });
    for (const angle of manifest.gaze_degrees) {
      const frame = selectPetsV2LookFrame(angle);
      expect(selectLuciaPose(gazeState, 50_000, frame)).toMatchObject({ frame, anchor: [96, 202], animated: false });
      expect(luciaPosition(gazeState, 50_000, 1000)).toBe(90);
    }
  });

  it("uses actual newspaper strips, not inherited core waiting labels", () => {
    for (const behavior of ["reading-enter", "reading", "reading-exit"] as const) {
      expect(selectLuciaPose(state({ behavior }), 0).image).toContain("newspaper/");
    }
  });

  it("bounds sock travel and suppresses all travel and repetitive frames for reduced motion", () => {
    const chase = state({ behavior: "chase", endsAt: 1000 });
    expect(luciaPosition(chase, 0, 1000)).toBe(90);
    expect(luciaPosition(chase, 1000, 1000)).toBe(500);
    expect(luciaPosition(state({ behavior: "pickup" }), 0, 1000)).toBe(500);
    for (const behavior of ["chase", "pickup", "carry-away", "skateboard", "reading", "feeding", "bark"] as const) {
      const reduced = state({ behavior, reducedMotion: true, endsAt: 1000 });
      expect(luciaPosition(reduced, 900, 1000)).toBe(90);
      expect(selectLuciaPose(reduced, 900).animated).toBe(false);
      expect(selectLuciaPose(reduced, 900).frame.column).toBe(0);
    }
  });
});


describe("Lucia decoded artwork lifetime", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("returns every retained source only after all images decode", async () => {
    const assets = Object.values(manifest.assets);
    const images: ImageStub[] = [];
    const finish: Array<() => void> = [];
    class ImageStub {
      src = "";
      naturalWidth: number;
      naturalHeight: number;
      constructor() {
        const asset = assets[images.length];
        this.naturalWidth = asset.width;
        this.naturalHeight = asset.height;
        images.push(this);
      }
      decode() { return new Promise<void>((resolve) => finish.push(resolve)); }
    }
    vi.stubGlobal("Image", ImageStub);
    let ready = false;
    const loading = loadLuciaArtwork().then((artwork) => { ready = true; return artwork; });
    expect(images).toHaveLength(assets.length);
    for (const resolve of finish.slice(0, -1)) resolve();
    await Promise.resolve();
    expect(ready).toBe(false);
    finish.at(-1)!();
    const artwork = await loading;
    expect([...artwork.values()]).toEqual(images);
    expect([...artwork.keys()]).toEqual(Object.keys(manifest.assets));
  });
});
