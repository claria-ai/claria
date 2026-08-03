import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ClientWorkspaceTabs, {
  type ClientWorkspaceTab,
} from "./ClientWorkspaceTabs";
import { setDistractionModeEnabled } from "../lib/distractionMode";

function Harness({ onBack = vi.fn() }: { onBack?: () => void }) {
  const [active, setActive] = useState<ClientWorkspaceTab>("record");
  return (
    <ClientWorkspaceTabs
      clientName="Ada Lovelace"
      activeTab={active}
      onSelect={(tab) => {
        setActive(tab);
        return true;
      }}
      onBack={onBack}
    >
      <p>{active} panel</p>
    </ClientWorkspaceTabs>
  );
}

beforeEach(() => {
  setDistractionModeEnabled(false);
});

describe("client workspace tabs", () => {
  it("keeps Record and Chat identifiers and adds an opt-in Writing tab", () => {
    render(<Harness />);
    expect(document.querySelector('[data-tab="record"]')).not.toBeNull();
    expect(document.querySelector('[data-tab="chat"]')).not.toBeNull();
    expect(document.querySelector('[data-tab="writing"]')).not.toBeNull();
    expect(
      screen.getByRole("tab", { name: "Record" }).getAttribute("aria-selected")
    ).toBe("true");
    expect(screen.getByText("record panel")).toBeDefined();
  });

  it("selects Chat and Writing without changing the existing Chat label", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByRole("tab", { name: "Chat" }));
    expect(screen.getByText("chat panel")).toBeDefined();
    expect(
      screen.getByRole("tab", { name: "Chat" }).getAttribute("aria-selected")
    ).toBe("true");

    await userEvent.click(screen.getByRole("tab", { name: "Writing" }));
    expect(screen.getByText("writing panel")).toBeDefined();
    expect(
      screen.getByRole("tab", { name: "Writing" }).getAttribute("aria-selected")
    ).toBe("true");
  });

  it("supports roving focus with arrows, Home, and End", async () => {
    render(<Harness />);
    const record = screen.getByRole("tab", { name: "Record" });
    record.focus();

    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "Chat" })
    );
    expect(screen.getByText("chat panel")).toBeDefined();

    await userEvent.keyboard("{End}");
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "Writing" })
    );
    expect(screen.getByText("writing panel")).toBeDefined();

    await userEvent.keyboard("{Home}");
    expect(document.activeElement).toBe(record);
    expect(screen.getByText("record panel")).toBeDefined();

    await userEvent.keyboard("{ArrowLeft}");
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "Writing" })
    );
  });

  it("places the opt-in sock control quietly beside the client name", async () => {
    setDistractionModeEnabled(true);
    render(<Harness />);

    const button = screen.getByRole("button", {
      name: "Drop a sock for Lucia",
    });
    const heading = screen.getByRole("heading", { name: "Ada Lovelace" });
    expect(button.nextElementSibling).toBe(heading);
    expect(button.className).not.toContain("border");

    await userEvent.click(button);
    expect(screen.getByTestId("sock-drop")).toBeDefined();
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("routes the back control through the parent guard", async () => {
    const onBack = vi.fn();
    render(<Harness onBack={onBack} />);
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
