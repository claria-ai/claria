// ChatWidget streaming: deltas render incrementally through the assistant
// bubble, and a sender that never streams falls back to the awaited result.

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ChatModelsContext, type ChatModelsState } from "../lib/chatModels";
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
  ) => Promise<SendResult>
) {
  return render(
    <ChatModelsContext.Provider value={MODELS_STATE}>
      <ChatWidget onSend={onSend} />
    </ChatModelsContext.Provider>
  );
}

async function sendMessage(text: string) {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText("Chat message"), text);
  await user.click(screen.getByRole("button", { name: "Send" }));
}

describe("ChatWidget streaming", () => {
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
});
