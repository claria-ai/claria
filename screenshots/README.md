# Screenshot Capture

Automated screenshot generation for [claria-ai.github.io](https://claria-ai.github.io) using Playwright.

## How it works

The screenshots directory contains a Playwright test suite that captures screenshots of every page in the Claria desktop app. Instead of launching the full Tauri backend, the tests run against the Vite dev server (`localhost:1420`) with a mock of `window.__TAURI_INTERNALS__` injected before the app loads. This lets us render the real React UI with fixture data and no Rust/AWS dependencies.

Key files:

| File | Purpose |
|---|---|
| `fixtures.ts` | Mock IPC responses keyed by Tauri command name |
| `tauri-mock.ts` | Builds the `addInitScript` payload that stubs `window.__TAURI_INTERNALS__.invoke` |
| `capture.spec.ts` | Playwright tests — one per screenshot |
| `playwright.config.ts` | Viewport (1024×768 @2x), fake media device flags, Vite webServer config |
| `output/` | Generated PNGs (git-ignored) |

## Running

```bash
# First time only
cd screenshots
npm install
npx playwright install chromium

# Capture all screenshots (starts Vite dev server automatically)
npm run capture
```

The Vite dev server is started automatically by Playwright via the `webServer` config. If you already have it running on `:1420`, Playwright will reuse it.

## Output

Screenshots are written to `output/` at 2× resolution (Retina):

- `start.png` — Start/home screen
- `about.png` — About page
- `preferences.png` — Preferences with all sections expanded
- `clients.png` — Client list
- `client-record.png` — Client record (files tab)
- `client-chat.png` — Client chat with AI response
- `memo-recording.png` — Voice memo recording in progress
- `memo-review.png` — Voice memo review modal
- `aws.png` — AWS resource management

## Updating claria-ai.github.io

The generated PNGs are copied into the [claria-ai.github.io](https://github.com/claria-ai/claria-ai.github.io) repository and referenced by the landing page. After capturing new screenshots:

1. Run `npm run capture` here to regenerate `output/*.png`
2. Copy the PNGs into the github.io repo's image directory
3. Commit and push to publish the updated site

## Editing fixtures

All mock data lives in `fixtures.ts`. The fixture keys match Tauri IPC command names (e.g. `list_clients`, `plan`, `load_config`). For commands that take arguments, use `"command:arg"` keys — see `get_prompt:system-prompt` and `get_prompt:pdf-extraction` for an example. The arg-based routing is handled in `tauri-mock.ts`.

## Adding a new screenshot

1. Add fixture data for any new IPC commands in `fixtures.ts`
2. Add `data-*` attributes to React components if Playwright needs stable selectors
3. Add a new `test(...)` block in `capture.spec.ts`
4. Run `npm run capture` to verify
