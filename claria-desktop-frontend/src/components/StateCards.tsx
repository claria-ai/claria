import type { ReactNode } from "react";
import Spinner from "./Spinner";

export function ErrorBanner({
  message,
  onRetry,
  retryLabel = "Try again",
  className = "mb-6",
}: {
  message: string;
  /** Renders a retry button under the message. */
  onRetry?: () => void;
  retryLabel?: string;
  /** Margin/positioning classes appended to the card. */
  className?: string;
}) {
  return (
    <div className={`bg-red-50 border border-red-200 rounded-lg p-4 ${className}`}>
      <p className="text-red-800 text-sm">{message}</p>
      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="mt-2 px-3 py-1.5 text-sm text-red-700 border border-red-300 rounded-lg hover:bg-red-100 transition-colors"
        >
          {retryLabel}
        </button>
      )}
    </div>
  );
}

export function LoadingCard({ children }: { children: ReactNode }) {
  return (
    <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 text-center">
      <div className="flex items-center justify-center gap-2 text-blue-800 text-sm">
        <Spinner />
        <span>{children}</span>
      </div>
    </div>
  );
}

export function EmptyCard({ children }: { children: ReactNode }) {
  return (
    <div className="bg-gray-50 border border-gray-200 rounded-lg p-8 text-center">
      {children}
    </div>
  );
}
