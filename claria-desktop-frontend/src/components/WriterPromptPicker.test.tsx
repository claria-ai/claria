import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import WriterPromptPicker from "./WriterPromptPicker";
import type { WriterPrompt } from "../lib/tauri";

const prompts: WriterPrompt[] = [
  {
    schema_version: 1,
    id: "11111111-1111-4111-8111-111111111111",
    name: "Phase 1 — history",
    body: "Fill in Reason for Referral and Background; skip everything else.",
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
  },
  {
    schema_version: 1,
    id: "22222222-2222-4222-8222-222222222222",
    name: "Phase 2 — summary",
    body: "Draft the summary backing my diagnosis of $DIAGNOSIS.",
    created_at: "2026-08-02T00:00:00Z",
    updated_at: "2026-08-02T00:00:00Z",
  },
];

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("WriterPromptPicker", () => {
  it("renders nothing while the library is empty", () => {
    const { container } = render(
      <WriterPromptPicker
        prompts={[]}
        currentValue=""
        disabled={false}
        onPick={() => {}}
      />
    );
    expect(container.innerHTML).toBe("");
  });

  it("prefills the picked prompt body without confirmation when the box is empty", async () => {
    const onPick = vi.fn();
    render(
      <WriterPromptPicker
        prompts={prompts}
        currentValue=""
        disabled={false}
        onPick={onPick}
      />
    );
    await userEvent.selectOptions(
      screen.getByLabelText("Insert saved prompt"),
      prompts[1].id
    );
    expect(onPick).toHaveBeenCalledWith(prompts[1].body);
  });

  it("asks before replacing typed text and respects a refusal", async () => {
    const onPick = vi.fn();
    const confirm = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirm);
    render(
      <WriterPromptPicker
        prompts={prompts}
        currentValue="Half-typed thought"
        disabled={false}
        onPick={onPick}
      />
    );
    await userEvent.selectOptions(
      screen.getByLabelText("Insert saved prompt"),
      prompts[0].id
    );
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(onPick).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await userEvent.selectOptions(
      screen.getByLabelText("Insert saved prompt"),
      prompts[0].id
    );
    expect(onPick).toHaveBeenCalledWith(prompts[0].body);
  });
});
