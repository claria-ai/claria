import Spinner from "./Spinner";

/**
 * One row per upload still in flight.
 *
 * Uploads are shown separately from the file list rather than optimistically
 * inserted into it: the file only exists once the backend has stored it and
 * extracted its text, and a row that looks real but isn't invites the user to
 * click it.
 */
export default function UploadingRows({ filenames }: { filenames: string[] }) {
  if (filenames.length === 0) return null;

  return (
    <div className="divide-y divide-gray-100 border-t border-gray-100">
      {filenames.map((filename) => (
        <div key={filename} className="px-4 py-3 flex items-center gap-3">
          <Spinner />
          <div className="flex-1 min-w-0">
            <p className="text-sm text-gray-500 truncate">
              Uploading {filename}...
            </p>
          </div>
        </div>
      ))}
    </div>
  );
}
