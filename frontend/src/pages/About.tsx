import type { Page } from "../App";

export default function About({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <h2 className="text-2xl font-bold mb-6">About Claria</h2>

      <div className="space-y-4 text-gray-700">
        <p>
          <strong>Claria</strong> is a self-hosted psychological report
          management platform built on AWS. It provides secure, HIPAA-aligned
          infrastructure for managing assessments, generating reports, and
          maintaining audit trails.
        </p>

        <dl className="space-y-3 mt-6">
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">Version</dt>
            <dd className="text-sm font-mono">0.1.0</dd>
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
      </div>

      <div className="mt-8">
        <button
          onClick={() => navigate("start")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Back
        </button>
      </div>
    </div>
  );
}
