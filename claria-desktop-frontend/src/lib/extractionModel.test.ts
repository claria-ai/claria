import { describe, expect, it } from "vitest";
import { extractionModelUnavailable } from "./extractionModel";
describe("saved extraction model availability", () => {
  it("does not infer unavailability from failed discovery", () => {
    expect(extractionModelUnavailable("saved-model", [], false, "offline")).toBe(false);
    expect(extractionModelUnavailable("saved-model", [], true, null)).toBe(false);
    expect(extractionModelUnavailable("saved-model", [], false, null)).toBe(true);
    expect(extractionModelUnavailable(null, [], false, null)).toBe(false);
  });
});
