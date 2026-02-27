import type { Page } from "../App";

export default function StartScreen({
  navigate,
  configExists,
}: {
  navigate: (page: Page) => void;
  configExists: boolean;
}) {

  return (
    <div className="min-h-screen flex flex-col items-center justify-center p-8">
      <h1 className="text-4xl font-bold mb-2">Manage Your Claria</h1>
      <p className="text-gray-600 mb-12">
        Self-hosted psychological report management on AWS
      </p>

      <div className="flex flex-col gap-4 w-full max-w-xs">
        {!configExists && (
          <button
            onClick={() => navigate("guide-aws")}
            className="px-6 py-3 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors"
          >
            Create New System
          </button>
        )}

        {configExists && (
          <button
            onClick={() => navigate("dashboard")}
            className="px-6 py-3 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors"
          >
            Manage Existing System
          </button>
        )}

        <button
          onClick={() => navigate("about")}
          className="px-6 py-3 bg-white text-gray-700 rounded-lg font-medium border border-gray-300 hover:bg-gray-50 transition-colors"
        >
          About
        </button>
      </div>
    </div>
  );
}
