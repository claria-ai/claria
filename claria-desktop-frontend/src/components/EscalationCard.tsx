/**
 * Notice shown above the plan when applying needs elevated credentials.
 * Purely informational — the actual CTA lives in the page actions at the
 * bottom of the plan (see `#infra-plan-actions` in InfraState).
 */
export default function EscalationCard() {
  return (
    <div className="border border-amber-300 bg-amber-50 rounded-lg p-4">
      <p className="text-amber-900 font-medium text-sm">Sync required</p>
      <p className="text-amber-800 text-sm mt-1">
        Your AWS account must be synced with Claria's latest configuration.
        Review the changes below — applying them requires temporary elevated
        credentials (root or admin), used once and then discarded.
      </p>
      <button
        onClick={() =>
          document
            .getElementById("infra-plan-actions")
            ?.scrollIntoView({ behavior: "smooth", block: "end" })
        }
        className="mt-2 text-sm text-amber-700 underline hover:text-amber-900"
      >
        Go to apply ↓
      </button>
    </div>
  );
}
