// Data for the distraction-mode sock drop: phase timings, the shake-count
// roll, and the pixel-art sprites. The component in components/SockDrop.tsx
// renders these; keeping them here leaves that file exporting only components.

export const SOCK_DROP_TIMINGS = {
  /** Sock falls from the top of the screen and bounces to rest. */
  dropMs: 1000,
  /** Lucia walks in from the left edge to the sock. */
  walkInMs: 2200,
  /** Head-dip while she picks the sock up. */
  grabMs: 450,
  /** One full side-to-side shake. */
  shakeMsPerShake: 340,
  /** Lucia walks off the right edge, sock in mouth. */
  walkOffMs: 2200,
  /** One walk cycle: leg-frame swap and body waggle. */
  strideMs: 360,
} as const;

/** How many times Lucia shakes the sock: 3 to 8, inclusive. */
export function randomShakeCount(): number {
  return 3 + Math.floor(Math.random() * 6);
}

// ---------------------------------------------------------------------------
// Pixel sprites: string maps, one character per pixel, "." transparent
// ---------------------------------------------------------------------------

export type PixelMap = readonly string[];
export type PixelPalette = Readonly<Record<string, string>>;

// A cozy hand-knit sock: red cuff and heel, cream body, slate outline so it
// reads against the app's white background.
export const SOCK_PALETTE: PixelPalette = {
  D: "#64748b",
  R: "#dc2626",
  W: "#f8fafc",
};

export const SOCK_MAP: PixelMap = [
  "..DDDDDD....",
  ".DRRRRRRD...",
  ".DRRRRRRD...",
  ".DWWWWWWD...",
  ".DWWWWWWD...",
  "..DWWWWWD...",
  "..DWWWWWDD..",
  "..DWWWWWWWD.",
  ".DRWWWWWWWD.",
  ".DRRWWWWRRD.",
  "..DDDDDDDD..",
];

// Lucia, facing right: black silhouette, red collar, white eye, gray nose,
// pink tongue. Rows 0–11 are shared; the two frames differ only in the legs.
export const DOG_PALETTE: PixelPalette = {
  K: "#18181b",
  R: "#dc2626",
  W: "#ffffff",
  N: "#71717a",
  P: "#f472b6",
};

const DOG_BODY: PixelMap = [
  ".................KKK....",
  "................KKKKK...",
  "..K.............KKKKKK..",
  "..KK...........KKKKKKKK.",
  "...KK..........KKKKWKKKK",
  "...KKK.........KKKKKKKKN",
  "....KKKKKKKKKKKKKKKKKK..",
  "....KKKKKKKKKKKKRKKKKP..",
  "...KKKKKKKKKKKKKRKKKK...",
  "...KKKKKKKKKKKKKRKKK....",
  "...KKKKKKKKKKKKKKKK.....",
  "....KKKKKKKKKKKKKK......",
];

export const DOG_FRAME_A: PixelMap = [
  ...DOG_BODY,
  "....KKK......KKK........",
  "....KK........KK........",
  "...KK..........KK.......",
  "...KK...........KK......",
];

export const DOG_FRAME_B: PixelMap = [
  ...DOG_BODY,
  "....KKK......KKK........",
  ".....KK.......KK........",
  ".....KK.......KK........",
  "....KKK.......KKK.......",
];

const DOG_SCALE = 6;
export const DOG_WIDTH = DOG_FRAME_A[0].length * DOG_SCALE;
export const DOG_HEIGHT = DOG_FRAME_A.length * DOG_SCALE;
