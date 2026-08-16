/** How close to the bottom still counts as following the newest content. */
export const PIN_SLACK_PX = 48;

/**
 * Whether a scroll container is close enough to its bottom that new content
 * should keep scrolling into view.
 *
 * The slack matters: a reader who is a few pixels off the bottom — because
 * the last line half-wrapped, or because a trackpad overshot — is still
 * following along, and yanking them back would be as annoying as not
 * following at all. A reader who has scrolled up to re-read something is
 * not, and must keep their place.
 */
export function isPinnedToBottom(
  box: { scrollHeight: number; scrollTop: number; clientHeight: number },
  slack: number = PIN_SLACK_PX
): boolean {
  return box.scrollHeight - box.scrollTop - box.clientHeight <= slack;
}

/** How long a deliberate scroll holds off automatic scrolling. */
export const USER_SCROLL_GRACE_MS = 4000;

/**
 * Whether content is allowed to scroll itself into view.
 *
 * The same restraint as {@link isPinnedToBottom}, for content that arrives
 * somewhere other than the bottom: a reader who has just moved the document
 * themselves is reading, and yanking the viewport to a section the writer
 * happened to start would take the page away from them.
 */
export function mayAutoScroll(
  lastUserScrollAt: number | null,
  now: number,
  grace: number = USER_SCROLL_GRACE_MS
): boolean {
  return lastUserScrollAt === null || now - lastUserScrollAt >= grace;
}
