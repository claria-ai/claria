import { useEffect, useRef, useState, type RefObject } from "react";
import {
  PETS_V2_ATLAS,
  petsV2LookAngleFromOffset,
  selectPetsV2LookFrame,
  type SpriteFrame,
  type SpritePoint,
} from "./luciaSprite";

const NEUTRAL = selectPetsV2LookFrame(null);

/** Mount only for seated gaze mode; mode exit unmounts this tracker and its pointer history. */
export function useLuciaGaze(
  spriteRef: RefObject<HTMLElement | null>,
  {
    paused = false,
    reducedMotion,
    headAnchor,
    deadzonePx = 24,
  }: {
    /** Keep the pointer current during bark/twirl/reading without updating gaze poses. */
    paused?: boolean;
    reducedMotion: boolean;
    /** Measured head center in the approved Pets v2 cell, not a guessed body center. */
    headAnchor: SpritePoint;
    deadzonePx?: number;
  },
): SpriteFrame {
  const [frame, setFrame] = useState<SpriteFrame>(NEUTRAL);
  const pointer = useRef<SpritePoint | null>(null);

  useEffect(() => {
    let pendingFrame: number | null = null;

    const cancelFrame = () => {
      if (pendingFrame !== null) cancelAnimationFrame(pendingFrame);
      pendingFrame = null;
    };
    const update = () => {
      pendingFrame = null;
      if (document.hidden || !pointer.current || !spriteRef.current) return;
      const bounds = spriteRef.current.getBoundingClientRect();
      if (bounds.width <= 0 || bounds.height <= 0) return;
      const angle = petsV2LookAngleFromOffset({
        x: pointer.current.x - bounds.left - headAnchor.x * bounds.width / PETS_V2_ATLAS.cellWidth,
        y: pointer.current.y - bounds.top - headAnchor.y * bounds.height / PETS_V2_ATLAS.cellHeight,
      }, deadzonePx);
      const next = selectPetsV2LookFrame(angle);
      setFrame((previous) =>
        previous.row === next.row && previous.column === next.column ? previous : next,
      );
    };
    const schedule = () => {
      if (!paused && !reducedMotion && !document.hidden && pointer.current && pendingFrame === null) {
        pendingFrame = requestAnimationFrame(update);
      }
    };
    const onPointerMove = (event: PointerEvent) => {
      if (event.pointerType !== "mouse") return;
      pointer.current = { x: event.clientX, y: event.clientY };
      schedule();
    };
    const onLeave = () => {
      cancelFrame();
      pointer.current = null;
      setFrame(NEUTRAL);
    };
    const onVisibility = () => {
      if (document.hidden) cancelFrame();
      else schedule();
    };

    window.addEventListener("pointermove", onPointerMove, { passive: true });
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    window.addEventListener("blur", onLeave);
    document.addEventListener("pointerleave", onLeave);
    document.addEventListener("visibilitychange", onVisibility);
    schedule();
    return () => {
      cancelFrame();
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      window.removeEventListener("blur", onLeave);
      document.removeEventListener("pointerleave", onLeave);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [deadzonePx, headAnchor.x, headAnchor.y, paused, reducedMotion, spriteRef]);

  return !paused && !reducedMotion ? frame : NEUTRAL;
}
