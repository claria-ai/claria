// Record the Lucia showcase videos for claria-ai.github.io/fun.html. See README.md here.
// usage: node demos/lucia/record.mjs <meet|watch-and-feed|a-day> [--dry]
//   --dry: headless, no video, a screenshot at every checkpoint into output/lucia/dry-<name>-*.png
// Uses the e2e suite's Playwright and Tauri mock (the mock there knows distraction mode),
// so `npm install` in e2e/ first. Needs a Vite dev server at LUCIA_BASE_URL.
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("../..", import.meta.url)).replace(/\/$/, "");
const outDir = `${repo}/demos/output/lucia`;
const [which, flag] = process.argv.slice(2);
const DRY = flag === "--dry";
if (!["meet", "watch-and-feed", "a-day"].includes(which)) {
  console.error("usage: node demos/lucia/record.mjs <meet|watch-and-feed|a-day> [--dry]");
  process.exit(2);
}
mkdirSync(`${outDir}/raw`, { recursive: true });
const { chromium } = await import(`${repo}/e2e/node_modules/playwright/index.mjs`);
const { buildInitScript } = await import(`${repo}/e2e/tauri-mock.ts`);

const BASE = process.env.LUCIA_BASE_URL ?? "http://127.0.0.1:1437";
const VIEWPORT = { width: 768, height: 576 };
const RAIL = 142;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const browser = await chromium.launch({ channel: "chrome", headless: DRY });
const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2 });
const page = await context.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
page.on("response", (r) => { if (r.status() >= 400) errors.push(`http ${r.status()}: ${r.url()}`); });
page.on("console", (m) => { if (m.type() === "error") errors.push(`console: ${m.text()}`); });

const t0 = { at: 0 };
const log = [];
const mark = (what) => { const t = ((Date.now() - t0.at) / 1000).toFixed(1); log.push(`${t}s ${what}`); console.log(`${t}s ${what}`); };
let shot = 0;
const checkpoint = async (name) => {
  mark(`checkpoint ${name}`);
  if (DRY) await page.screenshot({ path: `${outDir}/dry-${which}-${String(++shot).padStart(2, "0")}-${name}.png` });
};

// ── Overlays (plain DOM, pointer-events:none, re-created after navigation) ──
const OVERLAY_CSS = `
#__cap{position:fixed;left:0;right:0;display:flex;justify-content:center;pointer-events:none;z-index:2147483640;transition:opacity .25s}
#__cap span{background:rgba(17,24,39,.86);color:#fff;font:600 17px/1.3 system-ui,-apple-system,sans-serif;padding:8px 16px;border-radius:10px;max-width:640px;text-align:center;box-shadow:0 4px 14px rgba(0,0,0,.18)}
#__chap{position:fixed;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;background:rgba(255,251,235,.94);pointer-events:none;z-index:2147483645;font-family:system-ui,-apple-system,sans-serif}
#__chap b{font-size:38px;color:#1f2937;letter-spacing:-.5px}
#__chap i{font-style:normal;font-size:18px;color:#6b7280}
#__dot{position:fixed;left:0;top:0;width:16px;height:16px;margin:-8px 0 0 -8px;border-radius:50%;background:#111827;box-shadow:0 0 0 2.5px rgba(255,255,255,.95),0 2px 6px rgba(0,0,0,.35);pointer-events:none;z-index:2147483647;transition-property:transform;transition-timing-function:cubic-bezier(.4,0,.2,1)}
.__ring{position:fixed;width:16px;height:16px;margin:-8px 0 0 -8px;border-radius:50%;border:3px solid #2563eb;box-sizing:border-box;pointer-events:none;z-index:2147483646;animation:__ring 650ms ease-out forwards}
@keyframes __ring{from{transform:scale(1);opacity:.95}to{transform:scale(4.2);opacity:0}}`;
let cursor = { x: VIEWPORT.width / 2, y: VIEWPORT.height / 2 };
let currentCaption = null;
const ensureOverlays = () => page.evaluate(({ css, x, y }) => {
  if (!document.getElementById("__ovcss")) {
    const s = document.createElement("style"); s.id = "__ovcss"; s.textContent = css; document.documentElement.appendChild(s);
  }
  if (!document.getElementById("__dot")) {
    const d = document.createElement("div"); d.id = "__dot"; d.style.transform = `translate(${x}px,${y}px)`;
    document.documentElement.appendChild(d);
  }
}, { css: OVERLAY_CSS, x: cursor.x, y: cursor.y });

