import { useState, useEffect, useCallback, useRef } from "react";
import {
  plan,
  infraChat,
  countInfraContextTokens,
  type ChatMessage,
  type ChatModel,
  type PlanEntry,
} from "../lib/tauri";
import type { Page } from "../App";
import ChatWidget from "../components/ChatWidget";

const SYSTEM_PROMPT = `You are Claria's infrastructure assistant. Claria is a desktop application for
healthcare clinicians that runs entirely in the user's own AWS account — there is
no middleman, no third-party server, and no data leaves the user's control.

## How Claria works
- The clinician installs the Claria desktop app on their computer.
- Claria provisions and manages AWS resources in the clinician's own AWS account.
- All client records, chat history, and files are stored in a private S3 bucket.
- The clinician's AWS credentials never leave their machine.

## AWS services used
- **S3**: Stores all client data — records, files, chat history, and the search index.
  Configured with versioning, server-side encryption (AES-256), and a bucket policy
  that blocks public access.
- **CloudTrail**: Audit logging — every API call to the S3 bucket is recorded.
- **Bedrock**: AI model access for chat conversations and report generation.
  Claria uses cross-region inference profiles for model availability.
- **Transcribe**: Audio transcription for voice memos.
- **IAM**: A dedicated least-privilege IAM user with a scoped policy that grants
  only the permissions Claria needs. The policy is managed by Claria and kept in sync.

## HIPAA technical safeguards
- **Encryption at rest**: S3 server-side encryption (AES-256) for all stored data.
- **Encryption in transit**: All AWS API calls use TLS.
- **Access control**: Dedicated IAM user with least-privilege policy.
- **Audit logging**: CloudTrail records all S3 data events.
- **Versioning**: S3 versioning protects against accidental deletion.
- **No public access**: Bucket policy and public access block prevent exposure.
- **BAA**: AWS Business Associate Agreement covers HIPAA-eligible services.

## Instructions
Answer questions about the infrastructure using the context below. Be specific —
reference actual resource names, their current state, and their purpose. If the
user asks whether something is configured correctly, compare the desired state to
the actual state and note any drift. Be concise and direct.`;

function buildInfraContext(entries: PlanEntry[]): string {
  let ctx = "<infrastructure_context>\n";
  for (const entry of entries) {
    ctx += `<resource label="${entry.spec.label}" type="${entry.spec.resource_type}" name="${entry.spec.resource_name}">\n`;
    ctx += `  <description>${entry.spec.description}</description>\n`;
    ctx += `  <desired_state>${JSON.stringify(entry.spec.desired, null, 2)}</desired_state>\n`;
    if (entry.actual != null) {
      ctx += `  <actual_state>${JSON.stringify(entry.actual, null, 2)}</actual_state>\n`;
    }
    ctx += `  <action>${entry.action}</action>\n`;
    ctx += `  <cause>${entry.cause}</cause>\n`;
    if (entry.drift.length > 0) {
      ctx += "  <drift>\n";
      for (const d of entry.drift) {
        ctx += `    <field name="${d.field}" expected="${JSON.stringify(d.expected)}" actual="${JSON.stringify(d.actual)}" />\n`;
      }
      ctx += "  </drift>\n";
    }
    ctx += "</resource>\n";
  }
  ctx += "</infrastructure_context>";
  return ctx;
}

