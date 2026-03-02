import { useState, useEffect, useCallback } from "react";
import { hasConfig, loadConfig, listChatModels, type ChatModel } from "./lib/tauri";
import StartScreen from "./pages/StartScreen";
import AwsAccountGuide from "./pages/AwsAccountGuide";
import MfaSetupGuide from "./pages/MfaSetupGuide";
import AccessKeyGuide from "./pages/AccessKeyGuide";
import CredentialIntake from "./pages/CredentialIntake";
import ScanProvision from "./pages/ScanProvision";
import AwsManage from "./pages/AwsManage";
import ClientList from "./pages/ClientList";
import ClientChat from "./pages/ClientChat";
import ClientRecord from "./pages/ClientRecord";
import About from "./pages/About";
import Preferences from "./pages/Preferences";
import InfraChat from "./pages/InfraChat";

export type Page =
  | "loading"
  | "start"
  | "guide-aws"
  | "guide-mfa"
  | "guide-access-key"
  | "credentials"
  | "scan"
  | "aws"
  | "clients"
  | "client-record"
  | "client-chat"
  | "infra-chat"
  | "preferences"
  | "about";

export default function App() {
  const [page, setPage] = useState<Page>("loading");
  const [configExists, setConfigExists] = useState(false);
  const [activeClientId, setActiveClientId] = useState<string | null>(null);
  const [activeClientName, setActiveClientName] = useState<string | null>(null);

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
      if (target === "start" || target === "aws") {
        refreshConfig();
      }
      // Retry loading models if they haven't loaded yet
      if (
        (target === "clients" || target === "client-record" || target === "client-chat" || target === "infra-chat" || target === "preferences") &&
        chatModels.length === 0
      ) {
        refreshChatModels();
      }
      setPage(target);
    },
    [refreshConfig, refreshChatModels, chatModels.length],
  );

  if (page === "loading") {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50">
        <p className="text-gray-500">Loading...</p>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 text-gray-900">
      {page === "start" && (
        <StartScreen navigate={navigate} configExists={configExists} />
      )}
      {page === "guide-aws" && <AwsAccountGuide navigate={navigate} />}
      {page === "guide-mfa" && <MfaSetupGuide navigate={navigate} />}
      {page === "guide-access-key" && <AccessKeyGuide navigate={navigate} />}
      {page === "credentials" && <CredentialIntake navigate={navigate} />}
      {page === "scan" && <ScanProvision navigate={navigate} />}
      {page === "aws" && <AwsManage navigate={navigate} />}
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
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
        />
      )}
      {page === "client-chat" && activeClientId && (
        <ClientChat
          navigate={navigate}
          clientId={activeClientId}
          clientName={activeClientName ?? "Client"}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
        />
      )}
      {page === "infra-chat" && (
        <InfraChat
          navigate={navigate}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
        />
      )}
      {page === "preferences" && (
        <Preferences
          navigate={navigate}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
          onPreferredModelChanged={setPreferredModelId}
        />
      )}
      {page === "about" && <About navigate={navigate} />}
    </div>
  );
}
