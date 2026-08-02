/**
 * Inline SVG icons used across the app.
 *
 * These are the handful of Heroicons-style outline paths that were pasted
 * into pages by hand. Keeping them here means one definition per glyph and
 * no icon-library dependency.
 *
 * Every icon takes a `className` that fully replaces the default size —
 * there is no class merging, so pass the complete sizing/colour classes you
 * want (e.g. `className="w-3 h-3"`).
 */

type IconProps = {
  className?: string;
};

/** Outline X. Closes modals, clears the search field, removes a context pill. */
export function CloseIcon({ className = "w-5 h-5" }: IconProps) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M6 18L18 6M6 6l12 12"
      />
    </svg>
  );
}

/** Left-pointing chevron for page back buttons. */
export function BackIcon({ className = "w-5 h-5" }: IconProps) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M15 19l-7-7 7-7"
      />
    </svg>
  );
}

/** Trash can for destructive row actions. */
export function TrashIcon({ className = "w-4 h-4" }: IconProps) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
      />
    </svg>
  );
}

/** Magnifying glass. */
export function SearchIcon({ className = "w-3.5 h-3.5" }: IconProps) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
      />
    </svg>
  );
}

/** Play triangle in a circle. Resumes a saved conversation. */
export function PlayIcon({ className = "w-4 h-4" }: IconProps) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z"
      />
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
      />
    </svg>
  );
}

/** Folder. Groups the chat-history files on the record page. */
export function FolderIcon({ className = "w-4 h-4" }: IconProps) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
      />
    </svg>
  );
}

/** Cog for record settings and other configuration controls. */
export function GearIcon({ className = "w-5 h-5" }: IconProps) {
  return (
    <svg
      className={className}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M9.6 3.9l.2 1.3c.1.5.4.8.8 1l.4.2c.4.2.9.3 1.3.1l1.2-.5c.5-.2 1.1 0 1.4.5l1.3 2.2c.3.5.2 1.1-.3 1.5l-1 .8c-.4.3-.5.8-.5 1.2v.5c0 .5.2.9.5 1.2l1 .8c.4.4.6 1 .3 1.5l-1.3 2.2c-.3.5-.9.7-1.4.5l-1.2-.5c-.4-.2-.9-.1-1.3.1l-.4.2c-.4.2-.7.6-.8 1l-.2 1.3c-.1.5-.6.9-1.1.9H8c-.6 0-1-.4-1.1-.9l-.2-1.3c-.1-.5-.4-.8-.8-1l-.4-.2c-.4-.2-.9-.3-1.3-.1l-1.2.5c-.5.2-1.1 0-1.4-.5L.3 16.5c-.3-.5-.2-1.1.3-1.5l1-.8c.4-.3.5-.8.5-1.2v-.5c0-.5-.2-.9-.5-1.2l-1-.8C.2 10.1 0 9.5.3 9l1.3-2.2c.3-.5.9-.7 1.4-.5l1.2.5c.4.2.9.1 1.3-.1l.4-.2c.4-.2.7-.6.8-1l.2-1.3c.1-.5.6-.9 1.1-.9h1.6z"
        transform="translate(2 0)"
      />
      <circle cx="12" cy="12" r="3" strokeWidth={2} />
    </svg>
  );
}

/**
 * The back chevron every page header re-implemented by hand, including the
 * accessible name those hand-rolled copies were missing.
 */
export function BackButton({
  onClick,
  label = "Back",
}: {
  onClick: () => void;
  label?: string;
}) {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      className="text-gray-500 hover:text-gray-700 transition-colors"
    >
      <BackIcon />
    </button>
  );
}
