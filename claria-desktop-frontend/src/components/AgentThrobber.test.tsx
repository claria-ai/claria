import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import AgentThrobber from "./AgentThrobber";

describe("AgentThrobber", () => {
  it("announces the current agent activity and optional detail", () => {
    render(
      <AgentThrobber
        label="Reading client context"
        detail="teacher-observation.txt"
      />
    );

    const status = screen.getByRole("status");
    expect(status.textContent).toContain("Reading client context");
    expect(status.textContent).toContain("teacher-observation.txt");
    expect(status.querySelectorAll("[aria-hidden=true] span")).toHaveLength(3);
  });
});
