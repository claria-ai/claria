/** The quiet "no messages yet" prompt shared by chat and writer surfaces. */
export default function ChatEmptyState({
  title,
  subtitle,
}: {
  title: string;
  subtitle?: string;
}) {
  return (
    <div className="text-center text-gray-400 text-sm mt-8">
      <p className="mb-1">{title}</p>
      {subtitle && <p className="text-xs">{subtitle}</p>}
    </div>
  );
}
