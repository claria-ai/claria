import { useState, useEffect, useCallback } from "react";
import { hasConfig } from "./lib/tauri";
import StartScreen from "./pages/StartScreen";
import AwsAccountGuide from "./pages/AwsAccountGuide";
import IamSetupGuide from "./pages/IamSetupGuide";
import CredentialIntake from "./pages/CredentialIntake";
import ScanProvision from "./pages/ScanProvision";
import ManageDashboard from "./pages/ManageDashboard";
import ClientList from "./pages/ClientList";
import ClientChat from "./pages/ClientChat";
import ClientRecord from "./pages/ClientRecord";
import About from "./pages/About";

export type Page =
  | "loading"
  | "start"
  | "guide-aws"
  | "guide-iam"
  | "credentials"
  | "scan"
  | "dashboard"
  | "clients"
  | "client-record"
  | "client-chat"
  | "about";

export default function App() {
  const [page, setPage] = useState<Page>("loading");
  const [configExists, setConfigExists] = useState(false);
  const [activeClientId, setActiveClientId] = useState<string | null>(null);
  const [activeClientName, setActiveClientName] = useState<string | null>(null);

  const refreshConfig = useCallback(async () => {
    const exists = await hasConfig().catch(() => false);
    setConfigExists(exists);
    return exists;
  }, []);

  useEffect(() => {
    refreshConfig().then((exists) => {
      setPage(exists ? "dashboard" : "start");
    });
  }, [refreshConfig]);

  const navigate = useCallback(
    (target: Page) => {
      // Refresh config knowledge on transitions that may have changed it
      if (target === "start" || target === "dashboard") {
        refreshConfig();
      }
      setPage(target);
    },
    [refreshConfig],
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
      {page === "guide-iam" && <IamSetupGuide navigate={navigate} />}
      {page === "credentials" && <CredentialIntake navigate={navigate} />}
      {page === "scan" && <ScanProvision navigate={navigate} />}
      {page === "dashboard" && <ManageDashboard navigate={navigate} />}
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
        />
      )}
      {page === "client-chat" && activeClientId && (
        <ClientChat
          navigate={navigate}
          clientId={activeClientId}
          clientName={activeClientName ?? "Client"}
        />
      )}
      {page === "about" && <About navigate={navigate} />}
    </div>
  );
}
