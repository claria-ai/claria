import { useCallback, useState } from "react";
import type { SectionIntent } from "../lib/tauri";
import { INTENT_LABELS, type PlanRow } from "../lib/draftPlan";
import StatusChip, { type StatusChipTone } from "./StatusChip";

/** What the previous attempt left behind, said plainly beside the heading. */
const PRIOR_LABEL: Record<string, { tone: StatusChipTone; label: string }> = {
  drafted: { tone: "success", label: "Drafted" },
  flagged: { tone: "warning", label: "Drafted" },
  kept: { tone: "neutral", label: "Unchanged" },
  skipped: { tone: "muted", label: "Skipped" },
  failed: { tone: "danger", label: "Failed" },
  pending: { tone: "neutral", label: "Not written" },
  drafting: { tone: "neutral", label: "Not written" },
};

/**
 * One section of the plan, expandable.
 *
 * The card is deliberately flat and self-contained — the pattern the
 * provisioner's plan list established — but it is a writer type through and
 * through: nothing here is shared with `PlanEntryCard`, which speaks about AWS
 * resources.
 */
export default function DraftPlanCard({
  row,
  intents,
  chip,
  disabled = false,
  recordFilenames,
  onChange,
}: {
  row: PlanRow;
  /** Directives this gate offers. Empty means the plan is read-only. */
  intents: readonly SectionIntent[];
  /** The trailing state marker: a directive at the gate, live status in a run. */
  chip: { tone: StatusChipTone; label: string; animated?: boolean };
  disabled?: boolean;
  /** Every record file this client has, offered as evidence. */
  recordFilenames: readonly string[];
  onChange: (row: PlanRow) => void;
}) {
  const [addingEvidence, setAddingEvidence] = useState(false);
  const editable = intents.length > 0;
  const prior = row.priorState ? PRIOR_LABEL[row.priorState] : null;

  const update = useCallback(
    (fields: Partial<PlanRow>) => onChange({ ...row, ...fields }),
    [onChange, row]
  );

  const unused = recordFilenames.filter(
    (filename) => !row.evidence.some((item) => item.filename === filename)
  );

  const curated = row.curatedRecords;
  const toggleRestriction = useCallback(
    () => update({ curatedRecords: curated === null ? [] : null }),
    [curated, update]
  );
  const toggleRecord = useCallback(
    (filename: string) => {
      const current = curated ?? [];
      update({
        curatedRecords: current.includes(filename)
          ? current.filter((candidate) => candidate !== filename)
          : [...current, filename],
      });
    },
    [curated, update]
  );
  const useEvidenceList = useCallback(
    () =>
      update({
        curatedRecords: row.evidence.map((item) => item.filename),
      }),
    [row.evidence, update]
  );

  return (
    <details
      data-testid="draft-plan-card"
      data-section-id={row.sectionId}
      className="group rounded-lg border border-gray-200 bg-white"
    >
      <summary className="flex cursor-pointer list-none items-start gap-2 p-3 [&::-webkit-details-marker]:hidden">
        <span className="mt-0.5 shrink-0 text-xs text-gray-400 transition-transform group-open:rotate-90">
          &#9656;
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium text-gray-800">
            {row.heading}
          </span>
          {row.scope !== "" && (
            <span className="mt-0.5 block truncate text-xs text-gray-500">
              {row.scope}
            </span>
          )}
        </span>
        {curated !== null && (
          <StatusChip
            tone={curated.length === 0 ? "warning" : "info"}
            label={
              curated.length === 0
                ? "No records chosen"
                : `${curated.length} record${curated.length === 1 ? "" : "s"} only`
            }
            className="shrink-0"
          />
        )}
        {prior && (
          <StatusChip
            tone={prior.tone}
            label={
              row.priorState === "failed" && row.priorError
                ? `Failed: ${row.priorError}`
                : prior.label
            }
            className="shrink-0"
          />
        )}
        <StatusChip
          tone={chip.tone}
          label={chip.label}
          animated={chip.animated}
          className="shrink-0"
        />
      </summary>

      <div className="space-y-3 border-t border-gray-100 px-3 py-3">
        {editable && (
          <div
            role="radiogroup"
            aria-label={`Directive for ${row.heading}`}
            className="inline-flex rounded-md border border-gray-300 bg-white p-0.5"
          >
            {intents.map((intent) => {
              const selected = intent === row.intent;
              return (
                <button
                  key={intent}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  disabled={disabled}
                  onClick={() => update({ intent })}
                  className={
                    selected
                      ? "rounded px-2.5 py-1 text-xs font-semibold bg-blue-600 text-white disabled:opacity-50"
                      : "rounded px-2.5 py-1 text-xs font-medium text-gray-600 hover:bg-gray-100 disabled:opacity-50"
                  }
                >
                  {INTENT_LABELS[intent]}
                </button>
              );
            })}
          </div>
        )}

        <label className="block">
          <span className="text-[11px] font-medium text-gray-600">Scope</span>
          <textarea
            aria-label={`Scope for ${row.heading}`}
            value={row.scope}
            readOnly={!editable}
            disabled={disabled}
            rows={2}
            onChange={(event) => update({ scope: event.currentTarget.value })}
            placeholder="What this section should cover…"
            className="mt-1 w-full resize-y rounded-md border border-gray-300 bg-white px-2.5 py-1.5 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50 read-only:bg-gray-50"
          />
        </label>

        <div>
          <span className="text-[11px] font-medium text-gray-600">Evidence</span>
          <div
            aria-label={`Evidence for ${row.heading}`}
            className="mt-1 flex flex-wrap items-center gap-1.5"
          >
            {row.evidence.length === 0 && (
              <span className="text-xs text-gray-400">
                No records named yet.
              </span>
            )}
            {row.evidence.map((item) => (
              <span
                key={item.filename}
                title={item.note ?? undefined}
                className="inline-flex max-w-full items-center gap-1.5 rounded-full border border-blue-200 bg-blue-50 px-2 py-0.5 text-[11px] text-blue-800"
              >
                <span className="truncate">{item.filename}</span>
                {editable && (
                  <button
                    type="button"
                    aria-label={`Remove ${item.filename} from ${row.heading}`}
                    disabled={disabled}
                    onClick={() =>
                      update({
                        evidence: row.evidence.filter(
                          (candidate) => candidate.filename !== item.filename
                        ),
                      })
                    }
                    className="shrink-0 text-blue-500 hover:text-blue-900 disabled:opacity-50"
                  >
                    ×
                  </button>
                )}
              </span>
            ))}
            {editable && (
              <button
                type="button"
                aria-label={`Add evidence to ${row.heading}`}
                aria-expanded={addingEvidence}
                disabled={disabled}
                onClick={() => setAddingEvidence((open) => !open)}
                className="rounded-full border border-dashed border-gray-300 px-2 py-0.5 text-[11px] font-medium text-gray-600 hover:bg-gray-50 disabled:opacity-50"
              >
                Add evidence
              </button>
            )}
          </div>
          {addingEvidence && editable && (
            <div
              aria-label={`Records available to ${row.heading}`}
              className="mt-1.5 max-h-40 overflow-y-auto rounded-md border border-gray-200 bg-gray-50 p-1.5"
            >
              {unused.length === 0 ? (
                <p className="px-1 py-0.5 text-[11px] text-gray-500">
                  Every record this client has is already named here.
                </p>
              ) : (
                unused.map((filename) => (
                  <button
                    key={filename}
                    type="button"
                    disabled={disabled}
                    onClick={() => {
                      update({
                        evidence: [...row.evidence, { filename, note: null }],
                      });
                      setAddingEvidence(false);
                    }}
                    className="block w-full truncate rounded px-1 py-0.5 text-left text-[11px] text-gray-700 hover:bg-white disabled:opacity-50"
                  >
                    {filename}
                  </button>
                ))
              )}
            </div>
          )}
        </div>

        <div>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              aria-label={`Restrict drafting to selected records for ${row.heading}`}
              checked={curated !== null}
              disabled={disabled || !editable}
              onChange={toggleRestriction}
              className="h-3.5 w-3.5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
            />
            <span className="text-[11px] font-medium text-gray-600">
              Restrict drafting to selected records
            </span>
          </label>
          {curated !== null && (
            <>
              <p className="mt-1 text-[11px] text-gray-500">
                This section is written from these records alone. The rest of
                this client's records are not sent with it.
              </p>
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  disabled={disabled || !editable || row.evidence.length === 0}
                  onClick={useEvidenceList}
                  className="rounded-full border border-gray-300 px-2 py-0.5 text-[11px] font-medium text-gray-600 hover:bg-gray-50 disabled:opacity-50"
                >
                  Use evidence list
                </button>
                {curated.length === 0 && (
                  <span className="text-[11px] text-amber-700">
                    Choose at least one record, or turn the restriction off.
                  </span>
                )}
              </div>
              <div
                aria-label={`Records available to ${row.heading} while restricted`}
                className="mt-1.5 max-h-40 overflow-y-auto rounded-md border border-gray-200 bg-gray-50 p-1.5"
              >
                {recordFilenames.length === 0 ? (
                  <p className="px-1 py-0.5 text-[11px] text-gray-500">
                    This client has no records to choose from.
                  </p>
                ) : (
                  recordFilenames.map((filename) => (
                    <label
                      key={filename}
                      className="flex items-center gap-2 rounded px-1 py-0.5 hover:bg-white"
                    >
                      <input
                        type="checkbox"
                        aria-label={filename}
                        checked={curated.includes(filename)}
                        disabled={disabled || !editable}
                        onChange={() => toggleRecord(filename)}
                        className="h-3.5 w-3.5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
                      />
                      <span className="truncate text-[11px] text-gray-700">
                        {filename}
                      </span>
                    </label>
                  ))
                )}
              </div>
            </>
          )}
        </div>

        <label className="block">
          <span className="text-[11px] font-medium text-gray-600">
            Instructions for this section{" "}
            <span className="font-normal text-gray-400">· optional</span>
          </span>
          <input
            type="text"
            aria-label={`Instructions for ${row.heading}`}
            value={row.instruction}
            readOnly={!editable}
            disabled={disabled}
            onChange={(event) =>
              update({ instruction: event.currentTarget.value })
            }
            placeholder="For example: keep this to three sentences."
            className="mt-1 w-full rounded-md border border-gray-300 bg-white px-2.5 py-1.5 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50 read-only:bg-gray-50"
          />
        </label>
      </div>
    </details>
  );
}
