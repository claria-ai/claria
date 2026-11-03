# Lucia showcase videos

Three videos of Lucia in distraction mode, embedded on
[claria-ai.github.io/fun.html](https://claria-ai.github.io/fun.html) ("Meet Lucia", linked from the
home page footer). Re-record them whenever her sprites or behavior change.

| Name | Length | Shows |
|---|---|---|
| `lucia-meet` | ~85 s | Real app: Preferences → General → Distraction mode, open a client, woof ×2 then a pet-twirl, the skateboard button, sock chase/pickup/carry, the Companion / Watch / Feed buttons, × hides her, Preferences brings her back |
| `lucia-watch-and-feed` | ~57 s | Watch: gaze follows the pointer around her head. Feed: dekopon 0→5 by clicking her and the button, "Full! 5/5", Start over |
| `lucia-a-day` | ~83 s | Real app on a fake clock that skips only idle waits: stretch, lie down, twirl, typing perks her up, newspaper while chat thinks, skateboard; then every "Preview Lucia action" on the demo page |

## Recording

Unlike the scenarios in `../scenarios/`, this is a plain Node script, not a Playwright test. It
reuses the e2e suite's Playwright and its Tauri mock (`e2e/tauri-mock.ts`), which knows
distraction mode. So from the repo root:

```bash
(cd e2e && npm install)                     # once; Chrome must be installed
(cd claria-desktop-frontend && npm run dev -- --host 127.0.0.1 --port 1437 --strictPort) &

node demos/lucia/record.mjs meet
node demos/lucia/record.mjs watch-and-feed
node demos/lucia/record.mjs a-day
demos/lucia/encode.sh ../claria-ai.github.io  # MP4s + posters, copied into the site checkout
```

Then in the site checkout run `uv run build.py`, check `fun.html` locally and commit.

- Everything lands in `demos/output/lucia/` (git-ignored): `raw/*.webm`, the MP4s, a
  `-poster.jpg` per video (its opening title card), and `rec-<name>.log`. The log has a
  timestamp for every caption, click, checkpoint and behavior change, so you can pull frames
  to check: `ffmpeg -ss <t> -i demos/output/lucia/lucia-meet.mp4 -frames:v 1 frame.png`.
- Use a different dev server with `LUCIA_BASE_URL=http://127.0.0.1:<port>`.
- `--dry` runs headless without video and saves a screenshot at each checkpoint. It's quick
  for checking a script change before a full take.
- It records in **headed** Chrome on purpose. Headless Chrome records at 1x and cuts the frame
  off at 489 px high. Three Chrome windows pop up during the run; leave them alone.
- Viewport is 768×576 at 2x, so Lucia is about 225 px tall in the 1280 px video with no crop.
  Captions, the cursor dot with click rings, and chapter cards are DOM overlays
  (`pointer-events: none`) injected by the script.
- A run fails fast if an expected behavior never shows up (`waitBehavior`): the log ends with
  `FAILED: …` and `fail-<name>.png` shows the screen at that moment.

## When Lucia changes

- **New or regenerated sprites:** just re-record. The script waits on `data-behavior` and
  `data-fed-count`, not on frame counts or timings, so new strips need no script change.
- **A new behavior or preview action:** add a beat to the matching video function in
  `record.mjs`, with a `caption(…)`, the action, and `waitBehavior(…)`. Preview actions are
  listed in `video3`.
- **Changed controls or labels:** the script finds them by accessible name (`Lucia controls`,
  `Companion` / `Watch` / `Feed`, `Drop a sock for Lucia`, `Give Lucia her skateboard`, `Feed a dekopon`, `Start over`,
  `Hide Lucia`, `Enable distraction mode`), the same as the e2e specs. Update both together.
