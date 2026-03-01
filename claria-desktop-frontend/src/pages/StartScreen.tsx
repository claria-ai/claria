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
      {/* Top-right links */}
      <div className="absolute top-4 right-6 flex items-center gap-3 text-sm text-gray-400">
        {configExists && (
          <button
            onClick={() => navigate("dashboard")}
            className="hover:text-gray-600 transition-colors flex items-center gap-1.5"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
            Manage
          </button>
        )}
        {configExists && <span className="text-gray-300">|</span>}
        <button
          onClick={() => navigate("about")}
          className="hover:text-gray-600 transition-colors"
        >
          About
        </button>
      </div>

      {/* Title */}
      <h1 className="text-4xl font-bold mb-10">Claria</h1>

      {/* Main action */}
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
            onClick={() => navigate("clients")}
            className="px-6 py-3 rounded-lg font-medium border border-gray-300 text-gray-700 hover:bg-gray-50 transition-colors flex items-center justify-center gap-2"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
            </svg>
            Client Files
          </button>
        )}
      </div>
    </div>
  );
}
