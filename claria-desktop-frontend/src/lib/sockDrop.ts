// Data for the distraction-mode sock drop: phase timings, the shake-count
// roll, and the pixel-art sprites. The component in components/SockDrop.tsx
// renders these; keeping them here leaves that file exporting only components.

export const SOCK_DROP_TIMINGS = {
  /** Sock falls from the top of the screen and bounces to rest. */
  dropMs: 1000,
  /** Lucia walks in from the left edge to the sock. */
  walkInMs: 2200,
  /** Lucia lowers into a play bow and picks the sock up. */
  grabMs: 500,
  /** One full side-to-side neck shake. */
  shakeMsPerShake: 340,
  /** Lucia stands and walks off with the sock. */
  walkOffMs: 2200,
  /** One walk cycle: leg-frame swap and gentle trot. */
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

// Lucia follows the greying black schnauzer from the Claria landing page:
// a compact dark silhouette, restrained charcoal-to-silver highlights, one
// calm dark eye, and the familiar red collar and brass tag.
export const DOG_PALETTE: PixelPalette = {
  O: "#0b0f17", // near-black outline
  K: "#151a23", // deepest fur
  D: "#202834", // dark charcoal
  C: "#2c3745", // charcoal
  G: "#465260", // muted grey fur
  H: "#667382", // cool highlight
  S: "#8b96a3", // mild silvering
  E: "#111820", // dark cool-grey eye
  N: "#030508", // nose
  R: "#a9343a", // collar
  A: "#e06a67", // collar highlight
  T: "#d5a13e", // brass tag
};

// Side-on standing body. The second frame changes the legs for the trot;
// the torso and raised tail remain stable between frames.
export const DOG_FRAME_A: PixelMap = [
  ".......OO.......................................",
  ".....OOHHO......................................",
  "...OOHHHHO......................................",
  "..OHHHHOO.......................................",
  "..OHHOO.........................................",
  "..OGGO..........................................",
  "..OGGGO..........OOOOOOOOOOOO...................",
  "...OGGO.......OOOCCCCCCCOOOOOOOO................",
  "...OGGOO...OOOCCCCCCGGGGGGGCCCCCOOO.............",
  "...OGGGOO.OOCCCGGGGGGGGHGGGGGGGGCDORROO.........",
  "...OOGGGGOODDDGGGSGGGGCCCCCCCGHGGCCRAAARO.......",
  "....OOGGOODDDDDDGCCCCCCCCCCGCCCCCDDRRRRRO.......",
  "......OOODDDDDHCCCCCGCCCCCCCCCCCCCGGDRRROO......",
  ".......OODDDDCCCCCCCCCCCCCCCCCCCCCGGGGGTOO......",
  ".......OODDDDCCCCCCCCHCCCCCCCHCCCCGGHHHDOO......",
  ".......OODDDGDCCCCCCCCCCCCCCCCCCCCKGHHHDOO......",
  "........OODDDDDDHCCCCCCCCGCCCCCKKKKGGHHGOO......",
  ".........OODDDDKKKKKKKKKKKKKKKKKCKKGGGGGDOO.....",
  "..........OOOOOOOOKKKOOOOOKKKKOOOOODOOGDDOO.....",
  "...........OODDDOOCKKODDOODDDDODDOODOOOOOOO.....",
  "............ODDDOOOOOODDOOOOOOODDOODODDOOO......",
  "............ODDDOOOOOODDOOOOO.ODDOOOODDOO.......",
  "............ODDDOO...ODDOO....ODDOO.ODDOO.......",
  "............ODDOO....DDOO....ODDOO...DDDO.......",
  "............ODDOO....DDOO....ODDOO...DDDOO......",
  "............ODDOO....DDOO....ODDOO...DDDOO......",
  "............ODDOO....DDOO....ODDOO...OOOOO......",
  "............OOOOO....GOOO....OOOOO...OGOOO......",
  "............OGOOO............OGOOO..............",
  "................................................",
  "................................................",
  "................................................",
];

export const DOG_FRAME_B: PixelMap = [
  ".......OO.......................................",
  ".....OOHHO......................................",
  "...OOHHHHO......................................",
  "..OHHHHOO.......................................",
  "..OHHOO.........................................",
  "..OGGO..........................................",
  "..OGGGO..........OOOOOOOOOOOO...................",
  "...OGGO.......OOOCCCCCCCOOOOOOOO................",
  "...OGGOO...OOOCCCCCCGGGGGGGCCCCCOOO.............",
  "...OGGGOO.OOCCCGGGGGGGGHGGGGGGGGCDORROO.........",
  "...OOGGGGOODDDGGGSGGGGCCCCCCCGHGGCCRAAARO.......",
  "....OOGGOODDDDDDGCCCCCCCCCCGCCCCCDDRRRRRO.......",
  "......OOODDDDDHCCCCCGCCCCCCCCCCCCCGGDRRROO......",
  ".......OODDDDCCCCCCCCCCCCCCCCCCCCCGGGGGTOO......",
  ".......OODDDDCCCCCCCCHCCCCCCCHCCCCGGHHHDOO......",
  ".......OODDDGDCCCCCCCCCCCCCCCCCCCCKGHHHDOO......",
  "........OODDDDDDHCCCCCCCCGCCCCCKKKKGGHHGOO......",
  ".........OODDDDKKKKKKKKKKKKKKKKKCKKGGGGGDOO.....",
  "..........OOOOOOOOKKKOOOOOKKKKOOOOODOOGDDOO.....",
  "...........OODDDOOCKKODDOODDDDODDOODOOOOOOO.....",
  "............ODDDOOOOOODDOOOOOOODDOOOODDOOO......",
  "............ODDDOOOOODDOOOOOO.ODDDOOODDOO.......",
  "............ODDDOO..ODDOO.....DDDDOOODDOO.......",
  "...........ODDDDDOO.ODDOO....ODDDDOODDOOO.......",
  "...........ODDDDDOO.ODDOO....ODDDDDODDOO........",
  "...........ODDDDDOO.DDOOO....ODDDDDODDOO........",
  "...........ODDDOOOOODDOO.....OODDDOODDOO........",
  "...........OOOOOOO.OOOOO.....OOOOOOOOOOO........",
  "...........OOOOO...OOOOO.......OOOOOOOOO........",
  "................................................",
  "................................................",
  "................................................",
];

// Play-bow/downward-dog pose: rump and tail stay high while the shoulders,
// neck and forelegs reach down toward the sock.
export const DOG_PLAY_BOW: PixelMap = [
  ".........OO.....................................",
  ".....OOOOSOO....................................",
  "....OHSSSSOO....................................",
  "....OHSOOOO.....................................",
  "....OHHO......OOOOOOOOOOOOOO....................",
  "....OHHO...OOOOODCCCCDDDDDOOOOO.................",
  "....OHHOO.OOODCCCCCGGGGCCCDDDOOO................",
  "....OHHGOOOCCCCGGGGGGGGGGGGCCCDOO...............",
  "....OOGGOODDDDGGGSGGGGGGHGGGGGCDOO..............",
  ".....OOOODDDDDDDDCCCCCCCCCCCCGGGCOO.............",
  ".......OODDDDHCCCCCCCCCCCCCCCCCHCCOO............",
  ".......OODDDCCCCCCCCGCCCCCCGCCCCDROOO...........",
  ".......OODDDCCCCCCCCCCCCCCCCCCCCDGAAOO..........",
  "........OODDDCCCCCCHCCCCCCCCCHCCCGRAAO..........",
  ".........OODDDGCCCCCCCCCCCCCCCCCCGGRAAO.........",
  "..........OOOOOOOCCCKKKKGKKKKCCCCCHGRROO........",
  "...........OODDDOKKKKOOOOOOKKKKKCDGGGGRO........",
  "...........OODDDOKKKKODDDOOKKKKKKDGGGGTOO.......",
  "...........OODDDOKKKKODDDOOKKKKKDDDGGSGDOO......",
  "...........OODDDO....ODDDOO.....ODDDGGGGOO......",
  "...........OODDDO....ODDDOO.....OOOOOOOOO.......",
  "...........OODDOO....ODDDOO.....ODDDDDDOO.......",
  "............ODDOO....ODDOO.......ODDODDOO.......",
  "............ODDOO....ODDOO.......ODDOODDO.......",
  "............ODDOO....ODDOO.......OODOODDOO......",
  "............ODDOO....ODDOO........ODDOODOO......",
  "............ODDOO....ODDOO........OODOOODOO.....",
  "............OOOOO....OOOOO.........OOOOODOOOO...",
  "............OGOOO....OGOOO..........OODOODDOOOO.",
  ".....................................OODDGDDHOO.",
  "......................................OOOOOOOOO.",
  "................................................",
];

// Compact side-profile head and neck. It overlaps the shoulder so the bowing
// movement reads as one connected dog rather than a head floating below her.
export const DOG_HEAD: PixelMap = [
  "......................",
  ".......OOOO...........",
  ".....OOOOOOOOK........",
  "...KKKKDDDDDDKKK......",
  "..KKKKDCCHSCDDKKK.....",
  ".KKKKDCCGCCGCDDCK.....",
  ".KKCKDDCCCCSEDDKKK....",
  ".KKKDDDDDCCCCGDKGKO...",
  ".OKDGDDCCCCCGGHGGGOO..",
  ".OKDDDCCCCCGGGGGHGGOO.",
  ".OKDDDDCCCCGGGGGGGGGNN",
  ".OODDDDDDOOOHGGGGOGOO.",
  ".OODCDDGDDDDDOGGHOOOO.",
  ".OODDDDDHDDDDOOOOOOO..",
  ".OOOOOGDDDGDDGOO......",
  "..OOOOOODDDHDOO.......",
  "....O...OODDOO........",
  "...........OO.........",
];

const DOG_SCALE = 4;
export const DOG_BODY_WIDTH = DOG_FRAME_A[0].length * DOG_SCALE;
export const DOG_BODY_HEIGHT = DOG_FRAME_A.length * DOG_SCALE;
export const DOG_HEAD_WIDTH = DOG_HEAD[0].length * DOG_SCALE;
export const DOG_HEAD_HEIGHT = DOG_HEAD.length * DOG_SCALE;
export const DOG_WIDTH = 64 * DOG_SCALE;
export const DOG_HEIGHT = 34 * DOG_SCALE;

export const DOG_LAYOUT = {
  bodyLeft: DOG_SCALE,
  bodyTop: DOG_SCALE,
  headLeft: 37 * DOG_SCALE,
  headTop: 3 * DOG_SCALE,
  headBowOffsetX: DOG_SCALE,
  headBowOffsetY: 5 * DOG_SCALE,
  mouthX: 59 * DOG_SCALE,
} as const;
