import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import ClientWorkspaceTabs, {
  type ClientWorkspaceView,
} from "./ClientWorkspaceTabs";

function Harness({ onBack = vi.fn() }: { onBack?: () => void }) {
  const [active, setActive] = useState<ClientWorkspaceView>("record");
  return (
    <ClientWorkspaceTabs
      clientName="Ada Lovelace"
      activeView={active}
      onSelect={(tab) => {
        setActive(tab);
        return true;
      }}
      onSettings={() => {
        setActive("settings");
        return true;
      }}
      onBack={onBack}
    >
      <p>{active} panel</p>
    </ClientWorkspaceTabs>
  );
}

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

  it("opens record settings from the gear and returns through a tab", async () => {
    render(<Harness />);
    const settings = screen.getByRole("button", { name: "Record settings" });
    await userEvent.click(settings);

    expect(settings.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("settings panel")).toBeDefined();
    const record = screen.getByRole("tab", { name: "Record" });
    expect(record.getAttribute("aria-selected")).toBe("false");
    expect(record.tabIndex).toBe(0);

    await userEvent.click(screen.getByRole("tab", { name: "Record" }));
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

  it("routes the back control through the parent guard", async () => {
    const onBack = vi.fn();
    render(<Harness onBack={onBack} />);
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