async function caption(text, pos = "strip") {
  currentCaption = text;
  await ensureOverlays();
  await page.evaluate(({ text, pos, rail }) => {
    let c = document.getElementById("__cap");
    if (!text) { c?.remove(); return; }
    if (!c) { c = document.createElement("div"); c.id = "__cap"; c.appendChild(document.createElement("span")); document.documentElement.appendChild(c); }
    c.style.top = pos === "top" ? "76px" : "auto";
    c.style.bottom = pos === "top" ? "auto" : pos === "low" ? "16px" : `${rail + 14}px`;
    c.firstChild.textContent = text;
  }, { text, pos, rail: RAIL });
  mark(`caption: ${text}`);
}
async function chapter(title, sub, ms = 1400) {
  await ensureOverlays();
  await page.evaluate(({ title, sub }) => {
    const c = document.createElement("div"); c.id = "__chap";
    c.innerHTML = `<b></b><i></i>`; c.firstChild.textContent = title; c.lastChild.textContent = sub || "";
    document.documentElement.appendChild(c);
    // The dot would sit on the title; hide it while the card is up.
    document.getElementById("__dot").style.visibility = "hidden";
  }, { title, sub });
  mark(`chapter: ${title}`);
  await sleep(ms);
  await page.evaluate(() => {
    document.getElementById("__chap")?.remove();
    document.getElementById("__dot").style.visibility = "";
  });
}
async function moveTo(x, y, dur) {
  await ensureOverlays();
  const dist = Math.hypot(x - cursor.x, y - cursor.y);
  dur ??= Math.round(Math.min(900, Math.max(350, 280 + dist * 0.9)));
  await page.evaluate(({ x, y, dur }) => {
    const d = document.getElementById("__dot"); d.style.transitionDuration = `${dur}ms`; d.style.transform = `translate(${x}px,${y}px)`;
  }, { x, y, dur });
  await page.mouse.move(x, y, { steps: Math.max(1, Math.round(dur / 40)) });
  cursor = { x, y };
  await sleep(Math.max(0, dur - Math.round(dur / 40) * 5) + 60);
}
/** Pointer that follows a path in lock-step (no CSS easing) — used for gaze sweeps. */
async function trace(points, stepMs = 35) {
  await ensureOverlays();
  for (const [x, y] of points) {
    await page.evaluate(({ x, y }) => {
      const d = document.getElementById("__dot"); d.style.transitionDuration = "0ms"; d.style.transform = `translate(${x}px,${y}px)`;
    }, { x, y });
    await page.mouse.move(x, y);
    cursor = { x, y };
    await sleep(stepMs);
  }
}
const ring = (x, y) => page.evaluate(({ x, y }) => {
  const r = document.createElement("div"); r.className = "__ring"; r.style.left = `${x}px`; r.style.top = `${y}px`;
  document.documentElement.appendChild(r); window.setTimeout(() => r.remove(), 800);
}, { x, y });
async function click(locator, { dx = 0, dy = 0, label } = {}) {
  await locator.waitFor({ state: "visible" });
  const box = await locator.boundingBox();
  const x = Math.round(box.x + box.width / 2 + dx), y = Math.round(box.y + box.height / 2 + dy);
  await moveTo(x, y);
  await ring(x, y);
  await sleep(120);
  await page.mouse.click(x, y);
  mark(`click ${label ?? ""}`);
  await sleep(250);
  await ensureOverlays().catch(() => {});
}

