import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import VersionHistoryModal from "./VersionHistoryModal";
import type { FileVersion } from "../lib/tauri";
import type { VersionSource } from "../lib/versions";

/**
 * One component now serves two call sites — the record page, which compares
 * versions, and the prompt editor, which does not. These tests pin down both
 * shapes, since a change made for one of them is now a change to the other.
 */

function version(id: string, overrides: Partial<FileVersion> = {}): FileVersion {
  return {
    version_id: id,
    last_modified: "2026-03-01T10:00:00Z",
    size: 2048,
    is_latest: false,
    ...overrides,
  };
}

const NEWER = version("v-newer-0000000000", { is_latest: true });
const OLDER = version("v-older-0000000000", {
  last_modified: "2026-02-01T10:00:00Z",
});

function source(overrides: Partial<VersionSource> = {}): VersionSource {
  return {
    list: () => Promise.resolve([NEWER, OLDER]),
    getText: (id) => Promise.resolve(`body of ${id}`),
    restore: () => Promise.resolve(),
    ...overrides,
  };
}

function open(props: Partial<Parameters<typeof VersionHistoryModal>[0]> = {}) {
  const onClose = vi.fn();
  const onRestored = vi.fn();
  const onError = vi.fn();
  const view = render(
    <VersionHistoryModal
      title="Version History: notes.txt"
      source={source()}
      onClose={onClose}
      onRestored={onRestored}
      onError={onError}
      {...props}
    />,
  );
  return { view, onClose, onRestored, onError };
}

const button = (name: string | RegExp) =>
  screen.getByRole<HTMLButtonElement>("button", { name });

/** Text of the diff row carrying the given background class. */
function diffRow(className: string): string {
  const row = [...document.querySelectorAll("div")].find((d) =>
    d.className.includes(className),
  );
  return row?.textContent ?? "";
}

