# Document extraction and processing outcomes

The extraction model is independent of the chat model. It is stored only in the
existing local configuration and synchronized preferences. Upgrading adds an
unconfigured value; no retired model is silently selected. Upload and explicit
retry use the same selected model and processing path. There is no automatic
retry, migration of existing records, or persisted processing registry.

The original remains at its full filename. A successful extraction is validated
as nonempty printable text before it is saved to `<filename>.text`. A failure
writes `<filename>.error` with a versioned Claria envelope, canonical reason and
code, and a random revision. Provider diagnostics and record content are not
copied into the marker. An ordinary `.error` file is neither overwritten nor
hidden. Failure to save the marker is reported in the immediate response.

Retries observe the existing marker before inference. Only after usable text is
saved is that observed marker conditionally deleted by ETag. A concurrent newer
marker survives; its random revision prevents identical reasons from sharing a
revision. Originals and prior extracted text remain unchanged on inference or
validation failure. Conditional write conflicts preserve the winning object.

Reader inventories recognize markers by content, not extension alone. Recognized
errors supersede extracted text. Printable originals remain readable. Empty,
binary, and oversized extracted sidecars are ineligible and readers fall back to
the original. Preview, search, Chat, Writer and citations use these inventory
rules. Existing corpus budgets still apply to the chosen representation.

A supported binary with no usable text and no recognized error has an unknown
previous outcome. It is offered explicit extraction, not labeled a proven
failure. Unsupported binary files are not marked failed. Processing status is
computed for responses; it is never stored in another object.

Frontend coordination is held only in account-and-client-scoped Maps/Sets. It
prevents duplicate processing of one file and retains immediate failure messages
for the session. Mutations include the account/region/system scope observed by
the UI; the command checks it before processing. A batch stops after its view
scope is disposed. Catalog errors do not prove that a saved model is unavailable;
only a successful catalog response can establish absence.

The UI distinguishes a saved original whose processing failed from an upload
whose original was not confirmed. Only the former offers retry. Unknown historical
outcomes offer extraction. Retry completion updates the file list and retains a
keyboard focus target. Screenshots use synthetic fixtures and mocked Tauri calls;
they do not qualify native WebView behavior or live AWS inference.