// ── Lucia handles ──
const scene = () => page.getByTestId("lucia-scene");
const sprite = () => page.getByTestId("lucia-sprite");
const controls = () => page.getByRole("group", { name: "Lucia controls" });
const modeButton = (name) => controls().getByRole("button", { name, exact: true });
const behavior = () => scene().getAttribute("data-behavior").catch(() => null);
async function waitBehavior(b, timeout = 15_000) {
  await page.locator(`[data-testid="lucia-scene"][data-behavior="${b}"]`).waitFor({ timeout });
  mark(`behavior=${b}`);
}
async function waitFed(n) { await page.locator(`[data-testid="lucia-scene"][data-fed-count="${n}"]`).waitFor({ timeout: 10_000 }); }
/** Lucia's head centre in viewport coordinates (head anchor 80,66 at 0.65 scale, as the e2e gaze test uses). */
async function head() {
  const b = await sprite().boundingBox();
  return { x: b.x + 80 * 0.65, y: b.y + 66 * 0.65 };
}
async function startRecording(name) {
  if (!DRY) await page.screencast.start({ path: `${outDir}/raw/${name}.webm`, size: { width: VIEWPORT.width * 2, height: VIEWPORT.height * 2 } });
  t0.at = Date.now();
  mark(`recording ${name}`);
}
async function stopRecording() {
  await caption(null);
  if (!DRY) await page.screencast.stop();
  mark("stopped");
}

async function openPreferencesGeneral() {
  await click(page.getByTitle("Preferences", { exact: true }), { label: "Preferences" });
  await sleep(900);
  await click(page.getByRole("button", { name: "General", exact: true }), { label: "General" });
  await sleep(900);
  return page.getByLabel("Enable distraction mode");
}
async function openJaneDoe() {
  await click(page.getByRole("button", { name: "Client Files" }), { label: "Client Files" });
  await sleep(900);
  await click(page.getByText("Jane Doe", { exact: true }), { label: "Jane Doe" });
}

// ───────────────────────── Video 1: Meet Lucia (real app) ─────────────────────────
async function video1() {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
  await page.goto(BASE);
  await page.getByRole("button", { name: "Client Files" }).waitFor();
  await ensureOverlays();
  await startRecording("lucia-meet");
  await chapter("Meet Lucia", "Claria's distraction-mode dog");
  await caption("Lucia is off by default. She lives in Preferences.", "low");
  await sleep(1800);
  const pref = await openPreferencesGeneral();
  await caption("Preferences → General → Enable distraction mode", "low");
  await sleep(1600);
  await click(pref, { label: "enable checkbox" });
  await checkpoint("pref-enabled");
  await sleep(1800);
  await caption("Now open a client…", "low");
  await click(page.getByRole("button", { name: "Back", exact: true }), { label: "Back" });
  await sleep(700);
  await openJaneDoe();
  await scene().waitFor();
  await caption("…and there she is in her strip, wagging hello");
  await sleep(600);
  await checkpoint("arrived");
  await sleep(3000);

  await caption("Click her → a (silent) woof");
  await sleep(1200);
  const h0 = await head();
  for (let i = 0; i < 2; i++) {
    await click(sprite(), { label: `bark ${i + 1}` });
    await waitBehavior("bark", 3000);
    await moveTo(h0.x + 150, h0.y - 45, 400); // step aside so the woof is unobstructed
    if (i === 0) { await sleep(250); await checkpoint("bark"); }
    await waitBehavior("idle", 5000);
    await sleep(1500);
  }
  await caption("Keep clicking → she plays something at random");
  await sleep(1200);
  // Pin the roll for a repeatable take: 0.2 lands on the skateboard in the fun pool.
  await page.evaluate(() => { window.__luciaRandom = Math.random; Math.random = () => 0.2; });
  await click(sprite(), { label: "pet 3" });
  await waitBehavior("board-chase", 3000);
  await sleep(600);
  await checkpoint("board-chase");
  await waitBehavior("skateboard", 10_000);
  await waitBehavior("idle", 15_000);
  await page.evaluate(() => { Math.random = window.__luciaRandom; });
  await sleep(1800);

  await caption("Drop a sock…");
  await sleep(900);
  await click(page.getByRole("button", { name: "Drop a sock for Lucia" }), { label: "sock" });
  await waitBehavior("chase", 3000);
  await caption("…she chases it…");
  await sleep(700);
  await checkpoint("chase");
  await waitBehavior("pickup");
  await caption("…picks it up…");
  await sleep(400);
  await checkpoint("pickup");
  await waitBehavior("carry-away");
  await caption("…and trots off with her prize");
  await sleep(900);
  await checkpoint("carry");
  await waitBehavior("idle");
  await caption("Three modes: Companion, Watch, Feed");
  await sleep(2200);

  await click(modeButton("Watch"), { label: "Watch" });
  await caption("Watch: she sits and follows your pointer");
  const h = await head();
  await moveTo(h.x + 120, h.y - 60, 700);
  await moveTo(h.x - 60, h.y - 70, 900);
  await moveTo(h.x - 70, h.y + 60, 900);
  await checkpoint("watch");
  await moveTo(h.x + 150, h.y + 40, 900);
  await sleep(900);
  await click(modeButton("Feed"), { label: "Feed" });
  await caption("Feed: dekopon time (more in part 2)");
  await sleep(1200);
  await click(page.getByRole("button", { name: "Feed a dekopon", exact: true }), { label: "feed once" });
  await waitFed(1);
  await checkpoint("feed-one");
  await sleep(1800);
  await click(modeButton("Companion"), { label: "Companion" });
  await caption("Companion: back to doing her own thing");
  await sleep(2800);

  await caption("The × hides her…");
  await sleep(1000);
  await click(page.getByRole("button", { name: "Hide Lucia" }), { label: "hide" });
  await page.getByTestId("lucia-rail").waitFor({ state: "detached" });
  await caption("…strip gone, workspace back to normal", "low");
  await checkpoint("hidden");
  await sleep(2600);
  await caption("Want her back? Preferences → Distraction mode", "low");
  await click(page.getByRole("button", { name: "Back", exact: true }), { label: "Back" });
  await sleep(600);
  await click(page.getByRole("button", { name: "Back", exact: true }), { label: "Back" });
  await sleep(700);
  const pref2 = await openPreferencesGeneral();
  await checkpoint("pref-off-again");
  await sleep(1400);
  await click(pref2, { label: "re-enable" });
  await sleep(1300);
  await click(page.getByRole("button", { name: "Back", exact: true }), { label: "Back" });
  await sleep(600);
  await openJaneDoe();
  await scene().waitFor();
  await caption("…and she's back!");
  await sleep(500);
  await checkpoint("back");
  await sleep(3200);
  await stopRecording();
}

