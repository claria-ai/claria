import { useState, useEffect, useCallback, useMemo } from "react";
import { hasConfig, loadConfig, listChatModels, type ChatModel } from "./lib/tauri";
import { ChatModelsProvider, type ChatModelsState } from "./lib/chatModels";
import StartScreen from "./pages/StartScreen";
import AwsAccountGuide from "./pages/AwsAccountGuide";
import MfaSetupGuide from "./pages/MfaSetupGuide";
import AccessKeyGuide from "./pages/AccessKeyGuide";
import Provision from "./pages/Provision";
import ClientList from "./pages/ClientList";
import ClientRecord from "./pages/ClientRecord";
import About from "./pages/About";
import Preferences from "./pages/Preferences";
import InfraChat from "./pages/InfraChat";
import CostExplorer from "./pages/CostExplorer";

export type Page =
  | "loading"
  | "start"
  | "guide-aws"
  | "guide-mfa"
  | "guide-access-key"
  | "provision"
  | "clients"
  | "client-record"
  | "infra-chat"
  | "cost-explorer"
  | "preferences"
  | "about";

export default function App() {
  const [page, setPage] = useState<Page>("loading");
  const [configExists, setConfigExists] = useState(false);
  const [activeClientId, setActiveClientId] = useState<string | null>(null);
  const [activeClientName, setActiveClientName] = useState<string | null>(null);
  const [openWriterTemplates, setOpenWriterTemplates] = useState(false);
  const [preferencesReturnPage, setPreferencesReturnPage] =
    useState<Page>("start");

  // Chat models loaded once on app startup
  const [chatModels, setChatModels] = useState<ChatModel[]>([]);
  const [chatModelsLoading, setChatModelsLoading] = useState(true);
  const [chatModelsError, setChatModelsError] = useState<string | null>(null);

  // Preferred model from config
  const [preferredModelId, setPreferredModelId] = useState<string | null>(null);

  const refreshConfig = useCallback(async () => {
    const exists = await hasConfig().catch(() => false);
    setConfigExists(exists);
    if (exists) {
      try {
        const info = await loadConfig();
        setPreferredModelId(info.preferred_model_id ?? null);
      } catch {
        // Config load failure is non-fatal here
      }
    }
    return exists;
  }, []);

  const refreshChatModels = useCallback(async () => {
    setChatModelsLoading(true);
    setChatModelsError(null);
    try {
      setChatModels(await listChatModels());
    } catch (e) {
      setChatModelsError(String(e));
    } finally {
      setChatModelsLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshConfig().then(() => {
      setPage("start");
    });
    refreshChatModels();
  }, [refreshConfig, refreshChatModels]);

  const navigate = useCallback(
    (target: Page) => {
      // Refresh config knowledge on transitions that may have changed it
      if (target === "start" || target === "provision") {
        refreshConfig();
      }
      // Retry loading models if they haven't loaded yet
      if (
        (target === "clients" || target === "client-record" || target === "infra-chat" || target === "preferences") &&
        chatModels.length === 0
      ) {
        refreshChatModels();
      }
      if (target === "preferences") {
        setOpenWriterTemplates(false);
        setPreferencesReturnPage("start");
      }
      setPage(target);
    },
    [refreshConfig, refreshChatModels, chatModels.length],
  );

  const chatModelsState = useMemo<ChatModelsState>(
    () => ({
      models: chatModels,
      loading: chatModelsLoading,
      error: chatModelsError,
      preferredModelId,
      retry: () => void refreshChatModels(),
      setPreferredModelId,
    }),
    [
      chatModels,
      chatModelsLoading,
      chatModelsError,
      preferredModelId,
      refreshChatModels,
    ]
  );

  if (page === "loading") {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50">
        <p className="text-gray-500">Loading...</p>
      </div>
    );
  }

  return (
    <ChatModelsProvider value={chatModelsState}>
    <div className="min-h-screen bg-gray-50 text-gray-900">
      {page === "start" && (
        <StartScreen navigate={navigate} configExists={configExists} />
      )}
      {page === "guide-aws" && <AwsAccountGuide navigate={navigate} />}
      {page === "guide-mfa" && <MfaSetupGuide navigate={navigate} />}
      {page === "guide-access-key" && <AccessKeyGuide navigate={navigate} />}
      {page === "provision" && <Provision navigate={navigate} />}
      {page === "clients" && (
        <ClientList
          navigate={navigate}
          onOpenClient={(id, name) => {
            setActiveClientId(id);
            setActiveClientName(name);
            navigate("client-record");
          }}
        />
      )}
      {page === "client-record" && activeClientId && (
        <ClientRecord
          navigate={navigate}
          clientId={activeClientId}
          clientName={activeClientName ?? "Client"}
          onClientNameChanged={setActiveClientName}
          onManageWriterTemplates={() => {
            setOpenWriterTemplates(true);
            setPreferencesReturnPage("client-record");
            setPage("preferences");
          }}
        />
      )}
      {page === "infra-chat" && <InfraChat navigate={navigate} />}
      {page === "preferences" && (
        <Preferences
          navigate={navigate}
          openWriterTemplates={openWriterTemplates}
          backPage={preferencesReturnPage}
        />
      )}
      {page === "cost-explorer" && <CostExplorer navigate={navigate} />}
      {page === "about" && <About navigate={navigate} />}
    </div>
    </ChatModelsProvider>
  );
}
