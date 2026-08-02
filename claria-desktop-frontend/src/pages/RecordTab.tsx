import { useState } from "react";
import {
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
import FileList from "../components/FileList";
import FilesPanelHeader from "../components/FilesPanelHeader";
import MemoRecorderBar from "../components/MemoRecorderBar";
import MemoReviewModal from "../components/MemoReviewModal";
import RecordFileDialogs from "../components/RecordFileDialogs";
import Spinner from "../components/Spinner";
import UploadingRows from "../components/UploadingRows";
import { ErrorBanner } from "../components/StateCards";
import { formatDateTime } from "../lib/format";
import { isChatHistory } from "../lib/recordFiles";
import { searchMatches } from "../lib/search";
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
  const [error, setError] = useState<string | null>(null);
  const { files, loading, uploading, refresh, upload, remove } = useRecordFiles(
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
  const regularFiles = files.filter(
    (f) =>
      !isChatHistory(f.filename) &&
      (searchMatches(f.filename, search.query) ||
        search.contentMatches.has(f.filename)),
  );

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-2xl mx-auto p-8">
        {error && <ErrorBanner message={error} />}

        {/* File list / drop zone */}
        <div
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
            <div className="p-8 text-center">
              <div className="flex items-center justify-center gap-2 text-gray-500 text-sm">
                <Spinner />
                <span>Loading files...</span>
              </div>
            </div>
          )}

          {!loading && chatHistoryFiles.length > 0 && (
            <ChatHistoryFolder
              clientId={clientId}
              files={chatHistoryFiles}
              onResume={onResumeChat}
              onDelete={fileDialog.openDelete}
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
                () => restoreDeletedFile(clientId, df.filename, df.version_id),
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
