# Document extraction and processing outcomes

The extraction model can differ from the chat model. It is stored only in the
existing local configuration and synchronized preferences. Unset, it means the
preferred chat model, which the account already runs, so upgrading never leaves
uploads without a model and no retired model is silently selected. Processing
fails as not configured only when neither model is set. Upload and explicit
retry use the same model and processing path. There is no automatic
retry, migration of existing records, or persisted processing registry.

The original remains at its full filename. A successful extraction is validated
as nonempty printable text before it is saved to `<filename>.text`. A failure
writes `<filename>.error` with a versioned Claria envelope, canonical reason and
code, and a random revision. Provider diagnostics and record content are not
copied into the marker. An ordinary `.error` file is neither overwritten nor
hidden. Failure to save the marker is reported in the immediate response.
Once the original is saved, processing never fails the command: every later
problem, storage included, comes back as a failed status the user can retry.

Retries observe the existing marker before inference. Only after usable text is
saved is that observed marker conditionally deleted by ETag. A concurrent newer
marker survives; its random revision prevents identical reasons from sharing a
revision. Originals and prior extracted text remain unchanged on inference or
validation failure. Conditional write conflicts preserve the winning object.

Reader inventories recognize markers by content, not extension alone. Recognized
errors supersede extracted text. Printable originals remain readable. Sidecar
eligibility comes from the listing alone: an empty or oversized sidecar is
ineligible and readers use the original. Only `.error` candidates are read to
build an inventory; sidecars are validated when they are read, and a blank or
binary one falls back to a printable original there. Preview, search, Chat,
Writer and citations use these inventory rules. Existing corpus budgets still
apply to the chosen representation.

A supported binary with no usable text and no recognized error has an unknown
previous outcome. It is offered explicit extraction, not labeled a proven
failure. Unsupported binary files are not marked failed. Processing status is
computed for responses from the listing and the recognized markers, without
reading originals or sidecars; it is never stored in another object.

Frontend coordination is held only in account-and-client-scoped Maps/Sets. It
prevents duplicate processing of one file and retains immediate failure messages
for the session. Mutations include the account/region/system scope observed by
the UI; the command checks it before processing. A drop that lands before the
scope loads waits for it instead of being discarded. A batch stops after its view
scope is disposed.

The UI distinguishes a saved original whose processing failed from an upload
whose original was not confirmed. Only the former offers retry, except when no
model is configured: a retry cannot fix that, so the file links to the extraction
model in Preferences instead. A failed retry shows the code and reason the backend
saved, read back from the listing. Unknown historical outcomes offer extraction.
File rows carry the status on their size line and an icon action like their
neighbours. Retry completion updates the file list and retains a
keyboard focus target. Screenshots use synthetic fixtures and mocked Tauri calls;
they do not qualify native WebView behavior or live AWS inference.
