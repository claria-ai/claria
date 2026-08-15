import { describe, expect, it } from "vitest";

import { PIN_SLACK_PX, isPinnedToBottom } from "./scroll";

describe("isPinnedToBottom", () => {
  it("follows a container sitting exactly at its bottom", () => {
    expect(
      isPinnedToBottom({ scrollHeight: 1000, scrollTop: 600, clientHeight: 400 })
    ).toBe(true);
  });

  it("still follows within the slack", () => {
    expect(
      isPinnedToBottom({
        scrollHeight: 1000,
        scrollTop: 600 - PIN_SLACK_PX,
        clientHeight: 400,
      })
    ).toBe(true);
  });

  it("lets go once the reader has scrolled up to read", () => {
    expect(
      isPinnedToBottom({ scrollHeight: 1000, scrollTop: 200, clientHeight: 400 })
    ).toBe(false);
  });

  /// A container shorter than its viewport has nothing to scroll, so it is
  /// always at the bottom — an empty conversation must not read as detached.
  it("treats an unscrollable container as pinned", () => {
    expect(
      isPinnedToBottom({ scrollHeight: 300, scrollTop: 0, clientHeight: 400 })
    ).toBe(true);
  });
});
