import { useLayoutEffect, useRef, type RefObject } from "react";
import { createPortal } from "react-dom";
import manifest from "../assets/lucia/manifest.json";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { setDistractionModeEnabled } from "../lib/distractionMode";
import { useLuciaBehavior } from "../lib/useLuciaBehavior";
import { useLuciaGaze } from "../lib/useLuciaGaze";
import type { LuciaBehaviorState, LuciaMode } from "../lib/luciaBehavior";
import type { SpriteFrame } from "../lib/luciaSprite";
import {
  loadLuciaArtwork, luciaImageUrl, luciaPosition, selectLuciaPose,
  LUCIA_BEHAVIOR_CONFIG, LUCIA_FRUIT_IMAGE, LUCIA_HEAD_ANCHOR, LUCIA_SCALE, LUCIA_SOCK_IMAGE,
} from "../lib/luciaArtwork";

// Match the standalone 56×48 sock to pickup's embedded blue bounds (105,171)–(161,201).
const SOCK_SCALE = LUCIA_SCALE * 1.15;
const SOCK_WIDTH = manifest.assets[LUCIA_SOCK_IMAGE].width * SOCK_SCALE;
const SOCK_HEIGHT = manifest.assets[LUCIA_SOCK_IMAGE].height * SOCK_SCALE;

/** The existing machine-local preference owns this mount, including every timer and input listener. */
export default function LuciaCompanion({ clientId, rail, controlsInitiallyOpen = false }: { clientId: string; rail: HTMLDivElement | null; controlsInitiallyOpen?: boolean }) {
  const artwork = useAsyncLoad(loadLuciaArtwork, []);
  if (artwork.data && rail) return <LuciaScene clientId={clientId} rail={rail} controlsInitiallyOpen={controlsInitiallyOpen} />;
  return (
    <div className="flex items-center gap-2 text-xs text-gray-500">
      <span role={artwork.error ? "alert" : "status"}>{artwork.error ?? "Loading Lucia…"}</span>
      <button type="button" onClick={() => setDistractionModeEnabled(false)}>Hide Lucia</button>
    </div>
  );
}

