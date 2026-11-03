import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import manifest from "../assets/lucia/manifest.json";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { logFrontendEvent } from "../lib/logBridge";
import { CloseIcon } from "./icons";
import { setDistractionModeEnabled } from "../lib/distractionMode";
import { useLuciaBehavior } from "../lib/useLuciaBehavior";
import { useLuciaGaze } from "../lib/useLuciaGaze";
import { isReading, ownsSock, type LuciaBehavior, type LuciaBehaviorState, type LuciaMode, type LuciaPreviewBehavior } from "../lib/luciaBehavior";
import type { SpriteFrame } from "../lib/luciaSprite";
import {
  type LuciaArtwork, loadLuciaArtwork, luciaBoardPosition, luciaImageUrl, luciaPosition, selectLuciaPose,
  LUCIA_BEHAVIOR_CONFIG, LUCIA_HEAD_ANCHOR, LUCIA_SCALE, LUCIA_SKATEBOARD_IMAGE, LUCIA_SOCK_IMAGE,
} from "../lib/luciaArtwork";

// Match the standalone 56×48 sock to pickup's embedded blue bounds (105,171)–(161,201).
const SOCK_SCALE = LUCIA_SCALE * 1.15;
const SOCK_WIDTH = manifest.assets[LUCIA_SOCK_IMAGE].width * SOCK_SCALE;
const SOCK_HEIGHT = manifest.assets[LUCIA_SOCK_IMAGE].height * SOCK_SCALE;
const BOARD_WIDTH = manifest.assets[LUCIA_SKATEBOARD_IMAGE].width * LUCIA_SCALE;
const BOARD_HEIGHT = manifest.assets[LUCIA_SKATEBOARD_IMAGE].height * LUCIA_SCALE;
const TRAVELLING: ReadonlySet<LuciaBehavior> = new Set(["chase", "carry-away", "board-chase", "skateboard"]);

type CompanionProps = { clientId: string; rail: HTMLDivElement | null; previewsEnabled?: boolean };

const MODES: ReadonlyArray<{ id: LuciaMode; label: string; title: string }> = [
  { id: "companion", label: "Companion", title: "Lucia keeps you company" },
  { id: "gaze", label: "Watch", title: "Lucia sits and watches your pointer" },
  { id: "feeding", label: "Feed", title: "Feed Lucia up to five dekopon" },
];

/** Rendered inside the workspace's Lucia rail; the existing machine-local preference owns this mount. */
export default function LuciaCompanion({ clientId, rail, previewsEnabled = false }: CompanionProps) {
  const artwork = useAsyncLoad(loadLuciaArtwork, []);
  if (artwork.data && rail) return <LuciaScene clientId={clientId} rail={rail} previewsEnabled={previewsEnabled} artwork={artwork.data} />;
  return (
    <div className="absolute inset-0 flex items-center gap-3 px-6 text-xs text-gray-500">
      <span role={artwork.error ? "alert" : "status"}>{artwork.error ?? "Loading Lucia…"}</span>
      <HideButton />
    </div>
  );
}

function HideButton() {
  return (
    <button type="button" onClick={() => setDistractionModeEnabled(false)}
      title="Hide Lucia (turn her back on in Preferences → Distraction mode)" aria-label="Hide Lucia"
      className="absolute right-3 top-2 p-1 rounded-md text-gray-400 hover:text-gray-700 hover:bg-gray-100">
      <CloseIcon className="w-4 h-4" />
    </button>
  );
}

