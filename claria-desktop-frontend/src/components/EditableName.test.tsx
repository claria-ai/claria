import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import EditableName from "./EditableName";

describe("EditableName", () => {
  it("edits, trims, and saves a session name", async () => {
    const save = vi.fn().mockResolvedValue(undefined);
    render(<EditableName value="Chat (1)" label="chat" onSave={save} />);

    await userEvent.click(screen.getByRole("button", { name: "Rename chat" }));
    const input = screen.getByRole("textbox", { name: "chat name" });
    await userEvent.clear(input);
    await userEvent.type(input, "  Intake notes  {Enter}");

    expect(save).toHaveBeenCalledWith("Intake notes");
  });

  it("keeps the editor open and shows save failures", async () => {
    const save = vi.fn().mockRejectedValue(new Error("name conflict"));
    render(<EditableName value="Writer Session (1)" label="writer session" onSave={save} />);

    await userEvent.click(
      screen.getByRole("button", { name: "Rename writer session" })
    );
    const input = screen.getByRole("textbox", { name: "writer session name" });
    await userEvent.clear(input);
    await userEvent.type(input, "Assessment");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Error: name conflict")).toBeTruthy();
    expect(input).toBeTruthy();
  });
});
