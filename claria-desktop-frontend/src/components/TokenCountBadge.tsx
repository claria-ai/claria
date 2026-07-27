import Spinner from "./Spinner";

/**
 * The little circular badge next to the "Context" label in a chat toolbar.
 *
 * Three states: a spinner while the count is in flight, a red `!` carrying
 * the failure in a hover tooltip, and a grey `?` carrying the approximate
 * token count. Renders nothing when there is no count and no error.
 */
export default function TokenCountBadge({
  counting,
  tokens,
  error,
}: {
  counting: boolean;
  tokens: number | null;
  error?: string | null;
}) {
  const label =
    tokens != null
      ? tokens >= 1000
        ? `~${(tokens / 1000).toFixed(1)}k tokens`
        : `~${tokens} tokens`
      : null;

  if (counting) {
    return (
      <span className="shrink-0 inline-flex items-center justify-center w-5 h-5 text-gray-400">
        <Spinner />
      </span>
    );
  }

  if (error) {
    return (
      <span
        className="shrink-0 inline-flex items-center justify-center w-5 h-5 rounded-full bg-red-50 border border-red-200 text-red-400 text-[10px] font-bold cursor-default group relative"
        title={error}
      >
        !
        <span className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 text-[11px] font-normal text-white bg-red-700 rounded max-w-xs whitespace-pre-wrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
          {error}
        </span>
      </span>
    );
  }

  if (label == null) return null;

  return (
    <span
      className="shrink-0 inline-flex items-center justify-center w-5 h-5 rounded-full bg-gray-100 border border-gray-200 text-gray-400 text-[10px] font-bold cursor-default group relative"
      title={label}
    >
      ?
      <span className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 text-[11px] font-normal text-white bg-gray-800 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
        {label}
      </span>
    </span>
  );
}
