import { useState } from "react";
import Spinner from "./Spinner";
import { ErrorBanner } from "./StateCards";
import type { AccessKeyInfo, AccessKeyLimitReached } from "../lib/tauri";

/**
 * Recovery UI for IAM's two-access-key ceiling.
 *
 * Each existing key is almost certainly in use by another computer running
 * Claria, and IAM gives no way to tell which. Deleting the wrong one locks
 * that computer out until it is onboarded again, so nothing here is
 * pre-selected and every deletion goes through a per-key confirmation that
 * names the key being destroyed.
 */
export default function AccessKeyLimitPanel({
  limit,
  keys,
  loadingKeys,
  keysError,
  deletingKeyId,
  onDelete,
  onCancel,
}: {
  limit: AccessKeyLimitReached;
  keys: AccessKeyInfo[];
  loadingKeys: boolean;
  keysError: string | null;
  deletingKeyId: string | null;
  onDelete: (keyId: string) => void;
  onCancel: () => void;
}) {
  const [confirmKeyId, setConfirmKeyId] = useState<string | null>(null);
  const busy = deletingKeyId !== null;

  return (
    <div className="space-y-4">
      <div className="bg-amber-50 border border-amber-200 rounded-lg p-4">
        <p className="text-sm font-medium text-amber-900 mb-1">
          Access key limit reached
        </p>
        <p className="text-sm text-amber-800">
          AWS allows the{" "}
          <code className="text-xs bg-amber-100 px-1 rounded">
            {limit.user_name}
          </code>{" "}
          user at most {limit.limit} access keys, and both slots are taken.
          Claria needs one for this computer.
        </p>
        <p className="text-xs text-amber-700 mt-2 font-mono break-words">
          {limit.message}
        </p>
      </div>

      <div className="bg-red-50 border border-red-300 rounded-lg p-4">
        <p className="text-sm font-semibold text-red-900 mb-1">
          Deleting a key locks a computer out of Claria
        </p>
        <p className="text-sm text-red-800">
          Each key below belongs to a computer that has already been set up.
          Deleting one revokes that computer's access immediately, and it will
          have to be onboarded again from scratch. AWS cannot undo this and
          cannot bring the key back.
        </p>
        <p className="text-sm text-red-800 mt-2">
          Use the created and last-used dates to find the key you no longer
          need. If you are not sure, cancel and check the other computer first.
        </p>
      </div>

      {keysError && (
        <ErrorBanner
          message={`Could not list the existing keys: ${keysError}`}
          className=""
        />
      )}

      {loadingKeys ? (
        <p className="text-sm text-gray-500 flex items-center gap-2">
          <Spinner /> Loading existing keys...
        </p>
      ) : (
        <div className="space-y-3">
          {keys.map((key) => (
            <div
              key={key.access_key_id}
              className="border rounded-lg p-3 space-y-3"
            >
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <p className="text-sm font-mono text-gray-800 break-all">
                    {key.access_key_id}
                  </p>
                  <div className="flex flex-wrap gap-x-4 gap-y-0.5 mt-1">
                    <span
                      className={`text-xs ${
                        key.status === "Active"
                          ? "text-green-700"
                          : "text-gray-500"
                      }`}
                    >
                      {key.status}
                    </span>
                    <span className="text-xs text-gray-500">
                      Created: {formatDate(key.created_at)}
                    </span>
                    <span className="text-xs text-gray-500">
                      {key.last_used_at
                        ? `Last used: ${formatDate(key.last_used_at)}${
                            key.last_used_service
                              ? ` (${key.last_used_service})`
                              : ""
                          }`
                        : "Last used: never"}
                    </span>
                  </div>
                </div>
                {confirmKeyId !== key.access_key_id && (
                  <button
                    onClick={() => setConfirmKeyId(key.access_key_id)}
                    disabled={busy}
                    className="flex-shrink-0 px-3 py-1.5 text-sm text-red-700 bg-red-50 border border-red-200 rounded-lg hover:bg-red-100 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    {deletingKeyId === key.access_key_id ? (
                      <span className="flex items-center gap-1.5">
                        <Spinner /> Deleting...
                      </span>
                    ) : (
                      "Delete this key"
                    )}
                  </button>
                )}
              </div>

              {confirmKeyId === key.access_key_id && (
                <div className="bg-red-50 border border-red-300 rounded-lg p-3">
                  <p className="text-sm text-red-900 mb-3">
                    Permanently delete{" "}
                    <span className="font-mono break-all">
                      {key.access_key_id}
                    </span>
                    ? Whichever computer is using this key loses access to
                    Claria the moment you confirm.
                  </p>
                  <div className="flex gap-2">
                    <button
                      onClick={() => setConfirmKeyId(null)}
                      disabled={busy}
                      className="flex-1 py-2 border rounded-lg text-sm bg-white disabled:opacity-50"
                    >
                      Keep this key
                    </button>
                    <button
                      onClick={() => onDelete(key.access_key_id)}
                      disabled={busy}
                      className="flex-1 py-2 bg-red-600 text-white rounded-lg text-sm hover:bg-red-700 disabled:opacity-50"
                    >
                      {deletingKeyId === key.access_key_id
                        ? "Deleting..."
                        : "Yes, delete it"}
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}

          {keys.length === 0 && !keysError && (
            <p className="text-sm text-gray-500">
              AWS reported no keys for this user. Retry the setup — the limit
              may have been resolved elsewhere.
            </p>
          )}
        </div>
      )}

      <button
        onClick={onCancel}
        disabled={busy}
        className="text-sm text-gray-500 hover:text-gray-700 disabled:opacity-50"
      >
        Cancel — leave every key alone
      </button>
    </div>
  );
}

/** Render an ISO 8601 timestamp as a local date, or "unknown" if absent. */
function formatDate(iso: string | null): string {
  if (!iso) return "unknown";
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) return iso;
  return parsed.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
