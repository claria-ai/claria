import { describe, expect, it } from "vitest";
import { findEscalationEntry, hasChanges, unreadableEntries } from "./plan";
import type { Action, CredentialScope, PlanEntry } from "./tauri";

function entry(
  resourceName: string,
  action: Action,
  credentialScope: CredentialScope
): PlanEntry {
  return {
    spec: {
      resource_type: "s3_bucket",
      resource_name: resourceName,
      lifecycle: "managed",
      label: resourceName,
      description: "",
      severity: "normal",
      credential_scope: credentialScope,
      iam_actions: [],
      desired: null,
    },
    action,
    cause:
      action === "ok" ? "in_sync" : action === "unknown" ? "unreadable" : "missing",
    drift: [],
    actual: null,
    error: action === "unknown" ? "AWS error: s3:HeadBucket failed" : null,
  };
}

describe("hasChanges", () => {
  it("is false before a scan has run", () => {
    expect(hasChanges(null)).toBe(false);
  });

  it("is false for an empty plan", () => {
    expect(hasChanges([])).toBe(false);
  });

  it("is false when every resource is already in sync", () => {
    expect(hasChanges([entry("a", "ok", "regular"), entry("b", "ok", "regular")])).toBe(
      false
    );
  });

  it("is true for any non-ok action", () => {
    for (const action of ["create", "modify", "delete", "precondition_failed"] as const) {
      expect(hasChanges([entry("a", "ok", "regular"), entry("b", action, "regular")])).toBe(
        true
      );
    }
  });

  // Nothing is known about an unreadable resource, so Apply has nothing to
  // do with it. Counting it would light up a button that cannot change it.
  it("is false when the only non-ok entry could not be read", () => {
    expect(
      hasChanges([entry("a", "ok", "regular"), entry("b", "unknown", "regular")])
    ).toBe(false);
  });

  it("is still true when something readable needs changing alongside", () => {
    expect(
      hasChanges([entry("a", "unknown", "regular"), entry("b", "create", "regular")])
    ).toBe(true);
  });
});

describe("unreadableEntries", () => {
  it("is empty before a scan has run", () => {
    expect(unreadableEntries(null)).toEqual([]);
  });

  it("picks out only the resources the scan could not read", () => {
    const found = unreadableEntries([
      entry("bucket", "unknown", "regular"),
      entry("trail", "create", "regular"),
      entry("policy", "ok", "elevated"),
    ]);
    expect(found.map((e) => e.spec.resource_name)).toEqual(["bucket"]);
  });
});

describe("findEscalationEntry", () => {
  it("is null before a scan has run", () => {
    expect(findEscalationEntry(null)).toBeNull();
  });

  it("finds an elevated resource that needs creating", () => {
    const found = findEscalationEntry([
      entry("bucket", "create", "regular"),
      entry("baa", "create", "elevated"),
    ]);
    expect(found?.spec.resource_name).toBe("baa");
  });

  it("finds an elevated resource that needs modifying", () => {
    expect(findEscalationEntry([entry("baa", "modify", "elevated")])?.spec.resource_name).toBe(
      "baa"
    );
  });

  it("ignores an elevated resource that is already in sync", () => {
    expect(findEscalationEntry([entry("baa", "ok", "elevated")])).toBeNull();
  });

  it("ignores changes that regular credentials can make", () => {
    expect(findEscalationEntry([entry("bucket", "create", "regular")])).toBeNull();
  });

  it("returns the first elevated entry, so the prompt is stable across scans", () => {
    const found = findEscalationEntry([
      entry("baa", "create", "elevated"),
      entry("bedrock", "create", "elevated"),
    ]);
    expect(found?.spec.resource_name).toBe("baa");
  });
});
