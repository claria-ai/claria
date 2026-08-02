import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ChatHistoryFolder from "./ChatHistoryFolder";

const mocks = vi.hoisted(() => ({
  loadChatHistory: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  loadChatHistory: mocks.loadChatHistory,
}));

const files = [
  {
    filename: "chat-history/1234567890.json",
    size: 512,
    uploaded_at: "2026-08-01T00:00:00Z",
  },
];

beforeEach(() => {
  vi.clearAllMocks();
});

describe("ChatHistoryFolder", () => {
  it("loads the selected conversation and hands the detail to the workspace", async () => {
    const detail = {
      chat_id: "1234567890",
      model_id: "model-1",
      messages: [],
      created_at: "2026-08-01T00:00:00Z",
    };
    mocks.loadChatHistory.mockResolvedValue(detail);
    const onResume = vi.fn();
    render(
      <ChatHistoryFolder
        clientId="client-1"
        files={files}
        onResume={onResume}
        onDelete={vi.fn()}
        onError={vi.fn()}
      />
    );

    await userEvent.click(screen.getByText("Chat History"));
    await userEvent.click(
      screen.getByRole("button", { name: "Resume conversation" })
    );

    expect(mocks.loadChatHistory).toHaveBeenCalledWith(
      "client-1",
      "1234567890"
    );
    expect(onResume).toHaveBeenCalledWith(detail);
  });

  it("surfaces a failed resume through the record error channel", async () => {
    mocks.loadChatHistory.mockRejectedValue(new Error("history unavailable"));
    const onError = vi.fn();
    render(
      <ChatHistoryFolder
        clientId="client-1"
        files={files}
        onResume={vi.fn()}
        onDelete={vi.fn()}
        onError={onError}
      />
    );

    await userEvent.click(screen.getByText("Chat History"));
    await userEvent.click(
      screen.getByRole("button", { name: "Resume conversation" })
    );

    expect(onError).toHaveBeenCalledWith("Error: history unavailable");
    expect(
      (screen.getByRole("button", {
        name: "Resume conversation",
      }) as HTMLButtonElement).disabled
    ).toBe(false);
  });
});
