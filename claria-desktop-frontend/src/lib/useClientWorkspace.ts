import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  type ClientWorkspaceTab,
  type ClientWorkspaceView,
} from "../components/ClientWorkspaceTabs";
import { type ResumeChat } from "../pages/ClientChat";
import { type WritingLeaveState } from "../pages/Writing";
import { type ChatHistoryDetail } from "./tauri";

const cleanWritingState: WritingLeaveState = {
  hasUnsavedWork: false,
  hasUnsavedReportEdits: false,
  busy: false,
};

/**
 * Cross-view navigation for a client workspace.
 *
 * Record, Chat, Writing, and Settings own their feature state. This hook owns
 * only the state needed to cross those boundaries: persisted sessions being
 * resumed, the Writing instance identity, and guards against losing work.
 */
export function useClientWorkspace(onBack: () => void) {
  const [activeView, setActiveView] =
    useState<ClientWorkspaceView>("record");
  const [pendingChat, setPendingChat] = useState<ResumeChat | null>(null);
  const [writingLeaveState, setWritingLeaveState] =
    useState<WritingLeaveState>(cleanWritingState);
  const [expectedReportId, setExpectedReportId] = useState<string | null>(null);
  const [writingInstance, setWritingInstance] = useState(0);

  function openChat(detail: ChatHistoryDetail) {
    setPendingChat({
      chatId: detail.chat_id,
      modelId: detail.model_id,
      messages: detail.messages.map((message) => ({
        role: message.role,
        content: message.content,
      })),
      usageByIndex: detail.messages.map((message) => message.usage),
      lastActivityIso: detail.created_at,
    });
    setActiveView("chat");
  }

  function openWriting(reportId: string) {
    setExpectedReportId(reportId);
    setWritingInstance((value) => value + 1);
    setActiveView("writing");
  }

  function mayLeaveWriting(): boolean {
    if (activeView !== "writing") return true;
    if (writingLeaveState.busy) {
      window.alert("Wait for the current Writing action to finish before leaving.");
      return false;
    }
    // Composer text and report-block references stay in process memory while
    // navigating. Only inline report edits need confirmation.
    return (
      !writingLeaveState.hasUnsavedReportEdits ||
      window.confirm(
        "Discard unsaved report edits? Your typed Writing instruction and references will be kept."
      )
    );
  }

  function selectTab(next: ClientWorkspaceTab): boolean {
    if (next === activeView) return true;
    if (!mayLeaveWriting()) return false;
    if (next === "writing") {
      setExpectedReportId(null);
      setWritingInstance((value) => value + 1);
    }
    setActiveView(next);
    return true;
  }

  function openSettings(): boolean {
    if (activeView === "settings") return true;
    if (!mayLeaveWriting()) return false;
    setActiveView("settings");
    return true;
  }

  function back() {
    if (mayLeaveWriting()) onBack();
  }

  useEffect(() => {
    if (
      activeView !== "writing" ||
      (!writingLeaveState.hasUnsavedWork && !writingLeaveState.busy)
    ) {
      return;
    }

    const beforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", beforeUnload);

    let disposed = false;
    let unlisten: (() => void) | undefined;
    if ("__TAURI_INTERNALS__" in window) {
      void getCurrentWindow()
        .onCloseRequested((event) => {
          if (writingLeaveState.busy) {
            event.preventDefault();
          } else if (
            writingLeaveState.hasUnsavedWork &&
            !window.confirm("Discard your unsaved report work and close Claria?")
          ) {
            event.preventDefault();
          }
        })
        .then((stopListening) => {
          if (disposed) stopListening();
          else unlisten = stopListening;
        });
    }

    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [activeView, writingLeaveState]);

  return {
    activeView,
    pendingChat,
    expectedReportId,
    writingInstance,
    openChat,
    openWriting,
    openSettings,
    selectTab,
    back,
    clearPendingChat: () => setPendingChat(null),
    updateWritingLeaveState: setWritingLeaveState,
  };
}
