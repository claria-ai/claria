import { useState } from "react";
import {
  createTextRecordFile,
  getRecordFileText,
  listDeletedFiles,
  loadChatHistory,
  restoreDeletedFile,
  updateTextRecordFile,
  type ChatHistoryDetail,
  type DeletedFile,
} from "../lib/tauri";
import ChatHistoryFolder from "../components/ChatHistoryFolder";
import CreateTextFileModal from "../components/CreateTextFileModal";
import DeletedSection from "../components/DeletedSection";
import DeleteFileModal from "../components/DeleteFileModal";
import DropZoneHint from "../components/DropZoneHint";
import EditTextModal from "../components/EditTextModal";
import FileIcon from "../components/FileIcon";
import FileList from "../components/FileList";
import FilesPanelHeader from "../components/FilesPanelHeader";
import MemoRecorderBar from "../components/MemoRecorderBar";
import MemoReviewModal from "../components/MemoReviewModal";
import Spinner from "../components/Spinner";
import TextPreviewModal from "../components/TextPreviewModal";
import TranscribeWizard from "../components/TranscribeWizard";
import TranscriptEditor from "../components/TranscriptEditor";
import UploadingRows from "../components/UploadingRows";
import VersionHistoryModal from "../components/VersionHistoryModal";
import { ErrorBanner } from "../components/StateCards";
import { formatDateTime } from "../lib/format";
import { searchMatches } from "../lib/search";
import {
  chatIdFromFilename,
  isAudioSidecar,
  isChatHistory,
} from "../lib/recordFiles";
import { recordFileVersions } from "../lib/versions";
import { useMemoRecorder } from "../lib/useMemoRecorder";
import { useMoreMode } from "../lib/useMoreMode";
import { useRecordFiles } from "../lib/useRecordFiles";
import { useRecordSearch } from "../lib/useRecordSearch";
import { useTranscriptionPrefs } from "../lib/useTranscriptionPrefs";
import { useWebviewFileDrop } from "../lib/useWebviewFileDrop";

/**
 * A client's files, and everything that reads from or writes to them: upload
 * by drop, live memo recording, search, version history and the text editors.
 *
 * Each of those owns its own state, in a hook or in a child. What stays here
 * is the state more than one of them touches — the error banner every
 * operation reports into, and the file list most of them invalidate — plus
 * which modal is open, which is the page's decision to make.
 */
