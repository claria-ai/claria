import { useMemo, useState } from "react";

import Modal from "./Modal";
import StatusChip from "./StatusChip";
import { reviewPropertyLabel } from "../lib/findings";
import type { ReviewPassInput, ReviewPassPreset } from "../lib/tauri";

/**
 * The list of review passes, shown before any of them fires.
 *
 * A sweep used to be one button and N silent requests. This is the same
 * button with the requests laid out first: one row per property, each showing
 * the checklist it will send, each editable in place, each removable from this
 * run. Hitting Start without touching anything runs exactly the sweep that
 * always ran.
 *
 * **Edits are for this run only.** They are not written back to the saved
 * writer prompt library, and the reason is that these are not the same kind of
 * thing. A library prompt is user input that rides into a writer turn as an
 * instruction; a reviewer checklist is one half of the host's own contract
 * with its validator — the other half (one row per section, quote verbatim,
 * call the tool once under this property's name) is composed around whatever
 * is typed here and is not editable, because a pass that dropped it would fail
 * validation and leave the property with no coverage row at all. Saving a
 * user's half account-wide would make a string nobody re-reads a standing
 * precondition for every future sweep.
 */
export default function ReviewPassPreflight({
  presets,
  open,
  busy,
  onStart,
  onCancel,
}: {
  presets: readonly ReviewPassPreset[];
  open: boolean;
  busy: boolean;
  onStart: (passes: ReviewPassInput[]) => void;
  onCancel: () => void;
}) {
  const [bodies, setBodies] = useState<Record<string, string>>({});
  const [removed, setRemoved] = useState<ReadonlySet<string>>(new Set());
  const defaults = useMemo(
    () =>
      Object.fromEntries(
        presets.map((preset) => [preset.property, preset.instructions]),
      ),
    [presets],
  );

  const kept = presets.filter((preset) => !removed.has(preset.property));
  const bodyFor = (property: string) =>
    bodies[property] ?? defaults[property] ?? "";
  const edited = (property: string) => bodyFor(property) !== defaults[property];
  const blank = kept.some((preset) => bodyFor(preset.property).trim() === "");

  const start = () => {
    onStart(
      kept.map((preset) => ({
        property: preset.property,
        // `null` is "send the shipped checklist", which keeps the request
        // bytes identical to the sweep that always ran and lets the host stay
        // the authority on its own default.
        instructions: edited(preset.property) ? bodyFor(preset.property) : null,
      })),
    );
  };

  return (
    <Modal
      open={open}
      onClose={onCancel}
      dismissible={!busy}
      title="Review the draft"
      variant="framed"
      className="flex max-h-[80vh] w-[46rem] max-w-full flex-col"
    >
      <div
        data-testid="review-pass-preflight"
        className="flex min-h-0 flex-1 flex-col"
      >
        <div className="px-5 py-3">
          <p className="text-xs leading-5 text-gray-600">
            One request per check, each reading the whole document. Edit what a
            check looks for, or drop it from this review. Edits apply to this
            review only.
          </p>
        </div>

        <div className="min-h-0 flex-1 space-y-2 overflow-y-auto bg-gray-50 px-5 py-3">
          {presets.map((preset) => {
            const isRemoved = removed.has(preset.property);
            return (
              <div
                key={preset.property}
                data-testid="review-pass-row"
                data-property={preset.property}
                data-removed={isRemoved}
                className={`rounded-lg border bg-white p-3 ${
                  isRemoved ? "border-gray-200 opacity-60" : "border-gray-200"
                }`}
              >
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-xs font-medium text-gray-800 capitalize">
                    {reviewPropertyLabel(preset.property)}
                  </span>
                  <StatusChip
                    tone={preset.pass === "style" ? "info" : "neutral"}
                    label={
                      preset.pass === "style"
                        ? "Can propose wording"
                        : "Reports only"
                    }
                  />
                  {edited(preset.property) && !isRemoved && (
                    <StatusChip tone="warning" label="Edited" />
                  )}
                  {edited(preset.property) && !isRemoved && (
                    <button
                      type="button"
                      onClick={() =>
                        setBodies((current) => {
                          const next = { ...current };
                          delete next[preset.property];
                          return next;
                        })
                      }
                      className="shrink-0 text-[11px] font-medium text-blue-700 hover:text-blue-900"
                    >
                      Reset
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() =>
                      setRemoved((current) => {
                        const next = new Set(current);
                        if (isRemoved) next.delete(preset.property);
                        else next.add(preset.property);
                        return next;
                      })
                    }
                    className="shrink-0 text-[11px] font-medium text-gray-600 hover:text-gray-900"
                  >
                    {isRemoved ? "Put back" : "Remove"}
                  </button>
                </div>

                {!isRemoved && (
                  <label className="mt-2 block">
                    <span className="sr-only">
                      What the {reviewPropertyLabel(preset.property)} check
                      looks for
                    </span>
                    <textarea
                      value={bodyFor(preset.property)}
                      onChange={(event) =>
                        setBodies((current) => ({
                          ...current,
                          [preset.property]: event.target.value,
                        }))
                      }
                      rows={6}
                      spellCheck={false}
                      className="w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-[11px] leading-4 text-gray-800"
                    />
                  </label>
                )}
                {isRemoved && (
                  <p className="mt-2 text-[11px] text-gray-500">
                    Not run. This property gets no coverage row for this
                    revision, and the audit trail records it as skipped.
                  </p>
                )}
              </div>
            );
          })}

          <details className="rounded-lg border border-gray-200 bg-white p-3">
            <summary className="cursor-pointer text-xs font-medium text-gray-700">
              The rules every check carries, whatever you type
            </summary>
            <ul className="mt-2 list-disc space-y-1 pl-4 text-[11px] leading-5 text-gray-600">
              <li>
                One row per section, including the sections it found nothing in
                — that row is how Claria knows the section was read.
              </li>
              <li>
                Quote from the section character for character; a tidied quote
                resolves against nothing and the finding is dropped.
              </li>
              <li>
                Style checks attach a replacement to every finding; consistency
                checks propose no text at all.
              </li>
              <li>
                Report only this property, and answer with exactly one call
                under its name.
              </li>
            </ul>
          </details>
        </div>

        <div className="flex items-center gap-3 border-t border-gray-200 px-5 py-3">
          <p className="min-w-0 flex-1 text-[11px] text-gray-500">
            {kept.length === 0
              ? "Keep at least one check to run a review."
              : `${kept.length} of ${presets.length} checks will run.`}
          </p>
          <button
            type="button"
            onClick={onCancel}
            className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={start}
            disabled={busy || kept.length === 0 || blank}
            title={
              blank
                ? "A check with no instructions cannot run. Write what it should look for, or remove it."
                : undefined
            }
            className="rounded-md bg-blue-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-blue-700 disabled:opacity-50"
          >
            Run {kept.length === presets.length ? "all" : kept.length} check
            {kept.length === 1 ? "" : "s"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
