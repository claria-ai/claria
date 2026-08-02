import { useCallback, useEffect, useRef, useState } from "react";
import {
  deleteRecordFile,
  listRecordFiles,
  uploadRecordFile,
  type RecordFile,
} from "./tauri";
import { basename } from "./recordFiles";

/**
 * The file list for one client record, plus the two writes that change it.
 *
 * Uploads run one path at a time rather than in parallel: each one triggers
 * server-side extraction, and a dropped folder's worth of PDFs firing at once
 * is how the extraction path gets rate-limited. Each filename appears in
 * `uploading` only while its own request is in flight, so a failure part-way
 * through a batch still lets the rest through and still refreshes at the end.
 */
export function useRecordFiles(
  clientId: string,
  onError: (message: string | null) => void,
) {
  const [files, setFiles] = useState<RecordFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [uploading, setUploading] = useState<string[]>([]);

  const onErrorRef = useRef(onError);
  useEffect(() => {
    onErrorRef.current = onError;
  });

  const refresh = useCallback(async () => {
    onErrorRef.current(null);
    try {
      setFiles(await listRecordFiles(clientId));
    } catch (e) {
      onErrorRef.current(String(e));
    } finally {
      setLoading(false);
    }
  }, [clientId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function upload(paths: string[]) {
    for (const path of paths) {
      const filename = basename(path);
      setUploading((prev) => [...prev, filename]);
      try {
        await uploadRecordFile(clientId, path);
      } catch (e) {
        onErrorRef.current(String(e));
      } finally {
        setUploading((prev) => prev.filter((f) => f !== filename));
      }
    }
    await refresh();
  }

  async function remove(filename: string) {
    try {
      await deleteRecordFile(clientId, filename);
      await refresh();
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }

  return { files, loading, uploading, refresh, upload, remove };
}
