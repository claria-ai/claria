import { expect, it } from "vitest";
import { beginProcessing, finishProcessing, processingSnapshot } from "./fileProcessing";
it("isolates accounts and prevents concurrent processing of one file", () => {
  expect(beginProcessing("account-a:client", "same.pdf")).toBe(true);
  expect(beginProcessing("account-a:client", "same.pdf")).toBe(false);
  expect(beginProcessing("account-b:client", "same.pdf")).toBe(true);
  finishProcessing("account-a:client", "same.pdf", { filename: "same.pdf", size: 1, uploaded_at: null,
    processing_status: { status: "failed", code: "invalid_text", reason: "No usable text" } });
  expect(processingSnapshot("account-a:client").failures).toHaveLength(1);
  expect(processingSnapshot("account-b:client").failures).toHaveLength(0);
  finishProcessing("account-a:client", "same.pdf");
  finishProcessing("account-b:client", "same.pdf");
  expect(processingSnapshot("account-a:client").failures).toHaveLength(0);
});
