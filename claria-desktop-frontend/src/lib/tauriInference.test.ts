import { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  getLuciaInferenceCount,
  subscribeLuciaInference,
} from "./luciaInference";
import {
  chatMessage,
  generateDraftPlan,
  generateFullReport,
  resumeDraftRun,
  runReviewSweeps,
  sendReportMessage,
  startDraftRun,
  stopStream,
  type ChatMessage,
  type ChatStreamEvent,
  type ReportBlockReferenceInput,
  type ReportTurnProgressView,
  type ReviewPassInput,
} from "./tauri";

type CommandResult =
  | { status: "ok"; data: unknown }
  | { status: "error"; error: string };
type Command = (...args: unknown[]) => Promise<CommandResult>;

const mocks = vi.hoisted(() => ({
  generateFullReport: vi.fn<Command>(),
  generateDraftPlan: vi.fn<Command>(),
  startDraftRun: vi.fn<Command>(),
  resumeDraftRun: vi.fn<Command>(),
  sendReportMessage: vi.fn<Command>(),
  runReviewSweeps: vi.fn<Command>(),
  chatMessage: vi.fn<Command>(),
  stopStream: vi.fn<Command>(),
  logFrontendEvent: vi.fn(async () => undefined),
}));

vi.mock("./bindings", () => ({ commands: mocks, events: {} }));

const REPORT_ID = "report-id";
const RUN_ID = "run-id";
const STREAM_ID = "stream-id";
const MODEL_ID = "test-model";
const REVISION = 7;
const references: ReportBlockReferenceInput[] = [
  { section_id: "section-id", block_index: 2 },
];
const passes: ReviewPassInput[] = [
  { property: "style", instructions: "Check the draft's style" },
];
const messages: ChatMessage[] = [{ role: "user", content: "Test request" }];
const filenames = ["context.txt"];
const progress: ReportTurnProgressView = { kind: "model_call_started", call_number: 1 };
const delta: ChatStreamEvent = { kind: "delta", text: "Test response" };
type InferenceEvent = ReportTurnProgressView | ChatStreamEvent;
type InferenceCommand = Exclude<keyof typeof mocks, "stopStream" | "logFrontendEvent">;

interface WrapperCase {
  name: InferenceCommand;
  start: (clientId: string, onEvent?: (event: InferenceEvent) => void) => Promise<unknown>;
  argsAfterClient: unknown[];
  event: InferenceEvent;
}

const cases: WrapperCase[] = [
  {
    name: "generateFullReport",
    start: (clientId, onEvent) => generateFullReport(
      clientId, REPORT_ID, REVISION, MODEL_ID, "Draft guidance", STREAM_ID, onEvent,
    ),
    argsAfterClient: [REPORT_ID, REVISION, MODEL_ID, "Draft guidance", STREAM_ID],
    event: progress,
  },
  {
    name: "generateDraftPlan",
    start: (clientId, onEvent) => generateDraftPlan(
      clientId, REPORT_ID, REVISION, "Plan instructions", STREAM_ID, onEvent,
    ),
    argsAfterClient: [REPORT_ID, REVISION, "Plan instructions", STREAM_ID],
    event: progress,
  },
  {
    name: "startDraftRun",
    start: (clientId, onEvent) => startDraftRun(
      clientId, REPORT_ID, RUN_ID, MODEL_ID, STREAM_ID, onEvent,
    ),
    argsAfterClient: [REPORT_ID, RUN_ID, MODEL_ID, STREAM_ID],
    event: progress,
  },
  {
    name: "resumeDraftRun",
    start: (clientId, onEvent) => resumeDraftRun(
      clientId, REPORT_ID, RUN_ID, "Resume instructions", MODEL_ID, STREAM_ID, onEvent,
    ),
    argsAfterClient: [REPORT_ID, RUN_ID, "Resume instructions", MODEL_ID, STREAM_ID],
    event: progress,
  },
  {
    name: "sendReportMessage",
    start: (clientId, onEvent) => sendReportMessage(
      clientId, REPORT_ID, REVISION, MODEL_ID, "Writer instruction", references, STREAM_ID, onEvent,
    ),
    argsAfterClient: [REPORT_ID, REVISION, MODEL_ID, "Writer instruction", references, STREAM_ID],
    event: progress,
  },
  {
    name: "runReviewSweeps",
    start: (clientId, onEvent) => runReviewSweeps(
      clientId, REPORT_ID, REVISION, passes, onEvent,
    ),
    argsAfterClient: [REPORT_ID, REVISION, passes],
    event: { kind: "review_pass_started", property: "style", index: 1, total: 1 },
  },
  {
    name: "chatMessage",
    start: (clientId, onEvent) => chatMessage(
      clientId, MODEL_ID, messages, STREAM_ID, "chat-id", filenames, "Chat name", onEvent,
    ),
    argsAfterClient: [MODEL_ID, messages, "chat-id", "Chat name", filenames, STREAM_ID],
    event: delta,
  },
];

