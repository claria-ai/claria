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
  await expect(page.getByRole("button", { name: "Rename writer session" })).toBeVisible();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Untitled report",
  );
  const starts = (await reportCommands()).filter(
    (command) => command === "start_report_workspace",
  );
  expect(starts.length).toBeGreaterThan(0);
  await expect(page.getByRole("tab", { name: "Get started" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.getByRole("tab", { name: "Write with Claude" }).click();

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
  await expect(page.getByTestId("report-table")).toContainText("Needs support");
  await expect(page.getByTestId("queued-report-edits")).toHaveCount(0);
  await page.getByRole("button", { name: /Context/ }).click();
  await expect(
    page.locator("#writing-context-control").getByText(
      "intake-parent-interview.txt",
      { exact: true },
    ),
  ).toBeVisible();
  await page.getByRole("button", {
    name: "Reference Summary, paragraph 1 in Writing chat",
  }).click();
  await page.getByRole("button", {
    name: "Reference Summary, table 3 in Writing chat",
  }).click();
  const reportReferences = page.getByLabel("Referenced report blocks");
  await expect(reportReferences).toContainText("Summary ¶1");
  await expect(reportReferences).toContainText(
    "Summary table 3: Domain | Finding · Attention | Needs support",
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

  await page
    .getByLabel("Accepted report draft")
    .getByRole("button", { name: "Discard" })
    .click();
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
  const startedReportId = invocations.find(
    (invocation) => invocation.cmd === "start_report_workspace"
  )?.args.reportId;
  expect(
    invocations.find((invocation) => invocation.cmd === "export_report_docx")
      ?.args
  ).toMatchObject({
    reportId: startedReportId,
    expectedRevision: 1,
  });
});

test("a planned whole-report draft is reviewed at the gate, then saved in one revision", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();

  // Reproduce replacement of a template-backed draft, not only generation
  // from the empty report.
  await page.getByRole("button", { name: "Apply template" }).click();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Imported Evaluation Template",
  );
  await page
    .getByLabel("Full report guidance")
    .fill("Use a concise clinical style.");
  await page.getByRole("button", { name: "Fill whole report" }).click();
  const confirmation = page.getByRole("dialog", {
    name: "Replace the working draft?",
  });
  await expect(confirmation).toContainText(
    "current revision will remain available",
  );
  await confirmation
    .getByRole("button", { name: "Fill whole report" })
    .click();

  // The gate: the plan is readable and editable, and nothing is written yet.
  await expect(
    page.getByText("Plan ready — review before drafting"),
  ).toBeVisible();
  await expect(page.getByTestId("plan-warnings")).toContainText(
    "unresolved_evidence:assessment-scores.json",
  );
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Imported Evaluation Template",
  );
  await page
    .getByTestId("draft-plan-card")
    .filter({ hasText: "Assessment Scores" })
    .locator("summary")
    .click();
  await page
    .getByLabel("Scope for Assessment Scores")
    .fill("Only the attention measures.");
  await page
    .getByRole("button", { name: "Start drafting (1 sections)" })
    .click();

  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Complete Generated Evaluation",
  );
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Drafted Assessment Scores from the client records.",
  );
  await expect(page.getByTestId("report-proposal")).toHaveCount(0);
  await expect(
    page.getByText(/Generated and saved revision 2 from 3 readable records/),
  ).toBeVisible();
  await expect(page.getByLabel("Writing instruction")).toHaveValue("");

  await page.getByRole("button", { name: /Context/ }).click();
  const writerContext = page.getByLabel("Writer context");
  await expect(writerContext.getByText("intake-parent-interview.txt")).toBeVisible();
  await expect(writerContext.getByText("teacher-observation.txt")).toBeVisible();
  await expect(writerContext.getByText("assessment-scores.json")).toBeVisible();
  await writerContext
    .getByRole("button", { name: "teacher-observation.txt" })
    .click();
  const preview = page.getByRole("dialog", { name: "teacher-observation.txt" });
  await expect(preview).toContainText(
    "Teacher observation record used for the complete report.",
  );
  await preview.getByRole("button", { name: "Close" }).first().click();

  await page.getByRole("tab", { name: "Get started" }).click();
  await expect(
    page.getByRole("button", { name: "Fill whole report" }),
  ).toHaveCount(0);
  await expect(page.getByLabel("Full report guidance")).toHaveCount(0);

  await page.getByRole("tab", { name: "Costs and cache" }).click();
  const usagePanel = page.getByTestId("session-usage-panel");
  await expect(usagePanel).toContainText("4,200 tok");
  await expect(usagePanel).toContainText("3,600 tok");
  await page.getByLabel("Show turn costs").check();
  await page.getByRole("tab", { name: "Write with Claude" }).click();
  await expect(page.getByText("$0.016", { exact: true })).toBeVisible();
  await expect(page.getByText("4,200 tok cached")).toBeVisible();
  await expect(page.getByText("10 tok new")).toBeVisible();

  const invocations = await page.evaluate(() =>
    (window as unknown as {
      __REPORT_INVOCATIONS__: Array<{ cmd: string; args: Record<string, unknown> }>;
    }).__REPORT_INVOCATIONS__,
  );
  expect(
    invocations.find((invocation) => invocation.cmd === "generate_draft_plan")
      ?.args,
  ).toMatchObject({
    expectedRevision: 1,
    instructions: "Use a concise clinical style.",
  });
  // Only the row the reader touched travels back to the plan.
  expect(
    invocations.find((invocation) => invocation.cmd === "update_draft_plan")
      ?.args.edits,
  ).toEqual([
    {
      section_id: "66666666-6666-4666-8666-666666666666",
      scope: "Only the attention measures.",
    },
  ]);
  expect(
    invocations.find((invocation) => invocation.cmd === "start_draft_run")?.args,
  ).toMatchObject({
    runId: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    modelId: "us.anthropic.claude-sonnet-4-20250514-v1:0",
  });
  expect(
    invocations.some((invocation) => invocation.cmd === "generate_full_report"),
  ).toBe(false);
});

