import manifest from "../assets/lucia/manifest.json";
import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ClientWorkspaceTabs, {
  type ClientWorkspaceView,
} from "./ClientWorkspaceTabs";
import { loadLuciaArtwork } from "../lib/luciaArtwork";
import { setDistractionModeEnabled } from "../lib/distractionMode";

vi.mock("../lib/luciaArtwork", async (original) => ({
  ...await original<typeof import("../lib/luciaArtwork")>(),
  loadLuciaArtwork: vi.fn(),
}));

function Harness({ onBack = vi.fn() }: { onBack?: () => void }) {
  const [active, setActive] = useState<ClientWorkspaceView>("record");
  return (
    <ClientWorkspaceTabs
      clientId="fixture-client"
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

beforeEach(() => {
  setDistractionModeEnabled(false);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  vi.mocked(loadLuciaArtwork).mockResolvedValue(new Map(Object.keys(manifest.assets).map((path) => [path, new Image()])));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
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

  it("keeps Lucia and her controls together in the rail, leaving the header clean", async () => {
    setDistractionModeEnabled(true);
    render(<Harness />);

    const button = await screen.findByRole("button", {
      name: "Drop a sock for Lucia",
    });
    const rail = screen.getByTestId("lucia-rail");
    const heading = screen.getByRole("heading", { name: "Ada Lovelace" });
    expect(rail.contains(button)).toBe(true);
    expect(heading.parentElement?.textContent).not.toContain("Lucia");

    await userEvent.click(button);
    expect(screen.getByTestId("lucia-scene").dataset.behavior).toBe("chase");
    expect((button as HTMLButtonElement).disabled).toBe(true);

    await userEvent.click(screen.getByRole("button", { name: "Hide Lucia" }));
    expect(screen.queryByTestId("lucia-rail")).toBeNull();
  });

  it("routes the back control through the parent guard", async () => {
    const onBack = vi.fn();
    render(<Harness onBack={onBack} />);
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
