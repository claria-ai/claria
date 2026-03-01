import { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import type { Page } from "../App";

export default function About({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const [version, setVersion] = useState<string>("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("unknown"));
  }, []);

  return (
    <div className="max-w-2xl mx-auto p-8">
      {/* Header with back arrow */}
      <div className="flex items-center gap-3 mb-8">
        <button
          onClick={() => navigate("start")}
          className="text-gray-500 hover:text-gray-700 transition-colors"
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
          </svg>
        </button>
        <h2 className="text-2xl font-bold">About Claria</h2>
      </div>

      <div className="space-y-4 text-gray-700">
        <p>
          Claria is a self-hosted data summarization and report writing tool
          for clinical settings, built on AWS with HIPAA-aligned infrastructure.
        </p>

        <dl className="space-y-3 mt-6">
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">Version</dt>
            <dd className="text-sm font-mono">{version}</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">Desktop App</dt>
            <dd className="text-sm">Tauri 2.x + React</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">Backend</dt>
            <dd className="text-sm">Rust + AWS SDK</dd>
          </div>
        </dl>

        <div className="flex gap-4 mt-6 text-sm">
          <a
            href="https://claria-ai.github.io"
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-600 hover:text-blue-800 underline underline-offset-2"
          >
            Website
          </a>
          <a
            href="https://github.com/claria-ai/claria"
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-600 hover:text-blue-800 underline underline-offset-2"
          >
            GitHub
          </a>
        </div>
      </div>
    </div>
  );
}