test("sections land while the draft runs, and Stop keeps the ones already saved", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();

  // A template with three sections, so the run has something to land into.
  await page
    .getByLabel("Writer template")
    .selectOption({ label: "Sectioned Evaluation Template" });
  await page.getByRole("button", { name: "Apply template" }).click();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Template referral text.",
  );

  // Hold the run at the gate so each section can be observed landing.
  await page.evaluate(() => {
    const holder = window as unknown as {
      __DRAFT_STEP__: number;
      __DRAFT_RESOLVE__: boolean;
    };
    holder.__DRAFT_STEP__ = 0;
    holder.__DRAFT_RESOLVE__ = false;
  });

  await page
    .getByLabel("Full report guidance")
    .fill("Use a concise clinical style.");
  await page.getByRole("button", { name: "Fill whole report" }).click();
  await page
    .getByRole("dialog", { name: "Replace the working draft?" })
    .getByRole("button", { name: "Fill whole report" })
    .click();

  // The plan first. Narrowing one section's scope proves the gate's edits
  // reach the backend before a single section is written.
  await expect(
    page.getByText("Plan ready \u2014 review before drafting"),
  ).toBeVisible();
  await page
    .getByTestId("draft-plan-card")
    .filter({ hasText: "Background" })
    .locator("summary")
    .click();
  await page.getByLabel("Scope for Background").fill("Only the school history.");
  await page.getByRole("button", { name: "Start drafting (3 sections)" }).click();

  const canvas = page.getByTestId("accepted-report-canvas");
  const progress = page
    .getByTestId("draft-run-progress")
    .getByRole("progressbar", { name: "Report sections drafted" });
  const releaseSection = async (step: number) => {
    await page.evaluate((value) => {
      (window as unknown as { __DRAFT_STEP__: number }).__DRAFT_STEP__ = value;
    }, step);
  };

  await releaseSection(1);
  await expect(canvas).toContainText(
    "Drafted Reason for Referral from the client records.",
  );
  await expect(progress).toHaveAttribute("aria-valuetext", "1 of 3 drafted");
  // The command has not returned: the section is on the page because it is
  // durable, not because the run finished.
  await expect(
    page.getByText(/Generated and saved revision/),
  ).toHaveCount(0);

  await releaseSection(2);
  await expect(canvas).toContainText(
    "Drafted Background from the client records.",
  );
  await expect(progress).toHaveAttribute("aria-valuetext", "2 of 3 drafted");
  await expect(canvas).toContainText("Template summary text.");

  // Both the run pane and the canvas strip carry Stop; either one ends it.
  await page
    .getByTestId("draft-run-pane")
    .getByRole("button", { name: "Stop run" })
    .click();

  const banner = page.getByTestId("draft-run-banner");
  await expect(banner).toContainText(
    "Stopped — 2 of 3 sections drafted and saved. Undone sections are unchanged.",
  );
  // Nothing about a stop the reader asked for is an error.
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(canvas).toContainText(
    "Drafted Reason for Referral from the client records.",
  );

  const invocations = await page.evaluate(() =>
    (window as unknown as {
      __REPORT_INVOCATIONS__: Array<{ cmd: string; args: Record<string, unknown> }>;
    }).__REPORT_INVOCATIONS__,
  );
  const started = invocations.find(
    (invocation) => invocation.cmd === "start_draft_run",
  );
  const stop = invocations.find((invocation) => invocation.cmd === "stop_stream");
  expect(stop?.args.streamId).toBe(started?.args.streamId);
  expect(
    invocations.find((invocation) => invocation.cmd === "update_draft_plan")
      ?.args.edits,
  ).toEqual([
    {
      section_id: "a2222222-2222-4222-8222-222222222222",
      scope: "Only the school history.",
    },
  ]);

  // Starting back up asks the plan's question again, this time about a
  // section that already landed.
  await banner.getByRole("button", { name: "Start back up" }).click();
  const referral = page
    .getByTestId("draft-plan-card")
    .filter({ hasText: "Reason for Referral" });
  await referral.locator("summary").click();
  await expect(referral.getByRole("radio", { name: "Keep" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await referral.getByRole("radio", { name: "Rewrite" }).click();
  await page.getByRole("button", { name: "Start back up (2 remaining)" }).click();

  await expect(canvas).toContainText(
    "Drafted Summary from the client records.",
  );
  await expect(page.getByTestId("draft-run-banner")).toHaveCount(0);

  const afterResume = await page.evaluate(() =>
    (window as unknown as {
      __REPORT_INVOCATIONS__: Array<{ cmd: string; args: Record<string, unknown> }>;
    }).__REPORT_INVOCATIONS__,
  );
  const planEdits = afterResume.filter(
    (invocation) => invocation.cmd === "update_draft_plan",
  );
  expect(planEdits[planEdits.length - 1]?.args.edits).toEqual([
    { section_id: "a1111111-1111-4111-8111-111111111111", intent: "rewrite" },
  ]);
  expect(
    afterResume.find((invocation) => invocation.cmd === "resume_draft_run")
      ?.args,
  ).toMatchObject({
    runId: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    updatedInstructions: null,
  });
});

test("Editor History resumes a specific Writing session", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();
  // A whole-report draft is planned against the report's sections, so the
  // session needs a template before it has anything to plan.
  await page.getByRole("button", { name: "Apply template" }).click();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Imported Evaluation Template",
  );
  await page.getByRole("button", { name: "Fill whole report" }).click();
  await page
    .getByRole("dialog", { name: "Replace the working draft?" })
    .getByRole("button", { name: "Fill whole report" })
    .click();
  await page
    .getByRole("button", { name: "Start drafting (1 sections)" })
    .click();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Complete Generated Evaluation",
  );

  await page.locator('[data-tab="record"]').click();
  await page.getByRole("button", { name: "Editor History" }).click();
  await expect(page.getByText("Complete Generated Evaluation")).toBeVisible();
  await page.getByTitle("Resume writing session").click();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Complete Generated Evaluation",
  );

  const invocations = await page.evaluate(() =>
    (window as unknown as {
      __REPORT_INVOCATIONS__: Array<{ cmd: string; args: Record<string, unknown> }>;
    }).__REPORT_INVOCATIONS__,
  );
  const startedReportId = invocations.find(
    (invocation) => invocation.cmd === "start_report_workspace",
  )?.args.reportId;
  expect(
    invocations.find((invocation) => invocation.cmd === "load_report_workspace")
      ?.args,
  ).toMatchObject({ reportId: startedReportId });
});

