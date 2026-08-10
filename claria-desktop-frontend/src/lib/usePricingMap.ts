// Resolve `ModelPricing` for every model that has produced a turn, so
// ledger math can price mixed-model sessions. Results accumulate across
// renders: each model id is looked up at most once per mount, and a failed
// or absent lookup is remembered (and logged through the backend bridge)
// rather than retried in a loop.

import { useEffect, useRef, useState } from "react";
import { logFrontendEvent } from "./logBridge";
import { lookupModelPricing, type ModelPricing } from "./tauri";

export function usePricingMap(
  modelIds: readonly string[]
): Map<string, ModelPricing> {
  const [pricingByModel, setPricingByModel] = useState<
    Map<string, ModelPricing>
  >(() => new Map());
  const attemptedRef = useRef(new Set<string>());
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // A stable key so the lookup effect re-runs only when the *set* of
  // models changes, not when the caller rebuilds the array each render.
  const key = Array.from(new Set(modelIds)).sort().join("\n");

  useEffect(() => {
    const pending =
      key === ""
        ? []
        : key.split("\n").filter((id) => !attemptedRef.current.has(id));
    if (pending.length === 0) return;
    for (const id of pending) attemptedRef.current.add(id);
    void Promise.all(
      pending.map(async (modelId) => {
        try {
          return [modelId, await lookupModelPricing(modelId)] as const;
        } catch (reason) {
          logFrontendEvent(
            "warn",
            `Pricing lookup failed for ${modelId}: ${String(reason)}`
          );
          return [modelId, null] as const;
        }
      })
    ).then((resolved) => {
      if (!mountedRef.current) return;
      const found = resolved.filter(
        (pair): pair is readonly [string, ModelPricing] => pair[1] != null
      );
      if (found.length === 0) return;
      setPricingByModel((prev) => {
        const next = new Map(prev);
        for (const [modelId, pricing] of found) next.set(modelId, pricing);
        return next;
      });
    });
  }, [key]);

  return pricingByModel;
}
