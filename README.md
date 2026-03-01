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

Requires Rust (2024 edition), Node.js, and npm.

```sh
# Check everything compiles
cargo check

# Build and launch the desktop app (auto-builds the frontend)
cargo run -p claria-desktop
```

## License

UNLICENSED — proprietary.
