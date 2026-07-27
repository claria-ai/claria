import { AUDIO_EXTENSIONS, fileExtension } from "../lib/recordFiles";

/** Square type badge for a record file, colour-coded by extension. */
export default function FileIcon({ filename }: { filename: string }) {
  const ext = fileExtension(filename);
  const isPdf = ext === "pdf";
  const isDoc = ext === "docx" || ext === "doc";
  const isAudio = AUDIO_EXTENSIONS.has(ext);

  return (
    <div
      className={`w-8 h-8 rounded flex items-center justify-center text-xs font-bold ${
        isPdf
          ? "bg-red-100 text-red-600"
          : isDoc
            ? "bg-blue-100 text-blue-600"
            : isAudio
              ? "bg-purple-100 text-purple-600"
              : "bg-gray-100 text-gray-500"
      }`}
    >
      {isPdf
        ? "PDF"
        : isDoc
          ? "DOC"
          : isAudio
            ? "AUD"
            : ext.toUpperCase().slice(0, 3) || "?"}
    </div>
  );
}