// ───────────────────────── Video 2: Watch & Feed (real app) ─────────────────────────
async function video2() {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
  await page.addInitScript(() => localStorage.setItem("claria.distraction_mode", "true"));
  await page.goto(BASE);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
  await waitBehavior("idle");
  await ensureOverlays();
  await startRecording("lucia-watch-and-feed");
  await chapter("Watch & Feed", "two of Lucia's three modes");
  await caption("Watch mode: she sits up and watches your pointer");
  await sleep(800);
  await click(modeButton("Watch"), { label: "Watch" });
  await waitBehavior("gaze");
  const h = await head();
  const circle = (r, turns, steps, start = -Math.PI / 2) => Array.from({ length: steps * turns + 1 }, (_, i) => {
    const a = start + (i / steps) * 2 * Math.PI;
    return [Math.round(h.x + Math.cos(a) * r), Math.round(h.y + Math.sin(a) * r)];
  });
  await moveTo(h.x, h.y - 75, 800);
  await caption("Circle around her: her eyes follow all the way round");
  await trace(circle(75, 2, 90), 40);
  await checkpoint("gaze-circle");
  await caption("…and anywhere else on screen");
  await moveTo(700, 60, 1000); await sleep(900);
  await checkpoint("gaze-far-right");
  await moveTo(40, 40, 1000); await sleep(900);
  await moveTo(560, 530, 1000); await sleep(900);
  await moveTo(h.x + 40, h.y - 110, 700); await sleep(900);
  await caption("Click her while she watches → woof");
  await click(sprite(), { label: "gaze bark" });
  await waitBehavior("bark", 3000);
  await moveTo(h.x + 160, h.y - 60, 400);
  await waitBehavior("gaze", 5000);
  await sleep(1400);

  await chapter("Feeding", "up to five dekopon", 1300);
  await click(modeButton("Feed"), { label: "Feed" });
  await waitBehavior("feeding-idle");
  await caption("Feed mode: 0/5 dekopon");
  await sleep(1600);
  for (let i = 0; i < 5; i++) {
    const viaLucia = i % 2 === 0;
    await caption(viaLucia ? `Click Lucia → dekopon #${i + 1}` : `Or press "Feed a dekopon" → #${i + 1}`);
    await sleep(500);
    await click(viaLucia ? sprite() : page.getByRole("button", { name: "Feed a dekopon", exact: true }), { label: `feed ${i + 1}` });
    await waitBehavior("feeding", 3000);
    if (viaLucia) await moveTo(h.x + 160, h.y - 50, 400);
    if (i === 2) { await sleep(450); await checkpoint("feeding-mid"); }
    await waitFed(i + 1);
    if (i < 4) {
      await caption(`${i + 1}/5: a little rounder each time`);
      await sleep(2000);
    }
  }
  await caption("Full! 5/5 dekopon");
  await sleep(300);
  await checkpoint("full");
  await waitBehavior("feeding-idle");
  await sleep(3000);
  await checkpoint("full-idle");
  await caption("Start over → back to 0/5");
  await sleep(700);
  await click(page.getByRole("button", { name: "Start over" }), { label: "start over" });
  await waitFed(0);
  await sleep(2600);
  await checkpoint("reset");
  await stopRecording();
}

