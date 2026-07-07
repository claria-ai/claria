export default function MoreToggle({
  active,
  onClick,
  title,
}: {
  active: boolean;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`p-2 rounded-lg border transition-colors ${
        active
          ? "border-blue-300 bg-blue-50 text-blue-600"
          : "border-gray-300 text-gray-500 hover:bg-gray-50"
      }`}
      title={title}
    >
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    </button>
  );
}
