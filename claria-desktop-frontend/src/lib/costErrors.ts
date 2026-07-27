// Cost Explorer errors reach the frontend as opaque strings from the Rust
// command layer. Two of them are recoverable by the user — a missing IAM
// permission and Cost Explorer data that has not been enabled or has not
// landed yet — and every screen that talks to Cost Explorer needs to say so in
// its own words. Classification lives here; the copy stays at the call site.

/** What an error string says went wrong, as far as the UI cares. */
export type CostErrorKind = "access_denied" | "data_unavailable" | "other";

/**
 * Classify a Cost Explorer error.
 *
 * `AccessDenied` is checked first: it is an unambiguous AWS error code,
 * whereas "not enabled" is loose prose that can appear inside a permissions
 * message too.
 */
export function classifyCostError(error: unknown): CostErrorKind {
  const msg = String(error);
  if (msg.includes("AccessDenied") || msg.includes("access denied")) {
    return "access_denied";
  }
  if (msg.includes("DataUnavailable") || msg.includes("not enabled")) {
    return "data_unavailable";
  }
  return "other";
}

/** Screen-specific wording for the two recoverable conditions. */
export interface CostErrorCopy {
  accessDenied: string;
  dataUnavailable: string;
  /** Shown when the error is neither. Defaults to the raw message. */
  fallback?: string;
}

/** Turn a Cost Explorer error into the message to put in front of the user. */
export function costErrorMessage(error: unknown, copy: CostErrorCopy): string {
  switch (classifyCostError(error)) {
    case "access_denied":
      return copy.accessDenied;
    case "data_unavailable":
      return copy.dataUnavailable;
    case "other":
      return copy.fallback ?? String(error);
  }
}