test("managed writer templates apply directly and export without responsibility nags", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();

  await expect(page.getByLabel("Writer template")).toContainText(
    "Imported Evaluation Template",
  );
  await page.getByRole("button", { name: "Apply template" }).click();
  await expect(page.getByText("Review DOCX template import")).toHaveCount(0);
  await expect(page.getByText(/Template Imported Evaluation Template applied/)).toBeVisible();
  await expect(page.getByLabel("Writer template")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Apply template" })).toHaveCount(0);

  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Imported Evaluation Template",
  );
  await expect(page.getByTestId("report-table")).toContainText("Attention");
  await expect(page.getByText(/carryover review required/)).toHaveCount(0);
  const exportButton = page.getByRole("button", { name: "Export .docx" });
  await expect(exportButton).toBeEnabled();

  // Imported tables and ordinary from-scratch blocks share one structured draft.
  await page.getByRole("button", { name: "Edit" }).click();
  await page
    .getByRole("textbox", {
      name: "Section 1 table 1, row 2, column 2",
    })
    .fill("91");
  await page.getByTestId("inline-report-editor").locator("section").hover();
  await page.getByRole("button", { name: "Add paragraph" }).click();
  await page.getByRole("button", { name: "Save now" }).click();
  await expect(page.getByTestId("accepted-report-canvas")).toContainText("91");
  await expect(exportButton).toBeEnabled();
  await exportButton.click();
  await expect(page.getByText("Word document exported from revision 2.")).toBeVisible();

  const commands = await page.evaluate(() =>
    (window as unknown as { __REPORT_COMMANDS__: string[] }).__REPORT_COMMANDS__,
  );
  expect(commands).toContain("preview_writer_template");
  expect(commands).toContain("apply_report_template");
  expect(commands).toContain("export_report_docx");
  expect(commands).not.toContain("acknowledge_report_template_review");
});

