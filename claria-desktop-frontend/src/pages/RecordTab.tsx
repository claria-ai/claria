import { useMemo, useRef, useState } from "react";
import {
  listChatHistories,
  listDeletedFiles,
  restoreDeletedFile,
  type ChatHistoryDetail,
  type DeletedFile,
} from "../lib/tauri";
import ChatHistoryFolder from "../components/ChatHistoryFolder";
import DeletedSection from "../components/DeletedSection";
import DropZoneHint from "../components/DropZoneHint";
import EditorHistoryFolder from "../components/EditorHistoryFolder";
import FileIcon from "../components/FileIcon";
import ProcessingFailuresModal from "../components/ProcessingFailuresModal";
import FileList from "../components/FileList";
import FilesPanelHeader from "../components/FilesPanelHeader";
import MemoRecorderBar from "../components/MemoRecorderBar";
import MemoReviewModal from "../components/MemoReviewModal";
import RecordFileDialogs from "../components/RecordFileDialogs";
import UploadingRows from "../components/UploadingRows";
import { ErrorBanner, LoadingCard } from "../components/StateCards";
import { formatDateTime } from "../lib/format";
import { isChatHistory } from "../lib/recordFiles";
import { searchMatches } from "../lib/search";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { useEditorHistory } from "../lib/useEditorHistory";
import { useMemoRecorder } from "../lib/useMemoRecorder";
import { useMoreMode } from "../lib/useMoreMode";
import { useRecordFileDialog } from "../lib/useRecordFileDialog";
import { useRecordFiles } from "../lib/useRecordFiles";
import { useRecordSearch } from "../lib/useRecordSearch";
import { useTranscriptionPrefs } from "../lib/useTranscriptionPrefs";
import { useWebviewFileDrop } from "../lib/useWebviewFileDrop";

/**
 * A client's files, and everything that reads from or writes to them: upload
 * by drop, live memo recording, search, version history and the text editors.
 *
 * Each feature owns its state in a hook or child. This page only composes
 * them around the shared file list and error banner.
 */