function LuciaScene({ clientId, rail, controlsInitiallyOpen }: { clientId: string; rail: HTMLDivElement; controlsInitiallyOpen: boolean }) {
  const { state, dropSock, setMode, bark, feed, reset } = useLuciaBehavior(LUCIA_BEHAVIOR_CONFIG, clientId);
  const sockBusy = ["chase", "pickup", "carry-away"].includes(state.behavior);
  const available = state.active && state.visible;
  const disabled = !available || state.mode === "feeding" || sockBusy;
  return (
    <>
      <button type="button" onClick={dropSock} disabled={disabled}
        title="Drop a sock for Lucia" aria-label="Drop a sock for Lucia"
        className="-ml-1 p-1.5 rounded-md hover:bg-gray-100 disabled:opacity-40">
        <img src={luciaImageUrl(LUCIA_SOCK_IMAGE)} alt="" width="20" height="20" />
      </button>
      <details open={controlsInitiallyOpen || undefined} className="relative text-xs text-gray-600">
        <summary className="cursor-pointer rounded p-1.5 hover:bg-gray-100">Lucia</summary>
        <div className="absolute left-0 top-full z-20 mt-2 w-56 rounded-lg border border-gray-200 bg-white p-3 shadow-lg space-y-3">
          <label className="block">Lucia mode
            <select aria-label="Lucia mode" value={state.mode}
              onChange={(event) => setMode(event.target.value as LuciaMode)}
              className="mt-1 w-full rounded border border-gray-300 p-1">
              <option value="companion">Companion</option>
              <option value="gaze">Seated gaze</option>
              <option value="feeding">Dekopon feeding</option>
            </select>
          </label>
          {state.mode === "feeding" && <>
            <p>Dekopon: {state.fedCount}/5</p>
            <button type="button" onClick={feed}
              disabled={!available || state.behavior !== "feeding-idle" || state.fedCount === 5 || state.inferencePending}
              className="rounded border border-gray-300 px-2 py-1 disabled:opacity-40">Feed a dekopon</button>
            <button type="button" onClick={reset} className="ml-2 underline">Reset feeding</button>
            <p>Reset, leaving feeding mode or hiding Lucia clears the count. Switching away from this client also resets it.</p>
          </>}
          <p>Silent companion. Reduced motion follows your system setting.</p>
          <button type="button" onClick={() => setDistractionModeEnabled(false)} className="underline">Hide Lucia</button>
        </div>
      </details>
      {createPortal(available ? (
        <div data-testid="lucia-scene" data-behavior={state.behavior} data-fed-count={state.fedCount}
          data-reduced-motion={state.reducedMotion} className="absolute inset-0 pointer-events-none">
          {state.mode === "gaze"
            ? <GazeSprite state={state} rail={rail} bark={bark} />
            : <LuciaSprite state={state} rail={rail} bark={bark} />}
        </div>
      ) : null, rail)}
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

type SpriteProps = { state: LuciaBehaviorState; rail: HTMLDivElement; bark: () => void };

function LuciaSprite({ state, rail, bark, gaze, spriteRef: gazeRef }: SpriteProps & {
  gaze?: SpriteFrame; spriteRef?: RefObject<HTMLButtonElement | null>;
}) {
  const ownRef = useRef<HTMLButtonElement>(null);
  const spriteRef = gazeRef ?? ownRef;
  const sockRef = useRef<HTMLImageElement>(null);
  const fruitRef = useRef<HTMLImageElement>(null);

  // Commit the new pose/registration before paint, together with sock visibility.
  useLayoutEffect(() => {
    const sprite = spriteRef.current;
    if (!sprite) return;
    let frame: number | null = null;
    let width = rail.clientWidth;
    let previousImage = "";
    let previousFrame = "";
    let previousLeft = "";
    let previousBottom = "";
    const paint = () => {
      const elapsed = Math.max(0, performance.now() - state.startedAt);
      const pose = selectLuciaPose(state, elapsed, gaze);
      const sheet = manifest.assets[pose.image as keyof typeof manifest.assets];
      if (previousImage !== pose.image) {
        sprite.style.backgroundImage = `url("${luciaImageUrl(pose.image)}")`;
        sprite.style.backgroundSize = `${sheet.width * LUCIA_SCALE}px ${sheet.height * LUCIA_SCALE}px`;
        sprite.dataset.image = pose.image;
        previousImage = pose.image;
      }
      const frameKey = `${pose.frame.row}:${pose.frame.column}`;
      if (previousFrame !== frameKey) {
        sprite.style.backgroundPosition = `${-pose.frame.column * 192 * LUCIA_SCALE}px ${-pose.frame.row * 208 * LUCIA_SCALE}px`;
        sprite.dataset.frame = frameKey;
        previousFrame = frameKey;
      }
      const left = `${luciaPosition(state, elapsed, width) - pose.anchor[0] * LUCIA_SCALE}px`;
      const bottom = `${6 - (208 - pose.anchor[1]) * LUCIA_SCALE}px`;
      if (previousLeft !== left) sprite.style.left = previousLeft = left;
      if (previousBottom !== bottom) sprite.style.bottom = previousBottom = bottom;
      if (sockRef.current) {
        // Register the standalone sock to the embedded sock in pickup frame zero.
        const pickupX = state.reducedMotion ? Math.min(90, width / 2) : width / 2;
        sockRef.current.style.left = `${pickupX + (133 - 96) * LUCIA_SCALE - SOCK_WIDTH / 2}px`;
      }
      if (fruitRef.current) {
        fruitRef.current.style.left = `${Math.min(90, width / 2) + 50}px`;
      }
      return pose.animated || (!state.reducedMotion && ["chase", "carry-away", "skateboard"].includes(state.behavior));
    };
    const tick = () => {
      frame = null;
      if (paint()) frame = requestAnimationFrame(tick);
    };
    const observer = new ResizeObserver(() => { width = rail.clientWidth; paint(); });
    observer.observe(rail);
    tick();
    return () => {
      observer.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [gaze, rail, spriteRef, state]);

  return <>
    <button ref={spriteRef} type="button" onClick={bark} aria-label="Lucia — bark (silent)"
      disabled={state.mode === "feeding" || state.inferencePending || ["chase", "pickup", "carry-away", "bark"].includes(state.behavior)}
      className="absolute pointer-events-auto rounded-lg focus-visible:outline-2 focus-visible:outline-blue-600"
      style={{ width: 192 * LUCIA_SCALE, height: 208 * LUCIA_SCALE, backgroundRepeat: "no-repeat" }}
      data-testid="lucia-sprite" />
    {state.behavior === "chase" && <img ref={sockRef} src={luciaImageUrl(LUCIA_SOCK_IMAGE)} alt=""
      data-testid="lucia-sock" className="absolute" style={{ width: SOCK_WIDTH, height: SOCK_HEIGHT, bottom: 0, transform: "rotate(-60deg)" }} />}
    {state.behavior === "feeding" && state.fedCount > 0 && <img ref={fruitRef} src={luciaImageUrl(LUCIA_FRUIT_IMAGE)} alt=""
      data-testid="lucia-fruit" className="absolute bottom-2" width="28" height="28" />}
  </>;
}
