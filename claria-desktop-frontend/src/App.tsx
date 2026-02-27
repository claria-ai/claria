import { useState, useEffect } from "react";
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

  useEffect(() => {
    hasConfig()
      .then((exists) => {
        setPage(exists ? "dashboard" : "start");
      })
      .catch(() => {
        setPage("start");
      });
  }, []);

  if (page === "loading") {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50">
        <p className="text-gray-500">Loading...</p>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 text-gray-900">
      {page === "start" && <StartScreen navigate={setPage} />}
      {page === "guide-aws" && <AwsAccountGuide navigate={setPage} />}
      {page === "guide-iam" && <IamSetupGuide navigate={setPage} />}
      {page === "credentials" && <CredentialIntake navigate={setPage} />}
      {page === "scan" && <ScanProvision navigate={setPage} />}
      {page === "dashboard" && <ManageDashboard navigate={setPage} />}
      {page === "about" && <About navigate={setPage} />}
    </div>
  );
}
