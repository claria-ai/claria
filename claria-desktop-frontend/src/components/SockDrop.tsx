import { useEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import {
  DOG_FRAME_A,
  DOG_FRAME_B,
  DOG_HEIGHT,
  DOG_PALETTE,
  DOG_WIDTH,
  SOCK_DROP_TIMINGS,
  SOCK_MAP,
  SOCK_PALETTE,
  randomShakeCount,
  type PixelMap,
  type PixelPalette,
} from "../lib/sockDrop";

// The distraction-mode sock drop: a pixelated sock falls to the bottom of the
// screen, Lucia (the Claria mascot, a black dog) waggles in from the left,
// grabs the sock, shakes it violently 3–8 times, and walks off with it in her
// mouth. When she leaves, the overlay unmounts and the distraction is cleared.
//
// Phases are driven by setTimeout rather than animationend so the sequence is
// deterministic under fake timers; the CSS animations (keyframes in
// index.css) are purely visual and take their durations from the shared
// timing constants via inline styles.

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
  const carrying = phase === "grab" || phase === "shake" || phase === "walk-off";

  // Lucia's mouth is at her right edge; parking her just left of center puts
  // the mouth on the sock, which rests at the horizontal center.
  const dogLeft =
    phase === "drop"
      ? `-${DOG_WIDTH + 24}px`
      : phase === "walk-off"
        ? "110%"
        : `calc(50% - ${DOG_WIDTH - 16}px)`;
  const dogTransition =
    phase === "walk-in"
      ? `left ${t.walkInMs}ms linear`
      : phase === "walk-off"
        ? `left ${t.walkOffMs}ms linear`
        : "none";

  const waggleStyle: CSSProperties = walking
    ? { animation: `sock-dog-waggle ${t.strideMs}ms ease-in-out infinite` }
    : {};
  // The shake rotates around roughly where her mouth is. During the grab she
  // leans into the sock; the shake animation then overrides that transform.
  const shakeStyle: CSSProperties = {
    transformOrigin: "85% 45%",
    ...(phase === "grab"
      ? { transform: "rotate(7deg)", transition: `transform ${t.grabMs}ms ease-out` }
      : {}),
    ...(phase === "shake"
      ? {
          animation: `sock-dog-shake ${t.shakeMsPerShake}ms ease-in-out ${shakes}`,
        }
      : {}),
  };
  const frameAStyle: CSSProperties = walking
    ? { animation: `sock-dog-frame-a ${t.strideMs}ms linear infinite` }
    : { opacity: 1 };
  const frameBStyle: CSSProperties = walking
    ? { animation: `sock-dog-frame-b ${t.strideMs}ms linear infinite` }
    : { opacity: 0 };

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

      {/* Lucia */}
      <div
        className="absolute bottom-1"
        style={{ left: dogLeft, transition: dogTransition }}
        data-testid="sock-dog"
      >
        <div style={waggleStyle}>
          <div style={shakeStyle}>
            <div
              className="relative"
              style={{ width: DOG_WIDTH, height: DOG_HEIGHT }}
            >
              <PixelSprite
                map={DOG_FRAME_A}
                palette={DOG_PALETTE}
                className="absolute inset-0 w-full h-full"
                style={frameAStyle}
              />
              <PixelSprite
                map={DOG_FRAME_B}
                palette={DOG_PALETTE}
                className="absolute inset-0 w-full h-full"
                style={frameBStyle}
              />
              {carrying && (
                <PixelSprite
                  map={SOCK_MAP}
                  palette={SOCK_PALETTE}
                  className="absolute"
                  style={{
                    width: 34,
                    height: 31,
                    left: DOG_WIDTH - 26,
                    top: DOG_HEIGHT * 0.36,
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

/** The composer button's pixel-sock glyph, drawn from the same sprite. */
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
        rects.push(<rect key={`${x},${y}`} x={x} y={y} width={1} height={1} fill={fill} />);
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