function LuciaScene({ clientId, rail, previewsEnabled, artwork }: CompanionProps & { rail: HTMLDivElement; artwork: LuciaArtwork }) {
  const { state, dropSock, setMode, pet, feed, reset, preview } = useLuciaBehavior(LUCIA_BEHAVIOR_CONFIG, clientId);
  const [paintError, setPaintError] = useState<string | null>(null);
  const sockBusy = ownsSock(state.behavior);
  const available = state.active && state.visible;
  const canFeed = available && state.mode === "feeding" && state.behavior === "feeding-idle"
    && state.fedCount < 5 && !state.inferencePending;
  if (paintError) {
    return (
      <div className="absolute inset-0 flex items-center gap-3 px-6 text-xs text-gray-500">
        <span role="alert">{paintError}</span>
        <HideButton />
      </div>
    );
  }
  return (
    <>
      {available && (
        <div data-testid="lucia-scene" data-behavior={state.behavior} data-fed-count={state.fedCount}
          data-reduced-motion={state.reducedMotion} className="absolute inset-0 z-10 pointer-events-none">
          {state.mode === "gaze"
            ? <GazeSprite state={state} rail={rail} pet={pet} feed={feed} canFeed={canFeed} artwork={artwork} onPaintError={setPaintError} />
            : <LuciaSprite state={state} rail={rail} pet={pet} feed={feed} canFeed={canFeed} artwork={artwork} onPaintError={setPaintError} />}
        </div>
      )}
      <div role="group" aria-label="Lucia controls"
        className="absolute left-44 bottom-4 flex flex-wrap items-center gap-2 text-xs text-gray-600">
        <div className="flex border border-gray-200 rounded-lg overflow-hidden bg-white">
          {MODES.map((mode) => {
            const selected = state.mode === mode.id;
            return (
              <button key={mode.id} type="button" aria-pressed={selected} title={mode.title}
                onClick={() => setMode(mode.id)} disabled={!available}
                className={`px-3 py-1 font-medium transition-colors disabled:opacity-40 ${
                  selected ? "bg-blue-600 text-white" : "bg-white text-gray-600 hover:bg-gray-50"
                }`}>
                {mode.label}
              </button>
            );
          })}
        </div>
        {state.mode === "feeding" ? <>
          <span aria-live="polite">{state.fedCount === 5 ? "Full! 5/5 dekopon" : `${state.fedCount}/5 dekopon`}</span>
          <button type="button" onClick={feed} disabled={!canFeed}
            className="rounded border border-gray-300 bg-white px-2 py-1 hover:bg-gray-50 disabled:opacity-40">Feed a dekopon</button>
          {state.fedCount > 0 && <button type="button" onClick={reset} className="underline">Start over</button>}
        </> : (
          <button type="button" onClick={dropSock} disabled={!available || sockBusy}
            title="Drop a sock for Lucia" aria-label="Drop a sock for Lucia"
            className="p-1 rounded-md border border-gray-200 bg-white hover:bg-gray-50 disabled:opacity-40">
            <img src={luciaImageUrl(LUCIA_SOCK_IMAGE)} alt="" width="20" height="20" />
          </button>
        )}
        {import.meta.env.DEV && previewsEnabled && (
          <select aria-label="Preview Lucia action" value=""
            disabled={!available || state.mode !== "companion" || state.reducedMotion || state.inferencePending || sockBusy || state.behavior === "bark" || isReading(state.behavior)}
            onChange={(event) => preview(event.target.value as LuciaPreviewBehavior)}
            className="rounded border border-gray-300 bg-white p-1 disabled:opacity-40">
            <option value="" disabled>Preview an action (demo)…</option>
            <option value="tailwag">Entrance wag</option>
            <option value="stretch">Stretch then lie down</option>
            <option value="ear-perk">Ears perk</option>
            <option value="twirl">Twirl in place</option>
            <option value="skateboard">Skateboard crossing</option>
          </select>
        )}
      </div>
      <HideButton />
    </>
  );
}

function GazeSprite(props: SpriteProps) {
  const spriteRef = useRef<HTMLButtonElement>(null);
  const gaze = useLuciaGaze(spriteRef, {
    paused: props.state.behavior !== "gaze", reducedMotion: props.state.reducedMotion,
    headAnchor: LUCIA_HEAD_ANCHOR,
  });
  return <LuciaSprite {...props} spriteRef={spriteRef} gaze={gaze} />;
}

type SpriteProps = {
  state: LuciaBehaviorState; rail: HTMLDivElement; pet: () => void; feed: () => void; canFeed: boolean;
  artwork: LuciaArtwork; onPaintError: (message: string) => void;
};

