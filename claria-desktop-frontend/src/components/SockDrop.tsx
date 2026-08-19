import { useEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import {
  DOG_BODY_HEIGHT,
  DOG_BODY_WIDTH,
  DOG_FRAME_A,
  DOG_FRAME_B,
  DOG_HEAD,
  DOG_HEAD_HEIGHT,
  DOG_HEAD_WIDTH,
  DOG_HEIGHT,
  DOG_LAYOUT,
  DOG_PALETTE,
  DOG_PLAY_BOW,
  DOG_WIDTH,
  SOCK_DROP_TIMINGS,
  SOCK_MAP,
  SOCK_PALETTE,
  randomShakeCount,
  type PixelMap,
  type PixelPalette,
} from "../lib/sockDrop";

// The distraction-mode sock drop: a pixelated sock falls to the bottom of the
// screen, Lucia trots in from the left, lowers into a play bow, grabs the sock,
// shakes it from her neck, and carries it away. When she leaves, the overlay
// unmounts and the distraction is cleared.
//
// Phases are driven by setTimeout rather than animationend so the sequence is
// deterministic under fake timers; CSS animations are purely visual and take
// their durations from the shared timing constants via inline styles.

type Phase = "drop" | "walk-in" | "grab" | "shake" | "walk-off";

export default function SockDrop({
  onDone,
  shakeCount,
}: {
  onDone: () => void;
  /** Overrides the random 3–8 shake count (tests only). */
  shakeCount?: number;
}) {
  const [phase, setPhase] = useState<Phase>("drop");
  const [shakes] = useState(() => shakeCount ?? randomShakeCount());

  // Latest onDone without restarting the phase timer when the parent
  // re-renders with a fresh closure.
  const onDoneRef = useRef(onDone);
  useEffect(() => {
    onDoneRef.current = onDone;
  }, [onDone]);

  useEffect(() => {
    const t = SOCK_DROP_TIMINGS;
    const next: Record<Phase, { phase: Phase | null; after: number }> = {
      drop: { phase: "walk-in", after: t.dropMs },
      "walk-in": { phase: "grab", after: t.walkInMs },
      grab: { phase: "shake", after: t.grabMs },
      shake: { phase: "walk-off", after: t.shakeMsPerShake * shakes },
      "walk-off": { phase: null, after: t.walkOffMs },
    };
    const step = next[phase];
    const id = setTimeout(() => {
      if (step.phase) {
        setPhase(step.phase);
      } else {
        onDoneRef.current();
      }
    }, step.after);
    return () => clearTimeout(id);
  }, [phase, shakes]);

  const t = SOCK_DROP_TIMINGS;
  const walking = phase === "walk-in" || phase === "walk-off";
  const bowing = phase === "grab" || phase === "shake";
  const carrying = phase === "grab" || phase === "shake" || phase === "walk-off";

  // Park Lucia's mouth on the sock at bottom-center. Her layered sprite can
  // change without moving where the interaction lands.
  const dogLeft =
    phase === "drop"
      ? `-${DOG_WIDTH + 24}px`
      : phase === "walk-off"
        ? "110%"
        : `calc(50% - ${DOG_LAYOUT.mouthX}px)`;
  const dogTransition =
    phase === "walk-in"
      ? `left ${t.walkInMs}ms linear`
      : phase === "walk-off"
        ? `left ${t.walkOffMs}ms linear`
        : "none";

  const motionStyle: CSSProperties = walking
    ? { animation: `sock-dog-trot ${t.strideMs}ms ease-in-out infinite` }
    : {};
  const frameAStyle: CSSProperties = bowing
    ? { opacity: 0 }
    : walking
      ? { animation: `sock-dog-frame-a ${t.strideMs}ms linear infinite` }
      : { opacity: 1 };
  const frameBStyle: CSSProperties = bowing
    ? { opacity: 0 }
    : walking
      ? { animation: `sock-dog-frame-b ${t.strideMs}ms linear infinite` }
      : { opacity: 0 };
  const bowStyle: CSSProperties = { opacity: bowing ? 1 : 0 };
  const headAnchorStyle: CSSProperties = {
    width: DOG_HEAD_WIDTH,
    height: DOG_HEAD_HEIGHT,
    left: DOG_LAYOUT.headLeft,
    top: DOG_LAYOUT.headTop,
    transformOrigin: "10% 50%",
    transform: bowing
      ? `translate(${DOG_LAYOUT.headBowOffsetX}px, ${DOG_LAYOUT.headBowOffsetY}px) rotate(4deg)`
      : "translate(0, 0) rotate(0deg)",
    transition: `transform ${t.grabMs}ms cubic-bezier(0.22, 1, 0.36, 1)`,
  };
  const neckStyle: CSSProperties = {
    width: DOG_HEAD_WIDTH,
    height: DOG_HEAD_HEIGHT,
    transformOrigin: "12% 50%",
    ...(phase === "shake"
      ? {
          animation: `sock-dog-neck-shake ${t.shakeMsPerShake}ms ease-in-out ${shakes}`,
        }
      : {}),
  };

  return createPortal(
    <div
      className="fixed inset-0 overflow-hidden pointer-events-none z-50"
      aria-hidden="true"
      data-testid="sock-drop"
      data-phase={phase}
      data-shakes={shakes}
    >
      {/* The dropped sock, resting at bottom-center until Lucia takes it. */}
      {!carrying && (
        <div
          className="absolute left-1/2 -translate-x-1/2 bottom-2"
          data-testid="sock-standing"
        >
          <div
            style={{
              animation: `sock-drop-fall ${t.dropMs}ms cubic-bezier(0.34, 1.1, 0.64, 1) both`,
            }}
          >
            <PixelSprite
              map={SOCK_MAP}
              palette={SOCK_PALETTE}
              style={{ width: 48, height: 44 }}
            />
          </div>
        </div>
      )}

      {/* Lucia: body pose and neck are separate so only her neck shakes. */}
      <div
        className="absolute bottom-1"
        style={{
          width: DOG_WIDTH,
          height: DOG_HEIGHT,
          left: dogLeft,
          transition: dogTransition,
        }}
        data-testid="sock-dog"
      >
        <div
          className="relative w-full h-full"
          style={motionStyle}
          data-testid="sock-dog-motion"
        >
          <div
            className="absolute inset-0"
            data-testid="sock-dog-body"
            data-pose={bowing ? "play-bow" : "standing"}
          >
            <PixelSprite
              map={DOG_FRAME_A}
              palette={DOG_PALETTE}
              className="absolute"
              style={{
                ...frameAStyle,
                width: DOG_BODY_WIDTH,
                height: DOG_BODY_HEIGHT,
                left: DOG_LAYOUT.bodyLeft,
                top: DOG_LAYOUT.bodyTop,
              }}
            />
            <PixelSprite
              map={DOG_FRAME_B}
              palette={DOG_PALETTE}
              className="absolute"
              style={{
                ...frameBStyle,
                width: DOG_BODY_WIDTH,
                height: DOG_BODY_HEIGHT,
                left: DOG_LAYOUT.bodyLeft,
                top: DOG_LAYOUT.bodyTop,
              }}
            />
            <PixelSprite
              map={DOG_PLAY_BOW}
              palette={DOG_PALETTE}
              className="absolute"
              style={{
                ...bowStyle,
                width: DOG_BODY_WIDTH,
                height: DOG_BODY_HEIGHT,
                left: DOG_LAYOUT.bodyLeft,
                top: DOG_LAYOUT.bodyTop,
              }}
            />
          </div>

          <div
            className="absolute"
            style={headAnchorStyle}
            data-testid="sock-dog-neck-anchor"
          >
            <div style={neckStyle} data-testid="sock-dog-neck">
              <PixelSprite
                map={DOG_HEAD}
                palette={DOG_PALETTE}
                className="absolute inset-0"
                style={{ width: DOG_HEAD_WIDTH, height: DOG_HEAD_HEIGHT }}
              />
              {carrying && (
                <PixelSprite
                  map={SOCK_MAP}
                  palette={SOCK_PALETTE}
                  className="absolute"
                  style={{
                    width: 32,
                    height: 29,
                    left: DOG_HEAD_WIDTH - 14,
                    top: DOG_HEAD_HEIGHT * 0.43,
                    transform: "rotate(105deg)",
                  }}
                  data-testid="sock-carried"
                />
              )}
            </div>
          </div>
        </div>
      </div>
    </div>,
    document.body
  );
}

/** The header button's pixel-sock glyph, drawn from the same sprite. */
export function SockIcon({ className }: { className?: string }) {
  return (
    <PixelSprite map={SOCK_MAP} palette={SOCK_PALETTE} className={className} />
  );
}

function PixelSprite({
  map,
  palette,
  className,
  style,
  "data-testid": testId,
}: {
  map: PixelMap;
  palette: PixelPalette;
  className?: string;
  style?: CSSProperties;
  "data-testid"?: string;
}) {
  const rects: React.ReactNode[] = [];
  map.forEach((row, y) => {
    [...row].forEach((ch, x) => {
      const fill = palette[ch];
      if (fill) {
        rects.push(
          <rect
            key={`${x},${y}`}
            x={x}
            y={y}
            width={1}
            height={1}
            fill={fill}
          />
        );
      }
    });
  });
  return (
    <svg
      viewBox={`0 0 ${map[0].length} ${map.length}`}
      shapeRendering="crispEdges"
      className={className}
      style={style}
      aria-hidden="true"
      data-testid={testId}
    >
      {rects}
    </svg>
  );
}