// ───────────────────── Video 3: A day with Lucia (real app + demo menu) ─────────────────────
async function video3() {
  // Tauri mock everywhere except the standalone demo page (which installs its own and refuses a pre-existing bridge).
  await page.addInitScript({ content: `if (location.pathname !== "/lucia-demo.html") {${buildInitScript({ configured: true })}}` });
  await page.addInitScript(() => localStorage.setItem("claria.distraction_mode", "true"));
  // Deterministic "lucky" roll so due twirl / skateboard deadlines fire (as the e2e cadence test does).
  await page.addInitScript(() => { Math.random = () => 0; });
  // Hold each chat reply ~6s so "Claria is thinking" is visible, and give it a friendlier answer.
  await page.addInitScript(() => {
    if (location.pathname === "/lucia-demo.html") return;
    const wrap = () => {
      const t = window.__TAURI_INTERNALS__;
      if (!t || t.__luciaWrapped) return;
      const invoke = t.invoke;
      t.invoke = async (cmd, args, ...rest) => {
        if (cmd !== "chat_message") return invoke(cmd, args, ...rest);
        await new Promise((r) => window.setTimeout(r, 6000));
        const res = await invoke(cmd, args, ...rest);
        return { ...res, content: "The teacher noticed attention drifting after about 10 minutes of seat work, and big improvements with movement breaks." };
      };
      t.__luciaWrapped = true;
    };
    wrap();
    document.addEventListener("DOMContentLoaded", wrap);
  });
  const now = new Date("2026-11-02T09:00:00Z");
  await page.clock.install({ time: now }); // flowing clock; we only jump over idle waits
  await page.goto(BASE);
  await page.getByRole("button", { name: "Client Files" }).waitFor();
  await ensureOverlays();
  await startRecording("lucia-a-day");
  await chapter("A day with Lucia", "what she gets up to while you work");
  await openJaneDoe();
  await scene().waitFor();
  await caption("Open a client: she arrives with a tail wag");
  await sleep(250);
  await checkpoint("tailwag");
  await waitBehavior("idle");
  const mountAt = await page.evaluate(() => performance.now());
  await caption("Idle: breathing, blinking, hanging out");
  await sleep(4200);

  await caption("Skip ahead: 30 quiet seconds…");
  await sleep(1400);
  await page.clock.fastForward(30_000);
  await waitBehavior("stretch", 3000);
  await caption("…she stretches…");
  await sleep(500);
  await checkpoint("stretch");
  await waitBehavior("bored");
  await caption("…and flops down. Bored now.");
  await sleep(3200);
  await checkpoint("bored");

  await caption("Skip ahead: a couple of quiet minutes…");
  await sleep(1400);
  const sinceMount = (await page.evaluate(() => performance.now())) - mountAt;
  await page.clock.fastForward(Math.max(1000, 120_000 - sinceMount + 500));
  await waitBehavior("twirl", 4000);
  await caption("…a twirl!");
  await sleep(450);
  await checkpoint("twirl");
  await waitBehavior("bored", 8000); // twirl → stretch → lie down again
  await sleep(2200);

  await caption("Start typing → her ears perk up", "top");
  await click(page.getByRole("tab", { name: "Chat", exact: true }), { label: "Chat tab" });
  await sleep(600);
  const box = page.getByRole("textbox");
  await click(box, { label: "textbox" });
  await box.pressSequentially("What did the teacher notice", { delay: 75 });
  await checkpoint("typing");
  await box.pressSequentially(" about attention?", { delay: 75 });
  await sleep(1400);
  await checkpoint("typed-idle");
  await caption("Send it: Claria thinks → Lucia reads the paper", "top");
  await sleep(600);
  await click(page.getByRole("button", { name: "Send", exact: true }), { label: "Send" });
  await waitBehavior("reading-enter", 3000);
  await waitBehavior("reading", 3000);
  await sleep(400);
  await checkpoint("reading");
  await waitBehavior("reading-exit", 10_000);
  await caption("Answer's in → paper folded away", "top");
  await sleep(500);
  await checkpoint("reading-exit");
  await waitBehavior("idle");
  await sleep(2000);
  await click(page.getByRole("tab", { name: "Record", exact: true }), { label: "Record tab" });
  await sleep(600);

  await caption("Skip ahead: quiet again…");
  await sleep(1400);
  await page.clock.fastForward(30_000);
  await waitBehavior("stretch", 3000);
  await caption("…stretch, flop…");
  await waitBehavior("bored");
  await sleep(1800);
  // A play deadline only fires if she is lying down when it comes due (missed ones never queue).
  await caption("Skip ahead: three minutes into her nap…");
  await sleep(1400);
  const sinceMount2 = (await page.evaluate(() => performance.now())) - mountAt;
  await page.clock.fastForward(Math.max(1000, 180_000 - sinceMount2 + 500));
  await waitBehavior("board-chase", 4000);
  await caption("…there goes her board…");
  await waitBehavior("skateboard", 8000);
  await caption("…skateboard!");
  await sleep(900);
  await checkpoint("skateboard");
  await waitBehavior("stretch", 8000).catch(() => {});
  await sleep(400);

  await chapter("On demand", "the demo page's “Preview Lucia action” menu", 1400);
  await page.goto(`${BASE}/lucia-demo.html`);
  await scene().waitFor();
  await ensureOverlays();
  await waitBehavior("idle");
  const select = page.getByLabel("Preview Lucia action", { exact: true });
  const options = [
    ["tailwag", "Entrance wag", "idle"],
    ["ear-perk", "Ears perk", "idle"],
    ["twirl", "Twirl in place", "idle"],
    ["stretch", "Stretch then lie down", "bored"],
    ["skateboard", "Skateboard crossing", null],
  ];
  for (const [value, label, after] of options) {
    await caption(`Preview: ${label}`);
    await sleep(500);
    const b = await select.boundingBox();
    const x = Math.round(b.x + b.width / 2), y = Math.round(b.y + b.height / 2);
    await moveTo(x, y);
    await ring(x, y);
    await select.selectOption(value);
    // A skate starts with the board rolling in; she rides back after it.
    if (value === "skateboard") await waitBehavior("board-chase", 3000);
    await waitBehavior(value, value === "skateboard" ? 8000 : 3000);
    await sleep(value === "skateboard" ? 1400 : 350);
    await checkpoint(`preview-${value}`);
    if (after) await waitBehavior(after, 8000);
    else await page.locator('[data-testid="lucia-scene"]:not([data-behavior="skateboard"])').waitFor({ timeout: 8000 });
    await sleep(after ? 1800 : 300);
  }
  await caption("That's Lucia. Turn her on in Preferences → General.");
  await sleep(2600);
  await stopRecording();
}

try {
  if (which === "meet") await video1();
  else if (which === "watch-and-feed") await video2();
  else await video3();
} catch (e) {
  mark(`FAILED: ${e.message.split("\n")[0]}`);
  await page.screenshot({ path: `${outDir}/fail-${which}.png` }).catch(() => {});
  if (!DRY) await page.screencast.stop().catch(() => {});
  process.exitCode = 1;
} finally {
  if (errors.length) console.log("PAGE ERRORS:\n" + errors.join("\n"));
  writeFileSync(`${outDir}/${DRY ? "dry" : "rec"}-${which}.log`, log.join("\n") + (errors.length ? "\nERRORS:\n" + errors.join("\n") : "") + "\n");
  await browser.close();
}
