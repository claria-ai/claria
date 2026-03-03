import { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { checkForUpdates, openUrl } from "../lib/tauri";
import type { UpdateCheck } from "../lib/tauri";
import type { Page } from "../App";

export default function About({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const [version, setVersion] = useState<string>("");
  const [update, setUpdate] = useState<UpdateCheck | null>(null);

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("unknown"));
    checkForUpdates().then(setUpdate).catch(() => {});
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

        {update?.update_available && (
          <div className="mt-4 p-3 bg-blue-50 border border-blue-200 rounded-lg text-sm text-blue-800">
            Update available:{" "}
            <button
              onClick={() => openUrl(update.release_url)}
              className="font-semibold underline underline-offset-2 hover:text-blue-600"
            >
              v{update.latest_version}
            </button>
          </div>
        )}

        <div className="flex flex-col gap-2 mt-6 text-sm">
          <button
            onClick={() => openUrl("https://claria-ai.github.io")}
            className="text-blue-600 hover:text-blue-800 underline underline-offset-2 text-left"
          >
            Claria-AI
          </button>
          <button
            onClick={() => openUrl("https://github.com/claria-ai/claria")}
            className="text-blue-600 hover:text-blue-800 underline underline-offset-2 text-left"
          >
            Browse Claria AI's Open Source Code
          </button>
          <button
            onClick={() => openUrl("https://docs.anthropic.com/en/release-notes/system-prompts")}
            className="text-blue-600 hover:text-blue-800 underline underline-offset-2 text-left"
          >
            Anthropic's Claude System Prompts
          </button>
          <button
            onClick={() => openUrl("https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices")}
            className="text-blue-600 hover:text-blue-800 underline underline-offset-2 text-left"
          >
            Claude Prompting Best Practices
          </button>
        </div>
      </div>
    </div>
  );
}