export default function RecordTab({
  clientId,
  onResumeChat,
  onResumeWriting,
}: {
  clientId: string;
  onResumeChat: (detail: ChatHistoryDetail) => void;
  onResumeWriting: (reportId: string) => void;
}) {
  const recordPanel = useRef<HTMLDivElement>(null);
  const [needsTextOnly, setNeedsTextOnly] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { files, loading, uploading, refresh, upload, remove, failures, processingOpen, openProcessing, dismissProcessing, retrying, retry, review } = useRecordFiles(
    clientId,
    setError,
  );
  const search = useRecordSearch(clientId, setError);
  const transcriptionPrefs = useTranscriptionPrefs();
  const editorHistory = useEditorHistory(clientId);
  const fileDialog = useRecordFileDialog({
    clientId,
    onError: setError,
    onChanged: refresh,
    remove,
  });

  const dragging = useWebviewFileDrop({
    onDrop: upload,
    divert: fileDialog.divertDrop,
  });

  // Live memo recording — microphone, PCM buffer and timers all live in here.
  const memo = useMemoRecorder({
    clientId,
    onError: setError,
    onSaved: refresh,
  });

  const {
    moreMode,
    toggleMoreMode,
    deletedItems: deletedFiles,
    deletedLoading: deletedFilesLoading,
    restoringKey,
    restore,
  } = useMoreMode<DeletedFile>(
    () => listDeletedFiles(clientId),
    (e) => setError(String(e)),
  );

  const chatHistoryFiles = files
    .filter((f) => isChatHistory(f.filename))
    .sort((a, b) => (b.uploaded_at ?? "").localeCompare(a.uploaded_at ?? ""));
  const chatHistorySignature = useMemo(
    () => chatHistoryFiles.map((file) => `${file.filename}:${file.uploaded_at}`).join("|"),
    [chatHistoryFiles]
  );
  const chatHistoriesLoad = useAsyncLoad(
    chatHistorySignature === "" ? null : () => listChatHistories(clientId),
    [clientId, chatHistorySignature]
  );
  const visibleChatHistories =
    chatHistorySignature === "" ? [] : (chatHistoriesLoad.data ?? []);
  // Feature errors share one banner; the chat-history load error joins it
  // without being copied into state.
  const bannerError = error ?? chatHistoriesLoad.error;

  const regularFiles = files.filter(
    (f) =>
      !isChatHistory(f.filename) &&
      (!needsTextOnly || f.processing_status?.status === "failed" || f.processing_status?.status === "unknown") &&
      (searchMatches(f.filename, search.query) ||
        search.contentMatches.has(f.filename)),
  );

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-2xl mx-auto p-8">
        {bannerError && <ErrorBanner message={bannerError} />}

        {files.some((file) => file.processing_status?.status === "unknown" || file.processing_status?.status === "failed") && (
          <div className="mb-4 p-4 rounded-lg border border-amber-200 bg-amber-50 text-sm text-amber-900">
            <p>Some files do not have usable text. For older files, the previous processing outcome is unknown.</p>
            <label className="flex items-center gap-2 mt-3"><input type="checkbox" checked={needsTextOnly} onChange={(event) => setNeedsTextOnly(event.target.checked)} />Show only files needing text</label>
          </div>
        )}
        {failures.length > 0 && !processingOpen && <button onClick={openProcessing} className="mb-4 text-sm text-red-700 hover:underline">Review {failures.length} processing failures</button>}
        {/* File list / drop zone */}
        <div
          ref={recordPanel}
          tabIndex={-1}
          aria-label="Record files"
          className={`border-2 rounded-lg transition-colors ${
            dragging ? "border-blue-400 bg-blue-50" : "border-gray-200 bg-white"
          }`}
        >
          <FilesPanelHeader
            search={search.search}
            onSearchChange={search.setSearch}
            searchOpen={search.searchOpen}
            onOpenSearch={search.open}
            onCloseSearch={search.close}
            moreMode={moreMode}
            onToggleMoreMode={toggleMoreMode}
            showRecordMemo={memo.ready && memo.state === "idle"}
            onRecordMemo={memo.start}
            onUploadAudio={fileDialog.openTranscription}
            onCreateTextFile={fileDialog.openCreate}
          />

          <MemoRecorderBar
            state={memo.state}
            elapsed={memo.elapsed}
            transcript={memo.transcript}
            onTranscriptChange={memo.setTranscript}
            multilingual={memo.multilingual}
            detectedLanguage={memo.detectedLanguage}
            gpu={memo.gpu}
            modelLabel={memo.modelLabel}
            onPause={memo.pause}
            onResume={memo.resume}
            onDone={memo.done}
            onCancel={memo.cancel}
          />

          {loading && (
            <div className="p-4">
              <LoadingCard>Loading files...</LoadingCard>
            </div>
          )}

          {!loading && visibleChatHistories.length > 0 && (
            <ChatHistoryFolder
              clientId={clientId}
              entries={visibleChatHistories}
              onResume={onResumeChat}
              onDelete={fileDialog.openDelete}
              onReviewProcessing={review}
              onError={setError}
            />
          )}

          {!loading && (
            <EditorHistoryFolder
              entries={editorHistory}
              onResume={onResumeWriting}
            />
          )}

          {!loading && regularFiles.length > 0 && (
            <FileList
              files={regularFiles}
              query={search.query}
              contentMatches={search.contentMatches}
              showVersions={moreMode}
              onOpenVersions={fileDialog.openVersions}
              onEdit={fileDialog.openEdit}
              onPreview={fileDialog.openPreview}
              onDelete={fileDialog.openDelete}
              onReviewProcessing={review}
            />
          )}

          <UploadingRows filenames={uploading} />

          {!loading && search.query !== "" && regularFiles.length === 0 && (
            <div className="px-4 py-6 text-center">
              <p className="text-sm text-gray-400">
                No file names or contents match &ldquo;{search.query}&rdquo;
              </p>
            </div>
          )}

          {!loading && (
            <DropZoneHint
              dragging={dragging}
              prefs={transcriptionPrefs}
              spacious={files.length === 0 && uploading.length === 0}
            />
          )}
        </div>
      </div>

      {processingOpen && <ProcessingFailuresModal files={failures} retrying={retrying} onRetry={retry} onClose={() => {
        dismissProcessing();
        requestAnimationFrame(() => {
          if (!document.activeElement || document.activeElement === document.body) recordPanel.current?.focus();
        });
      }} />}

      <RecordFileDialogs
        clientId={clientId}
        dragging={dragging}
        controller={fileDialog}
        onError={setError}
      />


      {memo.state === "review" && (
        <MemoReviewModal
          filename={memo.filename}
          onFilenameChange={memo.setFilename}
          transcript={memo.transcript}
          onTranscriptChange={memo.setTranscript}
          saving={memo.saving}
          onDiscard={memo.cancel}
          onSave={memo.save}
        />
      )}

      {/* Deleted files (More mode) */}
      {moreMode && !loading && (
        <div className="max-w-2xl mx-auto px-8 pb-8">
          <DeletedSection
            title="Deleted Files"
            noun="files"
            loading={deletedFilesLoading}
            items={deletedFiles}
            itemKey={(df) => df.filename}
            primary={(df) => df.filename}
            subtitle={(df) =>
              df.deleted_at
                ? `Deleted ${formatDateTime(df.deleted_at)}`
                : "Deleted"
            }
            icon={(df) => <FileIcon filename={df.filename} />}
            restoringKey={restoringKey}
            onRestore={(df) =>
              restore(
                df.filename,
                () => restoreDeletedFile(clientId, df.filename),
                (f) => f.filename === df.filename,
                refresh,
              )
            }
          />
        </div>
      )}

    </div>
  );
}
