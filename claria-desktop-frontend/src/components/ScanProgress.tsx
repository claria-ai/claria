export interface ScanItem {
  label: string;
  status: "pending" | "scanning" | "done";
}

export default function ScanProgress({ items }: { items: ScanItem[] }) {
  const visible = items.filter((i) => i.status !== "pending");

  return (
    <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
      <p className="text-sm font-medium text-blue-800 mb-3">
        Scanning AWS resources...
      </p>
      <div className="space-y-1.5">
        {visible.map((item) => (
          <div
            key={item.label}
            className={`flex items-center gap-2 text-sm transition-opacity duration-300 ${
              item.status === "scanning"
                ? "text-blue-700"
                : "text-gray-500"
            }`}
          >
            {item.status === "scanning" ? (
              <svg
                className="animate-spin h-3.5 w-3.5 shrink-0"
                viewBox="0 0 24 24"
                fill="none"
              >
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </svg>
            ) : (
              <svg
                className="h-3.5 w-3.5 shrink-0 text-green-500"
                viewBox="0 0 20 20"
                fill="currentColor"
              >
                <path
                  fillRule="evenodd"
                  d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z"
                  clipRule="evenodd"
                />
              </svg>
            )}
            <span>{item.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