describe("VersionHistoryModal", () => {
  it("loads the version list on open", async () => {
    const list = vi.fn(() => Promise.resolve([NEWER, OLDER]));
    open({ source: source({ list }) });

    expect(screen.getByText("Loading versions...")).toBeTruthy();
    await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());
    expect(list).toHaveBeenCalledTimes(1);
  });

  it("says so when there is no history", async () => {
    open({ source: source({ list: () => Promise.resolve([]) }) });
    await waitFor(() =>
      expect(screen.getByText("No version history found.")).toBeTruthy(),
    );
  });

  it("reports a failed list on the caller's banner", async () => {
    const { onError } = open({
      source: source({ list: () => Promise.reject(new Error("denied")) }),
    });
    await waitFor(() => expect(onError).toHaveBeenCalledTimes(1));
    expect(String(onError.mock.calls[0][0])).toContain("denied");
  });

  it("does not fetch after the modal is closed mid-flight", async () => {
    // The unresolved list must not write into an unmounted component.
    let settle: (versions: FileVersion[]) => void = () => {};
    const { view, onError } = open({
      source: source({
        list: () => new Promise((resolve) => (settle = resolve)),
      }),
    });

    view.unmount();
    await new Promise((r) => setTimeout(r, 0));
    settle([NEWER]);
    await new Promise((r) => setTimeout(r, 0));

    expect(onError).not.toHaveBeenCalled();
  });

  it("toggles an inline preview of one version", async () => {
    open();
    await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

    await userEvent.click(screen.getAllByRole("button", { name: "View" })[0]);
    await waitFor(() =>
      expect(screen.getByText(`body of ${NEWER.version_id}`)).toBeTruthy(),
    );

    await userEvent.click(button("Hide"));
    expect(screen.queryByText(`body of ${NEWER.version_id}`)).toBeNull();
  });

  it("shows a failed preview inline, not on the banner behind the dialog", async () => {
    const { onError } = open({
      source: source({ getText: () => Promise.reject(new Error("gone")) }),
    });
    await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

    await userEvent.click(screen.getAllByRole("button", { name: "View" })[0]);

    await waitFor(() => expect(screen.getByText(/Error:.*gone/)).toBeTruthy());
    expect(onError).not.toHaveBeenCalled();
  });

  it("restores a version, then closes and tells the caller to reload", async () => {
    const restore = vi.fn(() => Promise.resolve());
    const { onClose, onRestored } = open({ source: source({ restore }) });
    await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

    await userEvent.click(button("Restore"));

    await waitFor(() => expect(onRestored).toHaveBeenCalledTimes(1));
    expect(restore).toHaveBeenCalledWith(OLDER.version_id);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("offers no Restore on the current version", async () => {
    open();
    await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());
    expect(screen.getAllByRole("button", { name: "Restore" })).toHaveLength(1);
  });

  describe("without comparison (the prompt editor's shape)", () => {
    it("shows no checkboxes, no Compare and no footer Close", async () => {
      open();
      await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

      expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
      expect(screen.queryByRole("button", { name: "Compare" })).toBeNull();
      // Only the heading's X — no footer Close button.
      expect(screen.queryAllByRole("button", { name: "Close" })).toHaveLength(
        1,
      );
    });
  });

  describe("with comparison (the record page's shape)", () => {
    it("counts the selection and enables Compare at two", async () => {
      open({ enableCompare: true });
      await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

      expect(
        screen.getByText("Select 2 versions to compare (0/2)"),
      ).toBeTruthy();
      expect(button("Compare").disabled).toBe(true);

      const boxes = screen.getAllByRole("checkbox");
      await userEvent.click(boxes[0]);
      expect(button("Compare").disabled).toBe(true);

      await userEvent.click(boxes[1]);
      expect(screen.getByText("2 versions selected")).toBeTruthy();
      expect(button("Compare").disabled).toBe(false);
    });

    it("diffs the two selected versions oldest-first", async () => {
      const getText = vi.fn((id: string) =>
        Promise.resolve(id === NEWER.version_id ? "second\n" : "first\n"),
      );
      open({ enableCompare: true, source: source({ getText }) });
      await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

      const boxes = screen.getAllByRole("checkbox");
      await userEvent.click(boxes[0]);
      await userEvent.click(boxes[1]);
      await userEvent.click(button("Compare"));

      await waitFor(() => expect(screen.getByText("Diff")).toBeTruthy());
      // Older is the left-hand side: "first" is removed, "second" is added.
      expect(diffRow("bg-red-50")).toContain("first");
      expect(diffRow("bg-green-50")).toContain("second");
    });

    it("replaces the oldest selection past two", async () => {
      const third = version("v-third-0000000000");
      open({
        enableCompare: true,
        source: source({ list: () => Promise.resolve([NEWER, OLDER, third]) }),
      });
      await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

      const boxes = screen.getAllByRole<HTMLInputElement>("checkbox");
      await userEvent.click(boxes[0]);
      await userEvent.click(boxes[1]);
      await userEvent.click(boxes[2]);

      expect(boxes[0].checked).toBe(false);
      expect(boxes[1].checked).toBe(true);
      expect(boxes[2].checked).toBe(true);
      expect(screen.getByText("2 versions selected")).toBeTruthy();
    });

    it("drops a stale diff when the selection changes", async () => {
      open({ enableCompare: true });
      await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

      const boxes = screen.getAllByRole("checkbox");
      await userEvent.click(boxes[0]);
      await userEvent.click(boxes[1]);
      await userEvent.click(button("Compare"));
      await waitFor(() => expect(screen.getByText("Diff")).toBeTruthy());

      await userEvent.click(boxes[1]);
      expect(screen.queryByText("Diff")).toBeNull();
    });

    it("closes from the footer button", async () => {
      const { onClose } = open({ enableCompare: true, showFooterClose: true });
      await waitFor(() => expect(screen.getByText("Current")).toBeTruthy());

      const closes = screen.getAllByRole("button", { name: "Close" });
      expect(closes).toHaveLength(2); // heading X plus the footer button
      await userEvent.click(closes[1]);

      expect(onClose).toHaveBeenCalledTimes(1);
    });
  });
});
