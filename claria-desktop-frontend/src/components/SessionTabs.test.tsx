import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import SessionTabs, { UsageTabIcon } from "./SessionTabs";

function Example() {
  const [active, setActive] = useState<"first" | "second" | "usage">("first");
  return (
    <SessionTabs
      idPrefix="example"
      label="Example session"
      active={active}
      onSelect={setActive}
      tabs={[
        { id: "first", label: "First" },
        { id: "second", label: "Second" },
        {
          id: "usage",
          label: "Costs and cache",
          compact: true,
          icon: <UsageTabIcon />,
        },
      ]}
    />
  );
}

describe("SessionTabs", () => {
  it("shares accessible arrow-key tabs with a compact usage affordance", async () => {
    render(<Example />);
    const first = screen.getByRole("tab", { name: "First" });
    const second = screen.getByRole("tab", { name: "Second" });
    const usage = screen.getByRole("tab", { name: "Costs and cache" });

    first.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(second.getAttribute("aria-selected")).toBe("true");
    await userEvent.keyboard("{End}");
    expect(usage.getAttribute("aria-selected")).toBe("true");
    expect(usage.className).toContain("w-14");
    expect(usage.getAttribute("aria-controls")).toBe("example-panel-usage");
  });
});
