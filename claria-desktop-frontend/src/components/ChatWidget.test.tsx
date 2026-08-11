// ChatWidget keeps streaming behavior and isolates optional usage details in
// the shared costs-and-cache tab.

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ChatModelsContext, type ChatModelsState } from "../lib/chatModels";
import type { ChatMessage, TurnUsage } from "../lib/tauri";
import ChatWidget, { type SendResult } from "./ChatWidget";

vi.mock("../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/tauri")>();
  return {
    ...actual,
    lookupModelPricing: vi.fn(async () => null),
    acceptModelAgreement: vi.fn(async () => undefined),
  };
});

const MODELS_STATE: ChatModelsState = {
  models: [{ model_id: "us.anthropic.claude-test", name: "Claude Test" }],
  loading: false,
  error: null,
  preferredModelId: "us.anthropic.claude-test",
  retry: () => {},
  setPreferredModelId: () => {},
};

function renderWidget(
  onSend: (
    modelId: string,
    messages: unknown,
    onDelta: (text: string) => void
  ) => Promise<SendResult>,
  initial?: {
    messages: ChatMessage[];
    usage: Array<TurnUsage | null>;
    timestamps?: Array<string | null>;
  }
) {
  return render(
    <ChatModelsContext.Provider value={MODELS_STATE}>
      <ChatWidget
        onSend={onSend}
        initialMessages={initial?.messages}
        initialUsageByIndex={initial?.usage}
        initialTimestampsByIndex={initial?.timestamps}
      />
    </ChatModelsContext.Provider>
  );
}

async function sendMessage(text: string) {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText("Chat message"), text);
  await user.click(screen.getByRole("button", { name: "Send" }));
}

describe("ChatWidget", () => {
  it("renders deltas incrementally, then the final assistant message", async () => {
    let resolveTurn!: (result: SendResult) => void;
    let emitDelta!: (text: string) => void;
    const onSend = vi.fn(
      (_modelId: string, _messages: unknown, onDelta: (text: string) => void) => {
        emitDelta = onDelta;
        return new Promise<SendResult>((resolve) => {
          resolveTurn = resolve;
        });
      }
    );

    renderWidget(onSend);
    await sendMessage("Hello");

    // First delta replaces the spinner with a streaming assistant bubble.
    emitDelta("Streaming ");
    await screen.findByText(/Streaming/);
    expect(screen.queryByText("Thinking...")).toBeNull();

    // Later deltas grow the same bubble.
    emitDelta("reply.");
    await screen.findByText(/Streaming reply\./);

    // The awaited response is the durable message; streaming state clears.
    resolveTurn({ content: "Streaming reply.", usage: null });
    await waitFor(() =>
      expect(screen.getAllByText(/Streaming reply\./).length).toBe(1)
    );
  });

  it("falls back to the awaited response when no deltas arrive", async () => {
    let resolveTurn!: (result: SendResult) => void;
    const onSend = vi.fn(
      () =>
        new Promise<SendResult>((resolve) => {
          resolveTurn = resolve;
        })
    );

    renderWidget(onSend);
    await sendMessage("Hello");

    // No deltas: the widget shows the non-streaming pending state.
    await screen.findByText("Thinking...");

    resolveTurn({ content: "Unary reply.", usage: null });
    await screen.findByText(/Unary reply\./);
    expect(screen.queryByText("Thinking...")).toBeNull();
  });

  it("keeps cost and cache details in their tab until turn costs are enabled", async () => {
    const usage: TurnUsage = {
      model_id: "us.anthropic.claude-test",
      input_tokens: 3,
      output_tokens: 60,
      cache_read_input_tokens: 4_243,
      cache_write_input_tokens: 5_000,
      cache_ttl: "five_minutes",
      cost_usd: 0.021,
      pricing_version: 1,
    };
    renderWidget(vi.fn(), {
      messages: [
        { role: "user", content: "Question" },
        { role: "assistant", content: "Answer" },
      ],
      usage: [null, usage],
      timestamps: [
        "2026-08-11T12:00:00Z",
        "2026-08-11T12:00:01Z",
      ],
    });

    expect(screen.getByText("Answer")).toBeDefined();
    expect(screen.queryByText("$0.021")).toBeNull();
    expect(screen.queryByText(/Session:/)).toBeNull();

    const usageTab = screen.getByRole("tab", { name: "Costs and cache" });
    expect(usageTab.className).toContain("w-14");
    await userEvent.click(usageTab);
    expect(screen.getByText("Session cost & cache")).toBeDefined();
    expect(screen.getByText("4,243 tok")).toBeDefined();
    expect(screen.getByText("5,000 tok")).toBeDefined();

    await userEvent.click(screen.getByLabelText("Show turn costs"));
    await userEvent.click(screen.getByRole("tab", { name: "Conversation" }));
    expect(screen.getByText("$0.021")).toBeDefined();
    expect(screen.getByText("4,243 tok cached")).toBeDefined();
    expect(screen.getByText("3 tok new")).toBeDefined();
  });
});