test("Writing previews and restores an old report without deleting later revisions", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();

  await page.getByRole("button", { name: "Apply template" }).click();
  await expect(page.getByText(/Template Imported Evaluation Template applied/)).toBeVisible();
  await expect(page.getByLabel("Writer template")).toHaveCount(0);
  await page.getByRole("button", { name: "Revisions" }).click();

  const historicalReport = page.getByTestId("revision-report-canvas");
  await expect(historicalReport).toContainText("Untitled report");
  await expect(historicalReport.locator("xpath=../..")).toHaveClass(
    /overflow-y-auto/,
  );
  await page.getByRole("button", { name: "Revert to version" }).click();

  await expect(page.getByTestId("accepted-report-canvas")).toContainText(
    "Untitled report",
  );
  await expect(page.getByText("Revision 2 · Saved")).toBeVisible();
  await expect(page.getByText(/Template Imported Evaluation Template applied/)).toBeVisible();

  // The imported revision is still available after the revert created r2.
  await page.getByRole("button", { name: "Revisions" }).click();
  await expect(page.getByTestId("revision-report-canvas")).toContainText(
    "Imported Evaluation Template",
  );
  await expect(
    page.getByRole("option", { name: /Revision 0 · Untitled report/ }),
  ).toBeAttached();

  const invocations = await page.evaluate(() =>
    (window as unknown as {
      __REPORT_INVOCATIONS__: Array<{
        cmd: string;
        args: Record<string, unknown>;
      }>;
    }).__REPORT_INVOCATIONS__,
  );
  expect(
    invocations.find((invocation) => invocation.cmd === "revert_report_revision")
      ?.args,
  ).toMatchObject({ expectedRevision: 1, revision: 0 });
});

