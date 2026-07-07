import type { ReactNode } from "react";
import Spinner from "./Spinner";

export function ErrorBanner({ message }: { message: string }) {
  return (
    <div className="bg-red-50 border border-red-200 rounded-lg p-4 mb-6">
      <p className="text-red-800 text-sm">{message}</p>
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
