import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
});

test("Writing is lazy, proposal-based, editable, referenceable, and exportable", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await expect(page.locator('[data-tab="record"]')).toHaveAttribute(
    "aria-selected",
    "true",
  );

  const reportCommands = () =>
    page.evaluate(() =>
      (window as unknown as { __REPORT_COMMANDS__: string[] }).__REPORT_COMMANDS__.slice(),
    );
  expect(await reportCommands()).toEqual([]);

  // Existing Chat selection still does not opt into report IPC.
  await page.locator('[data-tab="chat"]').click();
  await expect(page.getByText("Start the conversation.")).toBeVisible();
  expect(await reportCommands()).toEqual([]);

  await page.locator('[data-tab="writing"]').click();
  await expect(page.getByText("Writing assistant")).toBeVisible();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Untitled report",
  );
  const loads = (await reportCommands()).filter(
    (command) => command === "load_report_workspace",
  );
  expect(loads.length).toBeGreaterThan(0);

  await page
    .getByLabel("Writing instruction")
    .fill("Draft an initial report from the intake and teacher records.");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByTestId("report-proposal")).toBeVisible();
  await expect(page.getByText("Complete accepted vs final report")).toHaveCount(0);
  await expect(page.getByText("Read intake-parent-interview.txt, characters 0–3200")).toBeVisible();
  const rawReadTool = page.locator("details").filter({
    hasText: "Read intake-parent-interview.txt, characters 0–3200",
  });
  await expect(rawReadTool).not.toHaveAttribute("open", "");
  await rawReadTool.locator("summary").click();
  await expect(rawReadTool).toHaveAttribute("open", "");
  await expect(rawReadTool).toContainText('"filename": "intake-parent-interview.txt"');
  const rawProposalTool = page.locator("details").filter({
    hasText: "Staged report changes for approval",
  });
  await expect(rawProposalTool).not.toHaveAttribute("open", "");
  await rawProposalTool.locator("summary").click();
  await expect(rawProposalTool).toContainText('"name": "propose_report_changes"');
  await expect(rawProposalTool).toContainText('"operations"');
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Untitled report",
  );
  await expect(page.getByTestId("accepted-report-canvas")).not.toContainText(
    "Comprehensive Evaluation",
  );

  await page.getByRole("button", { name: "Accept & save" }).click();
  await expect(page.getByTestId("report-proposal")).not.toBeVisible();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Comprehensive Evaluation",
  );
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Jane was referred for an evaluation",
  );
  await expect(page.getByTestId("queued-report-edits")).toHaveCount(0);
  await page.getByRole("button", { name: /Context/ }).click();
  await expect(
    page.getByText("intake-parent-interview.txt", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", {
    name: "Reference Summary, paragraph 1 in Writing chat",
  }).click();
  await expect(page.getByLabel("Referenced report paragraphs")).toContainText(
    "Summary ¶1",
  );

  // Unsaved canvas edits guard tab navigation.
  await page.getByRole("button", { name: "Edit" }).click();
  await page.getByLabel("Report title").fill("Unsaved local title");
  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toContain("Discard unsaved report edits");
    await dialog.dismiss();
  });
  await page.locator('[data-tab="record"]').click();
  await expect(page.locator('[data-tab="writing"]')).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByLabel("Report title")).toContainText("Unsaved local title");

  await page.getByRole("button", { name: "Discard" }).click();
  await page.getByRole("button", { name: "Export .docx" }).click();
  await expect(
    page.getByText("Word document exported from revision 1.")
  ).toBeVisible();
  const commands = await reportCommands();
  expect(commands.filter((command) => command === "send_report_message")).toHaveLength(1);
  expect(
    commands.filter((command) => command === "resolve_report_proposal"),
  ).toHaveLength(1);
  expect(commands.filter((command) => command === "export_report_docx")).toHaveLength(1);

  const invocations = await page.evaluate(() =>
    (window as unknown as {
      __REPORT_INVOCATIONS__: Array<{ cmd: string; args: Record<string, unknown> }>;
    }).__REPORT_INVOCATIONS__
  );
  expect(
    invocations.find((invocation) => invocation.cmd === "send_report_message")
      ?.args
  ).toMatchObject({ expectedRevision: 0 });
  expect(
    invocations.find(
      (invocation) => invocation.cmd === "resolve_report_proposal"
    )?.args
  ).toMatchObject({ proposalId: "proposal-1", decision: "accept" });
  expect(
    invocations.find((invocation) => invocation.cmd === "export_report_docx")
      ?.args
  ).toMatchObject({
    reportId: "99999999-9999-4999-8999-999999999999",
    expectedRevision: 1,
  });
});

test("Writing back navigation retains an unsent instruction", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();
  await page.getByLabel("Writing instruction").fill("Keep this draft");

  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByText("Jane Doe")).toBeVisible();

  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();
  await expect(page.getByLabel("Writing instruction")).toHaveValue(
    "Keep this draft",
  );
});

test("existing Chat still sends and resumes its original history contract", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.getByRole("button", { name: "Chat History" }).click();
  await page.getByTitle("Resume conversation").click();
  await expect(page.getByText("Earlier question")).toBeVisible();
  await expect(page.getByText("Earlier answer")).toBeVisible();

  await page.getByPlaceholder("Type a message...").fill("New chat question");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Unchanged Chat response")).toBeVisible();

  const state = await page.evaluate(() => ({
    chat: (window as unknown as { __CHAT_COMMANDS__: unknown[] }).__CHAT_COMMANDS__,
    report: (window as unknown as { __REPORT_COMMANDS__: string[] })
      .__REPORT_COMMANDS__,
  }));
  expect(state.chat).toHaveLength(2);
  expect(state.report).toEqual([]);
});