function LuciaSprite({ state, rail, pet, feed, canFeed, artwork, onPaintError, gaze, spriteRef: gazeRef }: SpriteProps & {
  gaze?: SpriteFrame; spriteRef?: RefObject<HTMLButtonElement | null>;
}) {
  const ownRef = useRef<HTMLButtonElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const spriteRef = gazeRef ?? ownRef;
  const sockRef = useRef<HTMLImageElement>(null);
  const boardRef = useRef<HTMLImageElement>(null);
  const feeding = state.mode === "feeding";

  // Commit the new pose/registration before paint, together with sock visibility.
  useLayoutEffect(() => {
    const sprite = spriteRef.current;
    const canvas = canvasRef.current;
    if (!sprite || !canvas) return;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Lucia’s drawing surface is unavailable.");
    let frame: number | null = null;
    let sleep: number | null = null;
    let width = rail.clientWidth;
    let previousImage = "";
    let previousFrame = "";
    let previousLeft = "";
    let previousBottom = "";
    const paint = (): number | null => {
      const elapsed = Math.max(0, performance.now() - state.startedAt);
      const pose = selectLuciaPose(state, elapsed, gaze);
      const frameKey = `${pose.frame.row}:${pose.frame.column}`;
      if (previousImage !== pose.image || previousFrame !== frameKey) {
        const image = artwork.get(pose.image);
        if (!image) throw new Error(`Lucia artwork was not decoded: ${pose.image}`);
        // One persistent surface: source switches never wait for CSS image loading.
        // Copy a whole fixed cell at native resolution, then uniformly scale in CSS.
        context.clearRect(0, 0, manifest.cell_width, manifest.cell_height);
        context.drawImage(image,
          pose.frame.column * manifest.cell_width, pose.frame.row * manifest.cell_height,
          manifest.cell_width, manifest.cell_height, 0, 0, manifest.cell_width, manifest.cell_height);
        sprite.dataset.image = previousImage = pose.image;
        sprite.dataset.frame = previousFrame = frameKey;
      }
      const left = `${luciaPosition(state, elapsed, width) - pose.anchor[0] * LUCIA_SCALE}px`;
      const bottom = `${6 - (manifest.cell_height - pose.anchor[1]) * LUCIA_SCALE}px`;
      if (previousLeft !== left) sprite.style.left = previousLeft = left;
      if (previousBottom !== bottom) sprite.style.bottom = previousBottom = bottom;
      if (sockRef.current) {
        // Register the standalone sock to the embedded sock in pickup frame zero.
        const pickupX = state.reducedMotion ? Math.min(90, width / 2) : width / 2;
        sockRef.current.style.left = `${pickupX + (133 - 96) * LUCIA_SCALE - SOCK_WIDTH / 2}px`;
      }
      if (boardRef.current) boardRef.current.style.left = `${luciaBoardPosition(state, elapsed, width) - BOARD_WIDTH / 2}px`;
      // Travel moves every display frame; an in-place animation only needs its next cell.
      if (!state.reducedMotion && TRAVELLING.has(state.behavior)) return 0;
      return pose.nextFrameInMs;
    };
    // A paint failure inside rAF would escape React entirely; surface it in the rail instead.
    const safePaint = (): number | null => {
      try {
        return paint();
      } catch (error) {
        logFrontendEvent("error", "Lucia could not draw a frame; companion stopped");
        onPaintError(error instanceof Error ? error.message : "Lucia could not be drawn.");
        return null;
      }
    };
    const tick = () => {
      frame = null;
      sleep = null;
      const next = safePaint();
      if (next === null) return;
      if (next === 0) frame = requestAnimationFrame(tick);
      else sleep = window.setTimeout(() => { sleep = null; frame = requestAnimationFrame(tick); }, next);
    };
    const observer = new ResizeObserver(() => { width = rail.clientWidth; safePaint(); });
    observer.observe(rail);
    tick();
    return () => {
      observer.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
      if (sleep !== null) window.clearTimeout(sleep);
    };
  }, [artwork, gaze, onPaintError, rail, spriteRef, state]);

  return <>
    {/* Behind Lucia: the board rolls past her wheels-down on her ground line. */}
    {state.behavior === "board-chase" && <img ref={boardRef} src={luciaImageUrl(LUCIA_SKATEBOARD_IMAGE)} alt=""
      data-testid="lucia-board" className="absolute pointer-events-none" style={{ width: BOARD_WIDTH, height: BOARD_HEIGHT, bottom: 6 }} />}
    {/* Clicking Lucia does what the mode is for: feed while feeding, otherwise pet her (barks, then play). */}
    <button ref={spriteRef} type="button" onClick={feeding ? feed : pet}
      aria-label={feeding ? (state.fedCount === 5 ? "Lucia is full" : "Feed Lucia a dekopon") : "Pet Lucia"}
      title={feeding ? (state.fedCount === 5 ? "Lucia is full" : "Feed Lucia a dekopon") : "Pet Lucia"}
      disabled={feeding ? !canFeed : state.inferencePending || ownsSock(state.behavior) || state.petReacting}
      className="absolute pointer-events-auto rounded-lg cursor-pointer disabled:cursor-default focus-visible:outline-2 focus-visible:outline-blue-600"
      style={{ width: manifest.cell_width * LUCIA_SCALE, height: manifest.cell_height * LUCIA_SCALE }}
      data-testid="lucia-sprite">
      <canvas ref={canvasRef} width={manifest.cell_width} height={manifest.cell_height}
        aria-hidden="true" className="block h-full w-full pointer-events-none" />
    </button>
    {state.behavior === "chase" && <img ref={sockRef} src={luciaImageUrl(LUCIA_SOCK_IMAGE)} alt=""
      data-testid="lucia-sock" className="absolute" style={{ width: SOCK_WIDTH, height: SOCK_HEIGHT, bottom: 0, transform: "rotate(-60deg)" }} />}
  </>;
}
