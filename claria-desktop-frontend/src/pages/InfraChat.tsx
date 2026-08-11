import { useState, useCallback, useMemo } from "react";
import {
  plan,
  infraChat,
  countInfraContextTokens,
  type ChatMessage,
  type PlanEntry,
} from "../lib/tauri";
import type { Page } from "../App";
import ChatWidget from "../components/ChatWidget";
import { BackButton } from "../components/icons";
import TextPreviewModal from "../components/TextPreviewModal";
import { ErrorBanner } from "../components/StateCards";
import TokenCountBadge from "../components/TokenCountBadge";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { useContextTokens } from "../lib/useContextTokens";

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
}: {
  navigate: (page: Page) => void;
}) {
  const planLoad = useAsyncLoad(() => plan(), []);
  const scanning = planLoad.loading;
  const scanError = planLoad.error;
  const planEntries = useMemo(() => planLoad.data ?? [], [planLoad.data]);

  const [previewModal, setPreviewModal] = useState<{
    title: string;
    content: string;
  } | null>(null);

  // Count context tokens once the scan has produced a plan.
  const countContext = useCallback(
    () => countInfraContextTokens(planEntries),
    [planEntries]
  );
  const {
    tokens: contextTokens,
    counting: countingTokens,
    error: tokenCountError,
  } = useContextTokens(planEntries.length === 0 ? null : countContext);

  const handleSend = useCallback(
    async (
      modelId: string,
      messages: ChatMessage[],
      onDelta: (text: string) => void
    ) => {
      const response = await infraChat(modelId, messages, planEntries, (event) => {
        if (event.kind === "delta") onDelta(event.text);
      });
      return { content: response.content, usage: response.usage };
    },
    [planEntries]
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
            content: buildInfraContext(planEntries),
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
        <BackButton onClick={() => navigate("start")} />
        <div className="flex-1">
          <h2 className="text-lg font-semibold">Infrastructure</h2>
          <p className="text-xs text-gray-400">Ask about your AWS resources</p>
        </div>
      </div>

      {scanError ? (
        <div className="flex-1 flex items-center justify-center px-6">
          <ErrorBanner
            message={scanError}
            onRetry={() => navigate("start")}
            retryLabel="Go back"
            className="max-w-md"
          />
        </div>
      ) : (
        <ChatWidget
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
        <TextPreviewModal
          filename={previewModal.title}
          text={previewModal.content}
          onClose={() => setPreviewModal(null)}
        />
      )}
    </div>
  );
}