test("Writing opens the expanded template manager in Preferences", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();

  await page.getByRole("button", { name: "Manage templates" }).click();
  await expect(page.getByRole("heading", { name: "Preferences" })).toBeVisible();
  const manager = page.getByTestId("writer-template-manager");
  await expect(manager).toHaveAttribute("open", "");
  await expect(manager).toContainText("Imported Evaluation Template");

  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByText("Jane Doe")).toBeVisible();
});

test("opening Writing from the record starts a fresh session", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();
  await page.getByRole("tab", { name: "Write with Claude" }).click();
  await page.getByLabel("Writing instruction").fill("Keep this draft");

  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByText("Jane Doe")).toBeVisible();

  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="writing"]').click();
  await page.getByRole("tab", { name: "Write with Claude" }).click();
  await expect(page.getByLabel("Writing instruction")).toHaveValue("");

  const starts = await page.evaluate(() =>
    (window as unknown as {
      __REPORT_INVOCATIONS__: Array<{ cmd: string; args: Record<string, unknown> }>;
    }).__REPORT_INVOCATIONS__.filter(
      (invocation) => invocation.cmd === "start_report_workspace",
    ),
  );
  const uniqueReportIds = new Set(starts.map((start) => start.args.reportId));
  expect(uniqueReportIds.size).toBe(2);
});

test("a new Chat can be named directly before its first message", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();
  await page.locator('[data-tab="chat"]').click();

  await page.getByRole("button", { name: "Rename chat" }).click();
  await page.getByLabel("chat name").fill("Intake synthesis");
  await page.getByRole("button", { name: "Save" }).click();
  await page.getByPlaceholder("Type a message...").fill("Start the summary");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Unchanged Chat response")).toBeVisible();

  const invocations = await page.evaluate(
    () => (window as unknown as { __CHAT_COMMANDS__: Array<{ cmd: string; args: Record<string, unknown> }> })
      .__CHAT_COMMANDS__,
  );
  expect(invocations.find((invocation) => invocation.cmd === "chat_message")?.args)
    .toMatchObject({ chatName: "Intake synthesis", chatId: null });
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
  await expect(page.getByText("$0.021")).toHaveCount(0);

  await page.getByRole("tab", { name: "Costs and cache" }).click();
  const usagePanel = page.getByTestId("session-usage-panel");
  await expect(usagePanel).toContainText("4,243 tok");
  await expect(usagePanel).toContainText("5,000 tok");
  await expect(usagePanel).toContainText("write fees");
  await page.getByLabel("Show turn costs").check();
  await page.getByRole("tab", { name: "Conversation" }).click();
  await expect(page.getByText("$0.021", { exact: true })).toBeVisible();
  await expect(page.getByText("4,243 tok cached")).toBeVisible();
  await expect(page.getByText("3 tok new")).toBeVisible();

  const state = await page.evaluate(() => ({
    chat: (window as unknown as { __CHAT_COMMANDS__: unknown[] }).__CHAT_COMMANDS__,
    report: (window as unknown as { __REPORT_COMMANDS__: string[] })
      .__REPORT_COMMANDS__,
  }));
  expect(state.chat).toHaveLength(2);
  expect(state.report).toEqual([]);
});
