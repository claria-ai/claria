import { useState, useEffect, useCallback } from "react";
import { hasConfig } from "./lib/tauri";
import StartScreen from "./pages/StartScreen";
import AwsAccountGuide from "./pages/AwsAccountGuide";
import IamSetupGuide from "./pages/IamSetupGuide";
import CredentialIntake from "./pages/CredentialIntake";
import ScanProvision from "./pages/ScanProvision";
import ManageDashboard from "./pages/ManageDashboard";
import About from "./pages/About";

export type Page =
  | "loading"
  | "start"
  | "guide-aws"
  | "guide-iam"
  | "credentials"
  | "scan"
  | "dashboard"
  | "about";

export default function App() {
  const [page, setPage] = useState<Page>("loading");
  const [configExists, setConfigExists] = useState(false);

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
      {page === "about" && <About navigate={navigate} />}
    </div>
  );
}
