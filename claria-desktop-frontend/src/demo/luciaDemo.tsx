import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { fixtures } from "../../../screenshots/fixtures";
import ClientRecord from "../pages/ClientRecord";
import ErrorBoundary from "../components/ErrorBoundary";
import { ChatModelsContext } from "../lib/chatModels";
import { setDistractionModeEnabled } from "../lib/distractionMode";
import { withLuciaInference } from "../lib/luciaInference";
import type { ChatModel, LocalTranscriptionStatus } from "../lib/tauri";
import "../index.css";
import "./luciaDemo.css";

// Separate local-only entry; Vite's production build includes index.html only.
// Never install mocks over the real desktop bridge or on a remotely served page.
if (!import.meta.env.DEV || !["localhost", "127.0.0.1", "[::1]"].includes(location.hostname) || "__TAURI_INTERNALS__" in window) {
  throw new Error("The Lucia demo is available only in a local development browser, not the desktop app.");
}

const CLIENT_ID = "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb";
const MODEL_ID = "us.anthropic.claude-opus-4-6-20260301-v1:0";
const models: ChatModel[] = [{ model_id: MODEL_ID, name: "Simulated response — no model connection" }];
const demoFixtures: Record<string, unknown> = {
  ...fixtures,
  list_chat_models: models,
  get_local_transcription_status: { ...(fixtures.get_local_transcription_status as LocalTranscriptionStatus), models: [] },
  chat_message: {
    chat_id: "demo-chat", chat_name: "Simulated conversation", usage: null,
    content: "This is a simulated local response. No model was called and nothing was saved. Try Lucia’s seated gaze, drop a sock, or feed up to five dekopon using her header controls.",
  },
};

mockWindows("main");
mockIPC(async (command, payload) => {
  if (command === "log_frontend_event") return null;
  if (command === "plugin:webview|get_all_webviews") return [{ label: "main", url: location.origin }];
  if (command === "chat_message") await new Promise<void>((resolve) => window.setTimeout(resolve, 5000));
  // Unsupported operations fail locally. There is deliberately no native/network fallback.
  const args = payload && !Array.isArray(payload) ? payload as Record<string, unknown> : {};
  for (const field of ["promptName", "versionId", "filename"]) {
    const key = `${command}:${args[field]}`;
    if (key in demoFixtures) return structuredClone(demoFixtures[key]);
  }
  if (command in demoFixtures) return structuredClone(demoFixtures[command]);
  throw new Error("This operation is not available in the local demo. Nothing was sent or saved.");
}, { shouldMockEvents: true });
setDistractionModeEnabled(true);

export default function LuciaDemo() {
  const [simulating, setSimulating] = useState(false);
  const [notice, setNotice] = useState("");
  const simulate = async () => {
    setSimulating(true);
    try {
      await withLuciaInference(CLIENT_ID, () => new Promise<void>((resolve) => window.setTimeout(resolve, 6000)));
    } finally {
      setSimulating(false);
    }
  };
  return <ChatModelsContext.Provider value={{ models, loading: false, error: null, preferredModelId: MODEL_ID, retry: () => {}, setPreferredModelId: () => {} }}>
    <div className="h-screen flex flex-col bg-gray-50 text-gray-900">
      <div className="shrink-0 border-b border-amber-200 bg-amber-50 px-6 py-2 text-sm flex items-center gap-3 flex-wrap" role="region" aria-label="Local demo">
        <strong>Local Lucia demo</strong>
        <span>Synthetic data · simulated activity · nothing saved · no AWS or model connection</span>
        <button type="button" className="rounded border border-amber-400 px-2 py-1 disabled:opacity-40" onClick={() => void simulate()} disabled={simulating}>
          {simulating ? "Simulating inference…" : "Simulate inference"}
        </button>
        <button type="button" className="underline" onClick={() => setDistractionModeEnabled(true)}>Show Lucia</button>
        {notice && <span role="status">{notice}</span>}
      </div>
      <div className="lucia-demo-workspace flex-1 min-h-0">
        <ClientRecord clientId={CLIENT_ID} clientName="Jane Doe — synthetic demo" luciaControlsInitiallyOpen
          navigate={() => setNotice("Stay in this demo workspace: Record, Chat and Writing are available above.")}
          onClientNameChanged={() => {}} onManageWriterSection={() => setNotice("Template management is unavailable in this local demo.")} />
      </div>
    </div>
  </ChatModelsContext.Provider>;
}

createRoot(document.getElementById("root")!).render(<StrictMode><ErrorBoundary><LuciaDemo /></ErrorBoundary></StrictMode>);
