import type { PlanEntry, Action } from "../lib/tauri";
import type { JsonValue } from "../lib/bindings";
import CauseBadge from "./CauseBadge";
import FieldDriftList from "./FieldDriftList";

type JsonObject = Partial<{ [key: string]: JsonValue }>;

const actionStyles: Record<Action, { icon: string; border: string }> = {
  ok: { icon: "\u2705", border: "border-green-200" },
  create: { icon: "\uD83C\uDD95", border: "border-blue-200" },
  modify: { icon: "\uD83D\uDD27", border: "border-amber-200" },
  delete: { icon: "\uD83D\uDDD1\uFE0F", border: "border-red-200" },
  precondition_failed: { icon: "\uD83D\uDD12", border: "border-amber-300" },
};

export default function PlanEntryCard({ entry }: { entry: PlanEntry }) {
  const style = actionStyles[entry.action];
  const hasDetail = entry.actual != null;

  if (!hasDetail) {
    return (
      <div className={`border ${style.border} rounded-lg`}>
        <div className="flex items-start gap-3 p-4">
          <span className="shrink-0 mt-0.5">{style.icon}</span>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="font-medium text-sm text-gray-800">
                {entry.spec.label}
              </span>
              <span className="text-xs font-mono text-gray-400">
                {entry.spec.resource_name}
              </span>
            </div>
            <p className="text-sm text-gray-600 mt-0.5">
              {entry.spec.description}
            </p>
            <CauseBadge cause={entry.cause} />
            <FieldDriftList drifts={entry.drift} />
          </div>
        </div>
      </div>
    );
  }

  return (
    <details className={`border ${style.border} rounded-lg group`}>
      <summary className="flex items-start gap-3 p-4 cursor-pointer list-none [&::-webkit-details-marker]:hidden">
        <span className="shrink-0 mt-0.5">{style.icon}</span>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium text-sm text-gray-800">
              {entry.spec.label}
            </span>
            <span className="text-xs font-mono text-gray-400">
              {entry.spec.resource_name}
            </span>
          </div>
          <p className="text-sm text-gray-600 mt-0.5">
            {entry.spec.description}
          </p>
          <CauseBadge cause={entry.cause} />
          <FieldDriftList drifts={entry.drift} />
        </div>
        <span className="shrink-0 mt-1 text-gray-400 text-xs transition-transform group-open:rotate-90">
          &#9656;
        </span>
      </summary>
      <div className="border-t border-gray-100 bg-gray-50 px-4 py-3 ml-8 rounded-b-lg">
        <ResourceDetail entry={entry} />
      </div>
    </details>
  );
}

function ResourceDetail({ entry }: { entry: PlanEntry }) {
  const raw = entry.actual;
  if (raw == null || typeof raw !== "object" || Array.isArray(raw)) {
    return raw != null ? <GenericDetail actual={raw} /> : null;
  }
  const actual = raw as JsonObject;

  switch (entry.spec.resource_type) {
    case "iam_user_policy":
      return <IamPolicyDetail actual={actual} />;
    case "s3_bucket":
      return <S3BucketDetail actual={actual} resourceName={entry.spec.resource_name} />;
    case "s3_bucket_versioning":
      return <S3VersioningDetail actual={actual} />;
    case "s3_bucket_encryption":
      return <S3EncryptionDetail actual={actual} />;
    case "s3_bucket_policy":
      return <JsonPolicyDetail actual={raw} />;
    default:
      return <GenericDetail actual={raw} />;
  }
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between text-sm">
      <dt className="text-gray-500">{label}</dt>
      <dd className="font-mono text-gray-800">{value}</dd>
    </div>
  );
}

function ScrollableJson({ data }: { data: unknown }) {
  return (
    <pre className="text-xs font-mono text-gray-700 bg-white border border-gray-200 rounded p-3 max-h-64 overflow-y-auto whitespace-pre-wrap">
      {JSON.stringify(data, null, 2)}
    </pre>
  );
}

function IamPolicyDetail({ actual }: { actual: JsonObject }) {
  const doc = actual.policy_document;
  if (!doc) return <GenericDetail actual={actual} />;
  return (
    <div className="space-y-2">
      <p className="text-xs font-medium text-gray-500 uppercase tracking-wide">
        Policy Document
      </p>
      <ScrollableJson data={doc} />
    </div>
  );
}

function S3BucketDetail({
  actual,
  resourceName,
}: {
  actual: JsonObject;
  resourceName: string;
}) {
  return (
    <dl className="space-y-2">
      <DetailRow label="Bucket Name" value={resourceName} />
      <DetailRow label="Region" value={String(actual.region ?? "unknown")} />
    </dl>
  );
}

function S3VersioningDetail({ actual }: { actual: JsonObject }) {
  return (
    <div className="space-y-2">
      <dl className="space-y-2">
        <DetailRow label="Status" value={String(actual.status ?? "unknown")} />
        <DetailRow label="Retention" value="All versions retained indefinitely" />
      </dl>
      <p className="text-xs text-gray-500 mt-2">
        S3 versioning has no built-in expiration. A lifecycle policy would need
        to be added separately to limit version count or age.
      </p>
    </div>
  );
}

function S3EncryptionDetail({ actual }: { actual: JsonObject }) {
  const algo = String(actual.sse_algorithm ?? "unknown");
  return (
    <div className="space-y-2">
      <dl className="space-y-2">
        <DetailRow label="Algorithm" value={algo} />
        <DetailRow label="Scope" value="All objects in this S3 bucket" />
      </dl>
      <p className="text-xs text-gray-500 mt-2">
        {algo === "AES256"
          ? "Data is encrypted at rest using AES-256 with Amazon S3-managed keys (SSE-S3)."
          : `Data is encrypted at rest using ${algo}.`}
      </p>
    </div>
  );
}

function JsonPolicyDetail({ actual }: { actual: unknown }) {
  return (
    <div className="space-y-2">
      <p className="text-xs font-medium text-gray-500 uppercase tracking-wide">
        Policy Document
      </p>
      <ScrollableJson data={actual} />
    </div>
  );
}

function GenericDetail({ actual }: { actual: unknown }) {
  return <ScrollableJson data={actual} />;
}