export default function RecordTab({
  clientId,
  onResumeChat,
}: {
  clientId: string;
  onResumeChat: (detail: ChatHistoryDetail) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const { files, loading, uploading, refresh, upload, remove } = useRecordFiles(
    clientId,
    setError,
  );
  const search = useRecordSearch(clientId, setError);
  const transcriptionPrefs = useTranscriptionPrefs();

  const [previewText, setPreviewText] = useState<string | null>(null);
  const [previewFilename, setPreviewFilename] = useState<string | null>(null);
  const [editText, setEditText] = useState<string | null>(null);
  const [editFilename, setEditFilename] = useState<string | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null);
  const [showCreateText, setShowCreateText] = useState(false);
  const [chatFolderOpen, setChatFolderOpen] = useState(false);
  const [resumeLoading, setResumeLoading] = useState<string | null>(null);
  /** Which file's version history is open, if any. */
  const [versionFile, setVersionFile] = useState<string | null>(null);

  const [showTranscribeWizard, setShowTranscribeWizard] = useState(false);
  const [wizardDroppedFile, setWizardDroppedFile] = useState<string | null>(
    null,
  );

  // A drop while the wizard is open belongs to the wizard, which adopts the
  // path as its chosen file — otherwise it would upload via the fast path and
  // bypass the per-file options the user opened the wizard to set. Only the
  // first path is forwarded; the wizard is single-file.
  const dragging = useWebviewFileDrop({
    onDrop: upload,
    divert: (paths) => {
      if (!showTranscribeWizard) return false;
      const first = paths[0];
      if (first) setWizardDroppedFile(first);
      return true;
    },
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

  async function handlePreview(filename: string) {
    setPreviewFilename(filename);
    try {
      setPreviewText(await getRecordFileText(clientId, filename));
    } catch (e) {
      setPreviewText(`Error loading preview: ${String(e)}`);
    }
  }

  function handleClosePreview() {
    setPreviewText(null);
    setPreviewFilename(null);
  }

  async function handleEdit(filename: string) {
    setEditFilename(filename);
    try {
      setEditText(await getRecordFileText(clientId, filename));
    } catch (e) {
      setError(String(e));
    }
  }

  function handleCancelEdit() {
    setEditText(null);
    setEditFilename(null);
  }

  async function handleSaveEdit(filename: string, text: string) {
    setError(null);
    try {
      await updateTextRecordFile(clientId, filename, text);
      handleCancelEdit();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleDelete(filename: string) {
    setDeleteConfirm(null);
    await remove(filename);
  }

  async function handleResume(filename: string) {
    setResumeLoading(filename);
    try {
      onResumeChat(
        await loadChatHistory(clientId, chatIdFromFilename(filename)),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setResumeLoading(null);
    }
  }

  async function handleCreateTextFile(filename: string, content: string) {
    setError(null);
    try {
      await createTextRecordFile(clientId, filename, content);
      setShowCreateText(false);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

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
            onUploadAudio={() => setShowTranscribeWizard(true)}
            onCreateTextFile={() => setShowCreateText(true)}
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
              files={chatHistoryFiles}
              open={chatFolderOpen}
              onToggle={() => setChatFolderOpen(!chatFolderOpen)}
              resumeLoading={resumeLoading}
              onResume={handleResume}
              onDelete={setDeleteConfirm}
            />
          )}

          {!loading && regularFiles.length > 0 && (
            <FileList
              files={regularFiles}
              query={search.query}
              contentMatches={search.contentMatches}
              showVersions={moreMode}
              onOpenVersions={setVersionFile}
              onEdit={handleEdit}
              onPreview={handlePreview}
              onDelete={setDeleteConfirm}
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

      {/* Preview / segment editor.
          Audio sidecars get the structured TranscriptEditor (header parser,
          per-segment rows, speaker rename, save / restore-to-original via S3
          versioning). PDF/DOCX sidecars are plain text — render read-only. */}
      {previewText !== null &&
        previewFilename &&
        (isAudioSidecar(previewFilename) ? (
          <TranscriptEditor
            clientId={clientId}
            filename={previewFilename}
            initialBody={previewText}
            onClose={handleClosePreview}
            onSaved={(newBody) => setPreviewText(newBody)}
          />
        ) : (
          <TextPreviewModal
            filename={previewFilename}
            text={previewText}
            onClose={handleClosePreview}
          />
        ))}

      {editText !== null && editFilename && (
        <EditTextModal
          filename={editFilename}
          initialText={editText}
          onCancel={handleCancelEdit}
          onSave={(text) => handleSaveEdit(editFilename, text)}
        />
      )}

      {deleteConfirm && (
        <DeleteFileModal
          filename={deleteConfirm}
          onCancel={() => setDeleteConfirm(null)}
          onConfirm={() => handleDelete(deleteConfirm)}
        />
      )}

      {showTranscribeWizard && (
        <TranscribeWizard
          clientId={clientId}
          droppedFilePath={wizardDroppedFile}
          isDragging={dragging}
          onClose={() => {
            setShowTranscribeWizard(false);
            setWizardDroppedFile(null);
          }}
          onUploaded={() => {
            void refresh();
          }}
        />
      )}

      {showCreateText && (
        <CreateTextFileModal
          onCancel={() => setShowCreateText(false)}
          onCreate={handleCreateTextFile}
        />
      )}

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

      {versionFile && (
        <VersionHistoryModal
          title={
            isAudioSidecar(versionFile)
              ? `Transcript History: ${versionFile}`
              : `Version History: ${versionFile}`
          }
          source={recordFileVersions(clientId, versionFile)}
          onClose={() => setVersionFile(null)}
          onRestored={refresh}
          onError={setError}
          enableCompare
          showFooterClose
          className="max-w-4xl p-6 max-h-[90vh] flex flex-col"
        />
      )}
    </div>
  );
}