export default function InfraChat({
  navigate,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
}: {
  navigate: (page: Page) => void;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
}) {
  const [scanning, setScanning] = useState(true);
  const [scanError, setScanError] = useState<string | null>(null);
  const planEntriesRef = useRef<PlanEntry[]>([]);

  const [previewModal, setPreviewModal] = useState<{
    title: string;
    content: string;
  } | null>(null);

  // Token count state
  const [contextTokens, setContextTokens] = useState<number | null>(null);
  const [countingTokens, setCountingTokens] = useState(false);
  const [tokenCountError, setTokenCountError] = useState<string | null>(null);

  useEffect(() => {
    plan()
      .then((entries) => {
        planEntriesRef.current = entries;
      })
      .catch((e) => setScanError(String(e)))
      .finally(() => setScanning(false));
  }, []);

  // Count context tokens once scan is done and models are loaded.
  useEffect(() => {
    if (scanning || chatModels.length === 0 || planEntriesRef.current.length === 0) return;
    setCountingTokens(true);
    setTokenCountError(null);
    countInfraContextTokens(chatModels[0].model_id, planEntriesRef.current)
      .then(setContextTokens)
      .catch((e) => setTokenCountError(String(e)))
      .finally(() => setCountingTokens(false));
  }, [scanning, chatModels]);

  const handleSend = useCallback(
    async (modelId: string, messages: ChatMessage[]): Promise<string> => {
      return infraChat(modelId, messages, planEntriesRef.current);
    },
    []
  );

  const toolbar = !scanning ? (
    <div className="flex items-center gap-2 px-6 py-2 border-b border-gray-100 bg-white flex-wrap">
      <span className="text-xs text-gray-400 shrink-0 inline-flex items-center gap-1">Context <TokenCountBadge counting={countingTokens} tokens={contextTokens} error={tokenCountError} />:</span>
      <button
        onClick={() =>
          setPreviewModal({ title: "System Prompt", content: SYSTEM_PROMPT })
        }
        className="shrink-0 px-2.5 py-1 text-xs font-medium text-gray-500 bg-gray-100 border border-gray-200 rounded-full hover:bg-gray-200 transition-colors"
      >
        System Prompt
      </button>
      <button
        onClick={() =>
          setPreviewModal({
            title: "Infrastructure Context",
            content: buildInfraContext(planEntriesRef.current),
          })
        }
        className="shrink-0 px-2.5 py-1 text-xs font-medium text-blue-700 bg-blue-50 border border-blue-200 rounded-full hover:bg-blue-100 transition-colors"
      >
        Infrastructure
      </button>
    </div>
  ) : null;

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <button
          onClick={() => navigate("start")}
          className="text-gray-500 hover:text-gray-700 transition-colors"
        >
          <svg
            className="w-5 h-5"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M15 19l-7-7 7-7"
            />
          </svg>
        </button>
        <div className="flex-1">
          <h2 className="text-lg font-semibold">Infrastructure</h2>
          <p className="text-xs text-gray-400">Ask about your AWS resources</p>
        </div>
      </div>

      {scanError ? (
        <div className="flex-1 flex items-center justify-center px-6">
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 max-w-md text-center">
            <p className="text-red-800 text-sm">{scanError}</p>
            <button
              onClick={() => navigate("start")}
              className="mt-3 px-4 py-1.5 text-sm text-gray-600 hover:text-gray-800"
            >
              Go back
            </button>
          </div>
        </div>
      ) : (
        <ChatWidget
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
          onSend={handleSend}
          emptyStateTitle="Ask about your infrastructure."
          emptyStateSubtitle="Ask questions about your AWS resources, security configuration, and how Claria manages your environment."
          extraLoading={scanning}
          extraLoadingText="Scanning infrastructure..."
          toolbar={toolbar}
        />
      )}

      {/* Preview modal */}
      {previewModal != null && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                {previewModal.title}
              </h3>
              <button
                onClick={() => setPreviewModal(null)}
                className="text-gray-400 hover:text-gray-600 transition-colors"
              >
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </button>
            </div>
            <div className="flex-1 overflow-y-auto border border-gray-200 rounded-lg p-4">
              <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono">
                {previewModal.content}
              </pre>
            </div>
            <div className="flex justify-end mt-4">
              <button
                onClick={() => setPreviewModal(null)}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function TokenCountBadge({
  counting,
  tokens,
  error,
}: {
  counting: boolean;
  tokens: number | null;
  error?: string | null;
}) {
  const label =
    tokens != null
      ? tokens >= 1000
        ? `~${(tokens / 1000).toFixed(1)}k tokens`
        : `~${tokens} tokens`
      : null;

  if (counting) {
    return (
      <span className="shrink-0 inline-flex items-center justify-center w-5 h-5 text-gray-400">
        <svg
          className="animate-spin h-3.5 w-3.5"
          viewBox="0 0 24 24"
          fill="none"
        >
          <circle
            className="opacity-25"
            cx="12"
            cy="12"
            r="10"
            stroke="currentColor"
            strokeWidth="4"
          />
          <path
            className="opacity-75"
            fill="currentColor"
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
          />
        </svg>
      </span>
    );
  }

  if (error) {
    return (
      <span
        className="shrink-0 inline-flex items-center justify-center w-5 h-5 rounded-full bg-red-50 border border-red-200 text-red-400 text-[10px] font-bold cursor-default group relative"
        title={error}
      >
        !
        <span className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 text-[11px] font-normal text-white bg-red-700 rounded max-w-xs whitespace-pre-wrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
          {error}
        </span>
      </span>
    );
  }

  if (label == null) return null;

  return (
    <span
      className="shrink-0 inline-flex items-center justify-center w-5 h-5 rounded-full bg-gray-100 border border-gray-200 text-gray-400 text-[10px] font-bold cursor-default group relative"
      title={label}
    >
      ?
      <span className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 text-[11px] font-normal text-white bg-gray-800 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
        {label}
      </span>
    </span>
  );
}