function deferred() {
  let resolve!: (result: CommandResult) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<CommandResult>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function lastChannel(name: InferenceCommand): Channel<unknown> {
  const channel = mocks[name].mock.lastCall?.at(-1);
  if (!(channel instanceof Channel)) throw new Error(`${name} did not receive a Channel`);
  return channel;
}

const subscriptions: Array<() => void> = [];

function observe(clientId: string) {
  const counts: number[] = [];
  subscriptions.push(subscribeLuciaInference(clientId, () => {
    counts.push(getLuciaInferenceCount(clientId));
  }));
  return counts;
}

beforeEach(() => {
  vi.resetAllMocks();
  // Real Channels need the callback registry. Every command is mocked at the
  // generated binding boundary; any actual IPC attempt fails this harness.
  mockIPC(() => { throw new Error("Unexpected IPC outside mocked bindings"); });
});

afterEach(() => {
  subscriptions.splice(0).forEach((unsubscribe) => unsubscribe());
  clearMocks();
});

describe("client inference command wrappers", () => {
  it.each(cases)("$name preserves arguments, Channel events, and the returned value", async (entry) => {
    const clientId = `success-${entry.name}`;
    const counts = observe(clientId);
    const command = deferred();
    mocks[entry.name].mockImplementationOnce(() => {
      expect(getLuciaInferenceCount(clientId)).toBe(1);
      return command.promise;
    });
    const onEvent = vi.fn<(event: InferenceEvent) => void>();
    const request = entry.start(clientId, onEvent);
    const channel = lastChannel(entry.name);

    expect(mocks[entry.name]).toHaveBeenCalledOnce();
    expect(mocks[entry.name]).toHaveBeenCalledWith(clientId, ...entry.argsAfterClient, channel);
    // Arrays and objects, as well as scalar arguments, retain their identity.
    [clientId, ...entry.argsAfterClient].forEach((argument, index) => {
      expect(mocks[entry.name].mock.calls[0][index]).toBe(argument);
    });
    expect(channel.onmessage).toBe(onEvent);
    channel.onmessage(entry.event);
    expect(onEvent).toHaveBeenCalledWith(entry.event);
    expect(getLuciaInferenceCount(clientId)).toBe(1);

    // The bridge treats response data as opaque; it must return this exact object.
    const response = { result: entry.name };
    command.resolve({ status: "ok", data: response });
    expect(await request).toBe(response);
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    expect(counts).toEqual([1, 0]);
  });

  describe.each(["backend error", "promise rejection", "synchronous throw"] as const)("%s", (failureKind) => {
    it.each(cases)("$name clears activity and preserves the failure", async (entry) => {
      const clientId = `${failureKind}-${entry.name}`;
      const counts = observe(clientId);
      const failure = failureKind === "backend error" ? "Backend refused the request" : new Error(failureKind);
      if (failureKind === "synchronous throw") {
        mocks[entry.name].mockImplementationOnce(() => { throw failure; });
      } else if (typeof failure === "string") {
        mocks[entry.name].mockResolvedValueOnce({ status: "error", error: failure });
      } else {
        mocks[entry.name].mockRejectedValueOnce(failure);
      }

      await expect(entry.start(clientId)).rejects.toBe(failure);
      expect(getLuciaInferenceCount(clientId)).toBe(0);
      expect(counts).toEqual([1, 0]);
    });
  });

  it("aggregates overlapping planner, review, and chat requests while keeping clients separate", async () => {
    const clientId = "overlapping-wrapper-client";
    const otherClientId = "other-wrapper-client";
    const counts = observe(clientId);
    const otherCounts = observe(otherClientId);
    const planner = deferred();
    const review = deferred();
    const chat = deferred();
    const otherChat = deferred();
    mocks.generateDraftPlan.mockReturnValueOnce(planner.promise);
    mocks.runReviewSweeps.mockReturnValueOnce(review.promise);
    mocks.chatMessage.mockReturnValueOnce(chat.promise).mockReturnValueOnce(otherChat.promise);

    const planRequest = generateDraftPlan(clientId, REPORT_ID, REVISION, "Plan", STREAM_ID);
    const reviewRequest = runReviewSweeps(clientId, REPORT_ID, REVISION, passes);
    const chatRequest = chatMessage(clientId, MODEL_ID, messages, STREAM_ID);
    const otherRequest = chatMessage(otherClientId, MODEL_ID, messages, "other-stream");
    expect(getLuciaInferenceCount(clientId)).toBe(3);
    expect(getLuciaInferenceCount(otherClientId)).toBe(1);

    review.resolve({ status: "ok", data: {} });
    await reviewRequest;
    expect(getLuciaInferenceCount(clientId)).toBe(2);
    planner.reject("Planning failed");
    await expect(planRequest).rejects.toBe("Planning failed");
    expect(getLuciaInferenceCount(clientId)).toBe(1);
    chat.resolve({ status: "ok", data: {} });
    await chatRequest;
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    expect(getLuciaInferenceCount(otherClientId)).toBe(1);
    otherChat.resolve({ status: "ok", data: {} });
    await otherRequest;
    expect(counts).toEqual([1, 2, 3, 2, 1, 0]);
    expect(otherCounts).toEqual([1, 0]);
  });

  it("keeps Stop and chat done events separate from original request settlement", async () => {
    const clientId = "stopped-wrapper-client";
    const counts = observe(clientId);
    const chat = deferred();
    const stop = deferred();
    mocks.chatMessage.mockReturnValueOnce(chat.promise);
    mocks.stopStream.mockReturnValueOnce(stop.promise);
    const onEvent = vi.fn<(event: ChatStreamEvent) => void>();
    const request = chatMessage(clientId, MODEL_ID, messages, STREAM_ID, null, [], null, onEvent);
    const stopping = stopStream(STREAM_ID);
    expect(mocks.stopStream).toHaveBeenCalledWith(STREAM_ID);
    expect(getLuciaInferenceCount(STREAM_ID)).toBe(0);
    expect(getLuciaInferenceCount(clientId)).toBe(1);

    stop.resolve({ status: "ok", data: null });
    await stopping;
    expect(getLuciaInferenceCount(clientId)).toBe(1);
    mocks.stopStream.mockResolvedValueOnce({ status: "error", error: "Stop failed" });
    await expect(stopStream(STREAM_ID)).rejects.toBe("Stop failed");
    expect(getLuciaInferenceCount(clientId)).toBe(1);

    const done: ChatStreamEvent = { kind: "done", stop_reason: "stopped_by_user", usage: null };
    lastChannel("chatMessage").onmessage(done);
    expect(onEvent).toHaveBeenCalledWith(done);
    expect(getLuciaInferenceCount(clientId)).toBe(1);
    chat.resolve({ status: "ok", data: {} });
    await request;
    expect(counts).toEqual([1, 0]);
    expect(getLuciaInferenceCount(clientId)).toBe(0);
  });

  it("preserves omitted chat defaults and null resume instructions without event callbacks", async () => {
    const clientId = "optional-wrapper-client";
    mocks.chatMessage.mockResolvedValueOnce({ status: "ok", data: {} });
    const chat = chatMessage(clientId, MODEL_ID, messages, STREAM_ID);
    const chatChannel = lastChannel("chatMessage");
    expect(mocks.chatMessage).toHaveBeenCalledWith(
      clientId, MODEL_ID, messages, null, null, [], STREAM_ID, chatChannel,
    );
    expect(() => chatChannel.onmessage(delta)).not.toThrow();
    await chat;

    mocks.resumeDraftRun.mockResolvedValueOnce({ status: "ok", data: {} });
    const resume = resumeDraftRun(clientId, REPORT_ID, RUN_ID, null, MODEL_ID, STREAM_ID);
    const resumeChannel = lastChannel("resumeDraftRun");
    expect(mocks.resumeDraftRun).toHaveBeenCalledWith(
      clientId, REPORT_ID, RUN_ID, null, MODEL_ID, STREAM_ID, resumeChannel,
    );
    expect(() => resumeChannel.onmessage(progress)).not.toThrow();
    await resume;
    expect(getLuciaInferenceCount(clientId)).toBe(0);
  });
});
