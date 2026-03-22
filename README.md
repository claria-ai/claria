# Claria

Self-hosted clinical record management for psychologists. Claria deploys entirely into your own AWS account — your client data never leaves infrastructure you control.

## What does Claria do?

Claria is a desktop app that connects directly to your own AWS cloud storage. There is no middleman server — just your computer and your AWS account.

- **Client records** — create and manage client files with drag-and-drop uploads (PDFs, documents, audio)
- **AI assistant** — chat with Claude about a client's records to help draft reports, summarize notes, or ask clinical questions
- **Audio transcription** — drop in a session recording and get an automatic text transcript
- **Version history** — every change to every file is preserved; compare versions side-by-side and restore previous versions or accidentally deleted files
- **Full-text search** — search across all your records instantly
- **Guided setup** — Claria walks you through creating your AWS account, setting up security, and getting started

## AWS Bedrock

Claria uses **Amazon Bedrock** to give you access to Claude, Anthropic's AI model. Bedrock runs the AI inside your own AWS account, which means your prompts and client data stay within your AWS environment — they are not sent to Anthropic or any third party.

You enable Bedrock through the AWS console (Claria walks you through this), and then Claria handles the rest. There is nothing to install or manage on the AI side — AWS runs the model for you and charges based on usage.

## HIPAA

Claria is designed to support HIPAA compliance. Your data is encrypted at rest and in transit, every access is logged via CloudTrail, S3 versioning preserves a complete audit trail, and the IAM user Claria creates follows least-privilege principles.

However, HIPAA compliance is a shared responsibility. Claria provides the technical safeguards, but as a clinician you are responsible for understanding the administrative and physical safeguard requirements that apply to your practice. This includes signing a Business Associate Agreement (BAA) with AWS, maintaining appropriate access controls, and following your own organization's privacy policies. We recommend consulting with a HIPAA compliance specialist to ensure your overall workflow meets the requirements for handling protected health information (PHI).

## Development

### Prerequisites

- **Rust** — stable toolchain, 2024 edition. Install via [rustup](https://rustup.rs/).
- **Node.js** — any current LTS version. Install via [nvm](https://github.com/nvm-sh/nvm) or your package manager.
- **Tauri system dependencies** — Tauri needs native libraries for the webview and window chrome. See the [Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/) for your OS. On Ubuntu/Debian this is roughly:
  ```sh
  sudo apt install libwebkit2gtk-4.1-dev build-essential libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
  ```

### Running locally

Install the Tauri CLI, then use it to launch the app in dev mode:

```sh
cargo install tauri-cli --locked
cargo tauri dev
```

`cargo tauri dev` does three things:
1. Starts the Vite dev server on `http://localhost:1420` (hot-reload for JS/CSS/HTML changes)
2. Builds the Rust backend
3. Opens the desktop window pointing at the dev server

For a production build:

```sh
cargo tauri build
```

### Checks

Run these before committing:

```sh
cargo clippy -- -D warnings   # lint (warnings are errors)
cargo test                     # all workspace tests
cd claria-desktop-frontend && npm run lint  # frontend lint
```

## License

UNLICENSED — proprietary.
