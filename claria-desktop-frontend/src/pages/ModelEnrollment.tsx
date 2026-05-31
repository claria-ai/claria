import { useState, useEffect, useCallback, useRef } from "react";
import type { Page } from "../App";
import {
  listModelEnrollments,
  getModelEnrollment,
  getUseCaseForm,
  submitUseCaseForm,
  executeModelAgreement,
  openUrl,
  type ModelEnrollment,
  type UseCaseForm,
} from "../lib/tauri";

const POLL_INTERVAL_MS = 12_000;
const MAX_POLLS = 80; // ~16 minutes at 12s

export default function ModelEnrollmentPage({
  navigate,
  initialModelId,
  onEnrolled,
}: {
  navigate: (page: Page) => void;
  initialModelId?: string | null;
  onEnrolled?: () => void;
}) {
  const [enrollments, setEnrollments] = useState<ModelEnrollment[]>([]);
  const [ftuSubmitted, setFtuSubmitted] = useState<boolean | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [executing, setExecuting] = useState<string | null>(null);
  const pollCountRef = useRef(0);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [list, form] = await Promise.all([
        listModelEnrollments(),
        getUseCaseForm(),
      ]);
      setEnrollments(list);
      setFtuSubmitted(form.submitted);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  // Poll any pending models until they flip to executed (or we hit the cap).
  const pendingIds = enrollments
    .filter((m) => m.status.kind === "pending")
    .map((m) => m.model_id)
    .sort()
    .join(",");

  useEffect(() => {
    if (!pendingIds) {
      pollCountRef.current = 0;
      return;
    }
    const id = setInterval(async () => {
      pollCountRef.current += 1;
      if (pollCountRef.current > MAX_POLLS) {
        clearInterval(id);
        return;
      }
      const ids = pendingIds.split(",");
      const updates = await Promise.all(
        ids.map((modelId) => getModelEnrollment(modelId).catch(() => null)),
      );
      setEnrollments((prev) =>
        prev.map((m) => {
          const u = updates.find((x) => x && x.model_id === m.model_id);
          return u ?? m;
        }),
      );
      if (updates.some((u) => u && u.status.kind === "executed")) {
        onEnrolled?.();
      }
    }, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [pendingIds, onEnrolled]);

  async function handleExecute(model: ModelEnrollment) {
    setExecuting(model.model_id);
    setError(null);
    try {
      await executeModelAgreement(model.model_id);
      // Optimistically mark Pending; polling will confirm.
      setEnrollments((prev) =>
        prev.map((m) =>
          m.model_id === model.model_id
            ? { ...m, status: { kind: "pending" } }
            : m,
        ),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setExecuting(null);
    }
  }

  return (
    <div className="max-w-3xl mx-auto p-8">
      {/* Header */}
      <div className="flex items-center gap-3 mb-2">
        <button
          onClick={() => navigate("start")}
          className="text-gray-500 hover:text-gray-700 transition-colors"
          title="Back"
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
          </svg>
        </button>
        <h2 className="text-2xl font-bold">Model Access</h2>
      </div>
      <p className="text-sm text-gray-500 mb-6">
        Enroll in the Anthropic Claude models you want to use. Signing up accepts
        the model's AWS Marketplace agreement on your account.
      </p>

      {loading && (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-4">
          <Spinner />
          <span>Loading models...</span>
        </div>
      )}

      {error && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-3 mb-4">
          <p className="text-red-800 text-sm whitespace-pre-wrap">{error}</p>
          <p className="text-red-700 text-xs mt-1">
            If this says access is denied, your IAM policy may need refreshing —
            re-run setup from the AWS configuration screen.
          </p>
        </div>
      )}

      {/* First-time-use form gate */}
      {!loading && ftuSubmitted === false && (
        <UseCaseFormCard onSubmitted={load} />
      )}

      {!loading && enrollments.length > 0 && (
        <div className="space-y-3">
          {enrollments.map((m) => (
            <ModelCard
              key={m.model_id}
              model={m}
              highlighted={!!initialModelId && m.model_id === initialModelId}
              ftuSubmitted={ftuSubmitted === true}
              executing={executing === m.model_id}
              onExecute={() => handleExecute(m)}
            />
          ))}
        </div>
      )}

      {!loading && enrollments.length === 0 && !error && (
        <p className="text-sm text-gray-500">No Claude models found in this region.</p>
      )}
    </div>
  );
}

function ModelCard({
  model,
  highlighted,
  ftuSubmitted,
  executing,
  onExecute,
}: {
  model: ModelEnrollment;
  highlighted: boolean;
  ftuSubmitted: boolean;
  executing: boolean;
  onExecute: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (highlighted) ref.current?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [highlighted]);

  const status = model.status;
  return (
    <div
      ref={ref}
      className={`border rounded-lg p-4 ${
        highlighted ? "border-blue-400 ring-2 ring-blue-200" : "border-gray-200"
      }`}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="font-medium text-gray-900 truncate">{model.name}</p>
          <p className="text-xs text-gray-400 truncate">{model.model_id}</p>
        </div>
        <StatusBadge kind={status.kind} />
      </div>

      {status.kind === "blocked" && (
        <p className="text-sm text-gray-600 mt-2">{status.reason}</p>
      )}

      {status.kind === "region_unavailable" && (
        <p className="text-sm text-gray-600 mt-2">
          This model isn't offered in your configured region.
        </p>
      )}

      {status.kind === "not_authorized" && (
        <div className="mt-2">
          <p className="text-sm text-gray-600">
            Your AWS account isn't authorized to use this model yet. It's gated
            by AWS — request access in the Amazon Bedrock console, then refresh
            this page.
          </p>
          <button
            onClick={() =>
              openUrl("https://console.aws.amazon.com/bedrock/home#/modelaccess")
            }
            className="mt-2 text-xs text-blue-600 hover:underline"
          >
            Open Bedrock console ↗
          </button>
        </div>
      )}

      {status.kind === "pending" && (
        <div className="flex items-center gap-2 text-sm text-gray-600 mt-2">
          <Spinner />
          <span>Provisioning your subscription — this can take a few minutes.</span>
        </div>
      )}

      {status.kind === "executed" && (
        <p className="text-sm text-green-700 mt-2">
          Ready to use. (The first chat message is the real confirmation.)
        </p>
      )}

      {status.kind === "use_case_form_required" && (
        <p className="text-sm text-amber-700 mt-2">
          Submit the first-time use-case form above to enable this model.
        </p>
      )}

      {/* Terms + pricing + Execute for available models */}
      {(status.kind === "available" || status.kind === "use_case_form_required") &&
        model.offer && (
          <div className="mt-3 border-t border-gray-100 pt-3">
            {model.offer.pricing.length > 0 && (
              <table className="text-xs text-gray-600 w-full mb-2">
                <tbody>
                  {model.offer.pricing.map((p, i) => (
                    <tr key={i}>
                      <td className="py-0.5 pr-3">{p.description ?? p.dimension ?? "Usage"}</td>
                      <td className="py-0.5 text-right">
                        {p.price ? `$${p.price}` : "—"}
                        {p.unit ? ` / ${p.unit}` : ""}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            {model.offer.agreement_duration && (
              <p className="text-xs text-gray-400">Term: {model.offer.agreement_duration}</p>
            )}
            <div className="flex items-center gap-3 mt-2">
              {model.offer.legal_terms_url && (
                <button
                  onClick={() => openUrl(model.offer!.legal_terms_url!)}
                  className="text-xs text-blue-600 hover:underline"
                >
                  View terms ↗
                </button>
              )}
              <button
                onClick={onExecute}
                disabled={executing || !ftuSubmitted}
                title={!ftuSubmitted ? "Submit the use-case form first" : undefined}
                className="ml-auto px-4 py-1.5 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
              >
                {executing ? "Signing up..." : "Execute"}
              </button>
            </div>
          </div>
        )}
    </div>
  );
}

function StatusBadge({ kind }: { kind: ModelEnrollment["status"]["kind"] }) {
  const styles: Record<string, string> = {
    executed: "bg-green-100 text-green-800",
    available: "bg-blue-100 text-blue-800",
    pending: "bg-amber-100 text-amber-800",
    use_case_form_required: "bg-amber-100 text-amber-800",
    region_unavailable: "bg-gray-100 text-gray-600",
    not_authorized: "bg-gray-100 text-gray-600",
    blocked: "bg-gray-100 text-gray-600",
  };
  const labels: Record<string, string> = {
    executed: "Enrolled",
    available: "Available",
    pending: "Provisioning",
    use_case_form_required: "Form required",
    region_unavailable: "Region n/a",
    not_authorized: "Not available",
    blocked: "Blocked",
  };
  return (
    <span className={`shrink-0 text-xs px-2 py-0.5 rounded-full ${styles[kind] ?? styles.blocked}`}>
      {labels[kind] ?? kind}
    </span>
  );
}

function UseCaseFormCard({ onSubmitted }: { onSubmitted: () => void }) {
  const [form, setForm] = useState<UseCaseForm>({
    company_name: "",
    company_website: "",
    intended_users: 0,
    industry_option: "Healthcare",
    other_industry_option: null,
    use_cases: "",
  });
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function submit() {
    setSaving(true);
    setErr(null);
    try {
      await submitUseCaseForm(form);
      onSubmitted();
    } catch (e) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  }

  const canSubmit =
    !saving && form.company_name.trim() && form.company_website.trim() && form.use_cases.trim();

  return (
    <div className="border border-amber-300 bg-amber-50 rounded-lg p-4 mb-5">
      <p className="font-medium text-amber-900">First-time setup</p>
      <p className="text-sm text-amber-800 mt-1 mb-3">
        Anthropic requires a one-time use-case form before any Claude model can be
        enabled on your account. This is submitted once.
      </p>
      <div className="space-y-2">
        <Field label="Company name">
          <input
            value={form.company_name}
            onChange={(e) => setForm({ ...form, company_name: e.target.value })}
            className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </Field>
        <Field label="Company website">
          <input
            value={form.company_website}
            onChange={(e) => setForm({ ...form, company_website: e.target.value })}
            placeholder="https://"
            className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </Field>
        <Field label="Intended users">
          <select
            value={form.intended_users}
            onChange={(e) => setForm({ ...form, intended_users: Number(e.target.value) })}
            className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg bg-white focus:outline-none focus:ring-2 focus:ring-blue-500"
          >
            <option value={0}>Internal employees</option>
            <option value={1}>Third parties</option>
            <option value={2}>Both</option>
          </select>
        </Field>
        <Field label="Industry">
          <input
            value={form.industry_option}
            onChange={(e) => setForm({ ...form, industry_option: e.target.value })}
            className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </Field>
        <Field label="How will you use the models?">
          <textarea
            value={form.use_cases}
            onChange={(e) => setForm({ ...form, use_cases: e.target.value })}
            rows={3}
            className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </Field>
      </div>
      {err && <p className="text-red-700 text-sm mt-2 whitespace-pre-wrap">{err}</p>}
      <button
        onClick={submit}
        disabled={!canSubmit}
        className="mt-3 px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
      >
        {saving ? "Submitting..." : "Submit form"}
      </button>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="text-xs text-gray-600">{label}</span>
      <div className="mt-1">{children}</div>
    </label>
  );
}

function Spinner() {
  return (
    <svg className="animate-spin h-4 w-4 text-gray-400" viewBox="0 0 24 24" fill="none">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
    </svg>
  );
}
