# Chat streaming

How a chat reply gets from Bedrock to the screen, at what cadence, and how
the reader ends it early.

Chat is the only surface that renders incrementally. Extraction,
translation, and the report writer drive `ConverseStream` too, but they
consume the whole response before returning.

## The path

```
Bedrock ConverseStream frames
  → StreamCollector          accumulates full text, stop reason, usage
  → DeltaPacer               decides what is worth showing yet
  → on_delta closure         (claria-desktop)
  → tauri::ipc::Channel      ChatStreamEvent::Delta
  → ChatWidget streamText    one growing assistant bubble
```

The command's return value still carries the complete reply, so history
persistence, the audit event, and the cost ledger never depend on what the
reader saw. A caller that ignores the channel — a test, a mock, a webview
that navigated away — loses only the incremental render.

## Pacing

`claria_bedrock::pacing` sits between the collector and `on_delta`. Bedrock
emits a few characters per frame; forwarding each one re-lays out Markdown
on every frame and leaves half-formed words on screen, which is unreadable
while it happens.

| Mode | Releases |
|---|---|
| `token` | every delta, unchanged |
| `paragraph` (default) | everything up to the last `\n\n` it can see |
| `off` | nothing — the completed reply is the only delivery |

Paragraph mode has one escape hatch: text that has run past
`PARAGRAPH_FLUSH_CEILING` without closing a paragraph is released at the
last line break, else the last space. Without it a long single-paragraph
answer, a fenced code block, or a list joined by single newlines would
appear only when the turn ended.

Under `token` and `paragraph`, every byte pushed in comes back out exactly
once, in order — the live bubble and the persisted reply are the same
characters. Under `off` the callback is never called at all.

The mode is a synced preference (`chat_streaming`, config v10), so it
follows the clinician across machines.

## Stopping

Three layers, keyed by a turn id the frontend mints before it invokes
anything:

1. `ChatWidget` generates `streamId` per turn and passes it to `onSend`,
   which hands it to `chat_message` / `infra_chat`. Stop calls
   `stop_chat_stream(streamId)`.
2. The command registers a `StopSignal` under that id in
   `DesktopState::chat_stops` for the length of the turn. The registration
   is an RAII guard — every `?` in a command body is an exit path, and a
   leaked entry would keep a finished turn addressable.
3. The stream loop selects on the signal alongside the next frame. It has
   to be a `select!` rather than a check between frames: a stream can sit
   silent for minutes while a large context prefills, and a Stop button
   that waits for the next token is not a Stop button.

Stopping is not an error. The stream is dropped where it stands — which
closes the connection, so the model stops producing tokens nobody wants —
the pacer flushes whatever it was holding, and the turn returns normally
with stop reason `stopped_by_user`. The partial reply is persisted to chat
history like any other.

The trailing `metadata` frame arrives after the stop reason, so an
abandoned stream reports no usage: a stopped turn is unmetered even though
Bedrock billed the tokens it had already produced.

A `stream_id` with no live turn behind it is a no-op, not an error — the
reply may have finished between the click and the call.

## Scrolling

The widget follows the bottom of the conversation only while the reader is
already there (`isPinnedToBottom`, 48px of slack). Scrolling up detaches and
holds position until the reader returns or presses jump-to-latest; sending a
message re-attaches, since that is an explicit request to be at the bottom.

Growing text scrolls with `behavior: "auto"`. A smooth scroll restarted on
every chunk is an animation that never finishes, and it is the other half of
why a streaming reply was impossible to read.
