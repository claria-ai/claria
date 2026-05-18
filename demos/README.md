# Demo Video Recording

Automated end-to-end video recording for [claria-ai.github.io](https://claria-ai.github.io) using Playwright.

## How it works

Same pattern as the sibling [`screenshots/`](../screenshots/) directory: Playwright drives the Vite dev server with `window.__TAURI_INTERNALS__` mocked, so the React frontend renders with fixture data and no Rust/AWS dependencies. The difference is that these tests run with `video: { mode: "on" }` and pace their interactions (mouse moves, typing, deliberate pauses) so the resulting WebM file is watchable as a product demo, not just functional verification.

Key files:

| File | Purpose |
|---|---|
| `fixtures.ts` | Scenario-specific mock data (clients, plan entries, chat responses, etc.) — typed exports consumed by scenarios |
| `tauri-mock.ts` | Builds the `addInitScript` payload that stubs `window.__TAURI_INTERNALS__.invoke` |
| `scenarios/01-bootstrap.spec.ts` | Fresh AWS account → provision all resources |
| `scenarios/02-sync.spec.ts` | Existing account with drift → reconciliation |
| `scenarios/03-record-chat.spec.ts` | Create a client, add intake notes, chat with AI |
| `playwright.config.ts` | Viewport (1024×768 @2x), fake media device flags, Vite webServer config |
| `output/test-results/` | Per-test Playwright artefacts (each `video.webm` lives in its directory) |
| `convert.sh` | WebM → MP4 conversion for the docs site |

## Running

```bash
# First time only
cd demos
npm install
npx playwright install chromium

# Record everything
npm run record

# Or record a single scenario
npm run record:bootstrap
npm run record:sync
npm run record:chat
```

Each scenario writes a video to `output/test-results/<scenario>-*/video.webm`. `npm run collect` copies them out to `output/<scenario>.webm` with stable names. `npm run convert` (calls `convert.sh`) produces the MP4 files the docs site embeds.

## Updating claria-ai.github.io

After recording new videos:

1. `npm run record` to regenerate the WebM files
2. `npm run collect` to gather them under stable names
3. `npm run convert` to produce MP4s
4. Copy the MP4s into the [claria-ai.github.io](https://github.com/claria-ai/claria-ai.github.io) repo
5. Commit and push to publish

## Editing fixtures

Mock data lives in `fixtures.ts`. The shape mirrors what each Tauri command returns (see `crates/claria-desktop/src/commands.rs` for the source of truth). For commands that change behaviour over time (e.g. `plan` returns `freshPlanEntries` on first call, `allOkEntries` after `apply`), the state machine lives in `tauri-mock.ts`'s `buildInitScript` closure — see the `scenario` switch and `appliedOnce` flag.

## Adding a new scenario

1. Add a new `*.spec.ts` under `scenarios/`
2. Add any new fixture data to `fixtures.ts`
3. Extend `tauri-mock.ts`'s `ScenarioConfig.scenario` union and switch behaviour as needed
4. Add a `record:<name>` script to `package.json`
5. Run it to verify

## Troubleshooting

**`npm run record` hangs with no output.** Playwright `--list` and `test` both produce zero stdout/stderr and never exit. Cause: your Node is newer than the pinned `@playwright/test` supports. Fix:

```bash
node --version                                # check what's actually running
npm install @playwright/test@latest --save-dev
```

The `engines: { node }` field in `package.json` + `engines-strict=true` in `.npmrc` should refuse the wrong Node at install time, but a corrupt Homebrew symlink can sneak through — `/opt/homebrew/opt/node@22` resolving to a newer version is a real failure mode seen in this repo. If `node --version` disagrees with what you expect, fix the symlink:

```bash
brew unlink node@22 && brew link --force node@22
```

The diagnostic for the silent hang is `DEBUG=pw:* npx playwright test --list` — if you see only `pw:channel` browser-type events and nothing else, you've hit the compatibility wall.
