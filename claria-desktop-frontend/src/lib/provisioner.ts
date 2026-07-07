import { useCallback, useState } from "react";
import type { ProvisionerProgress } from "./tauri";

export interface ScanItem {
  label: string;
  status: "pending" | "scanning" | "done";
}

export interface ApplyItem {
  label: string;
  action: string;
  status: "pending" | "in_progress" | "done";
}

/**
 * Track scan/apply checklist rows from streamed provisioner progress.
 *
 * Rows are keyed by the backend-provided index (sparse assignment) so they
 * render in backend order — apply() streams scan events before apply events,
 * and provision_apply restarts indexes for a second elevated pass.
 */
export function useProvisionerProgress() {
  const [scanItems, setScanItems] = useState<ScanItem[]>([]);
  const [applyItems, setApplyItems] = useState<ApplyItem[]>([]);

  const progressHandler = useCallback((p: ProvisionerProgress) => {
    if (p.kind === "scan_started") {
      setScanItems((prev) => {
        const next = [...prev];
        next[p.index] = { label: p.label, status: "scanning" };
        return next;
      });
    } else if (p.kind === "scan_completed") {
      setScanItems((prev) => {
        const next = [...prev];
        next[p.index] = { label: p.label, status: "done" };
        return next;
      });
    } else if (p.kind === "apply_started") {
      setApplyItems((prev) => {
        const next = [...prev];
        next[p.index] = { label: p.label, action: p.action, status: "in_progress" };
        return next;
      });
    } else if (p.kind === "apply_completed") {
      setApplyItems((prev) => {
        const next = [...prev];
        next[p.index] = { label: p.label, action: p.action, status: "done" };
        return next;
      });
    }
  }, []);

  const resetProgress = useCallback(() => {
    setScanItems([]);
    setApplyItems([]);
  }, []);

  return { scanItems, applyItems, progressHandler, resetProgress };
}
