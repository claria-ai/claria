import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  deleteRecordFile,
  listRecordFiles,
  uploadRecordFile,
  extractRecordFile,
  loadConfig,
  type RecordFile,
} from "./tauri";
import { useAsyncLoad } from "./useAsyncLoad";
import { beginProcessing, finishProcessing, processingSnapshot, reviewProcessing, subscribeProcessing, type ProcessingFile } from "./fileProcessing";
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
  const scope = useAsyncLoad(accountScope, [clientId]);
  const processingScope = `${scope.data ?? "pending"}:${clientId}`;
  const processing = useSyncExternalStore(useCallback((listener) => subscribeProcessing(processingScope, listener), [processingScope]), () => processingSnapshot(processingScope));
  const failures = processing.failures;
  const [processingOpen, setProcessingOpen] = useState(false);
  const [retrying, setRetrying] = useState<string | null>(null);
  const generation = useRef(0);
  useEffect(() => () => { generation.current += 1; }, [clientId]);
  const [files, setFiles] = useState<RecordFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [uploading, setUploading] = useState<string[]>([]);

  const onErrorRef = useRef(onError);
  useEffect(() => {
    onErrorRef.current = onError;
  });

  const refresh = useCallback(async () => {
    const request = generation.current;
    try {
      const next = await listRecordFiles(clientId);
      if (request === generation.current) setFiles(next);
    } catch (e) {
      if (request === generation.current) onErrorRef.current(String(e));
    } finally {
      if (request === generation.current) setLoading(false);
    }
  }, [clientId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function upload(paths: string[]) {
    const request = generation.current;
    // A drop that lands before the account loads waits for it rather than
    // being discarded; if it cannot load, the backend skips the scope check.
    const expectedScope = scope.data ?? (await accountScope().catch(() => null));
    const uploadScope = `${expectedScope ?? "pending"}:${clientId}`;
    for (const path of paths) {
      if (request !== generation.current) return;
      const filename = basename(path);
      if (!beginProcessing(uploadScope, filename)) continue;
      setUploading((prev) => [...prev, filename]);
      try {
        const file = await uploadRecordFile(clientId, path, expectedScope);
        finishProcessing(uploadScope, filename, file);
        if (request === generation.current && file.processing_status?.status === "failed") {
          setProcessingOpen(true);
        }
      } catch (e) {
        finishProcessing(uploadScope, filename, { filename, size: 0, uploaded_at: null, originalSaved: false,
          processing_status: { status: "failed", code: "storage_unavailable", reason: String(e) } });
        if (request !== generation.current) return;
        setProcessingOpen(true);
        onErrorRef.current(String(e));
      } finally {
        if (request === generation.current) setUploading((prev) => prev.filter((f) => f !== filename));
      }
    }
    if (request === generation.current) await refresh();
  }

  async function remove(filename: string) {
    try {
      await deleteRecordFile(clientId, filename);
      await refresh();
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }

  async function retry(filename: string) {
    if (retrying || !scope.data) return;
    const request = generation.current;
    if (!beginProcessing(processingScope, filename)) return;
    setRetrying(filename);
    try {
      await extractRecordFile(clientId, filename, scope.data);
      finishProcessing(processingScope, filename);
      if (request !== generation.current) return;
      await refresh();
    } catch (reason) {
      // The backend saved the outcome as an `.error` marker: read it back so
      // the dialog shows the real code and reason instead of a guess.
      const previous = failures.find((entry) => entry.filename === filename);
      const listed = await listRecordFiles(clientId).catch(() => null);
      if (listed && request === generation.current) setFiles(listed);
      const saved = listed?.find((entry) => entry.filename === filename);
      const status = saved?.processing_status;
      const file: ProcessingFile = saved && status?.status === "failed"
        ? { ...saved, processing_status: status }
        : {
            ...(previous ?? { filename, size: 0, uploaded_at: null }),
            processing_status: {
              status: "failed",
              code: previous?.processing_status?.status === "failed"
                ? previous.processing_status.code
                : "provider_unavailable",
              reason: String(reason),
            },
          };
      finishProcessing(processingScope, filename, file);
    } finally { if (request === generation.current) setRetrying(null); }
  }
  function review(file: RecordFile) {
    reviewProcessing(processingScope, file);
    setProcessingOpen(true);
  }
  return { files, loading, uploading, refresh, upload, remove, failures, processingOpen,
    openProcessing: () => setProcessingOpen(true),
    dismissProcessing: () => setProcessingOpen(false), retrying, retry, review };

}

/** The account, region and system the record view was loaded for. */
async function accountScope(): Promise<string> {
  const config = await loadConfig();
  return `${config.account_id}:${config.region}:${config.system_name}`;
}
