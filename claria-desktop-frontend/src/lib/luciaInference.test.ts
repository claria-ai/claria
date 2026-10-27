import { afterEach, describe, expect, it, vi } from "vitest";
import { logFrontendEvent } from "./logBridge";
import {
  getLuciaInferenceCount,
  subscribeLuciaInference,
  withLuciaInference,
} from "./luciaInference";

vi.mock("./logBridge", () => ({ logFrontendEvent: vi.fn() }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const subscriptions: Array<() => void> = [];

function observe(clientId: string) {
  const counts: number[] = [];
  const unsubscribe = subscribeLuciaInference(clientId, () => {
    counts.push(getLuciaInferenceCount(clientId));
  });
  subscriptions.push(unsubscribe);
  return { counts, unsubscribe };
}

afterEach(() => {
  subscriptions.splice(0).forEach((unsubscribe) => unsubscribe());
  vi.clearAllMocks();
});

describe("Lucia inference activity", () => {
  it("counts an actual command before invoking it and preserves its result", async () => {
    const clientId = "resolved-client";
    const command = deferred<{ content: string }>();
    const observed = observe(clientId);
    const operation = vi.fn(() => {
      expect(getLuciaInferenceCount(clientId)).toBe(1);
      return command.promise;
    });

    expect(getLuciaInferenceCount(clientId)).toBe(0);
    const request = withLuciaInference(clientId, operation);
    expect(operation).toHaveBeenCalledOnce();
    expect(observed.counts).toEqual([1]);

    const response = { content: "Finished" };
    command.resolve(response);
    expect(await request).toBe(response);
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    expect(observed.counts).toEqual([1, 0]);
  });

  it("retires a rejected command and preserves the original error", async () => {
    const clientId = "rejected-client";
    const command = deferred<void>();
    const observed = observe(clientId);
    const request = withLuciaInference(clientId, () => command.promise);
    const failure = new Error("The request failed");

    command.reject(failure);
    await expect(request).rejects.toBe(failure);
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    expect(observed.counts).toEqual([1, 0]);
  });

  it("also retires an operation that throws before returning its promise", async () => {
    const clientId = "throwing-client";
    const observed = observe(clientId);
    const failure = new Error("The command could not start");
    const request = withLuciaInference(clientId, () => {
      expect(getLuciaInferenceCount(clientId)).toBe(1);
      throw failure;
    });

    await expect(request).rejects.toBe(failure);
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    expect(observed.counts).toEqual([1, 0]);
  });

  it("keeps overlapping chat, planner, and review requests counted until each settles", async () => {
    const clientId = "overlapping-client";
    const observed = observe(clientId);
    const chat = deferred<string>();
    const planner = deferred<string>();
    const review = deferred<string>();
    const chatRequest = withLuciaInference(clientId, () => chat.promise);
    const planRequest = withLuciaInference(clientId, () => planner.promise);
    const reviewRequest = withLuciaInference(clientId, () => review.promise);
    expect(getLuciaInferenceCount(clientId)).toBe(3);

    planner.resolve("Plan ready");
    await planRequest;
    expect(getLuciaInferenceCount(clientId)).toBe(2);

    const failure = new Error("Review failed");
    review.reject(failure);
    await expect(reviewRequest).rejects.toBe(failure);
    expect(getLuciaInferenceCount(clientId)).toBe(1);

    chat.resolve("Reply ready");
    await chatRequest;
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    // A duplicate attempt to settle a promise cannot retire another request
    // or move the counter below zero.
    chat.resolve("Duplicate completion");
    review.reject(failure);
    await Promise.resolve();
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    expect(observed.counts).toEqual([1, 2, 3, 2, 1, 0]);
  });

  it("keeps different clients' counts and notifications independent", async () => {
    const first = observe("first-client");
    const second = observe("second-client");
    const firstCommand = deferred<void>();
    const secondCommand = deferred<void>();
    const firstRequest = withLuciaInference("first-client", () => firstCommand.promise);
    expect(first.counts).toEqual([1]);
    expect(second.counts).toEqual([]);
    expect(getLuciaInferenceCount("second-client")).toBe(0);

    const secondRequest = withLuciaInference("second-client", () => secondCommand.promise);
    firstCommand.resolve();
    await firstRequest;
    expect(getLuciaInferenceCount("first-client")).toBe(0);
    expect(getLuciaInferenceCount("second-client")).toBe(1);
    expect(first.counts).toEqual([1, 0]);
    expect(second.counts).toEqual([1]);

    secondCommand.resolve();
    await secondRequest;
    expect(second.counts).toEqual([1, 0]);
  });

  it("keeps a request alive after unsubscribe and lets a remounted scene observe its settlement", async () => {
    const clientId = "navigation-client";
    const command = deferred<void>();
    const previousScene = observe(clientId);
    const request = withLuciaInference(clientId, () => command.promise);
    previousScene.unsubscribe();
    expect(getLuciaInferenceCount(clientId)).toBe(1);

    const nextScene = observe(clientId);
    // Repeated cleanup of an old subscription must not detach a new one.
    previousScene.unsubscribe();
    expect(getLuciaInferenceCount(clientId)).toBe(1);
    command.resolve();
    await request;
    expect(getLuciaInferenceCount(clientId)).toBe(0);
    expect(previousScene.counts).toEqual([1]);
    expect(nextScene.counts).toEqual([0]);
  });

  it("owns repeated subscriptions of the same callback independently", async () => {
    const clientId = "shared-listener-client";
    const listener = vi.fn();
    const unsubscribeFirst = subscribeLuciaInference(clientId, listener);
    const unsubscribeSecond = subscribeLuciaInference(clientId, listener);
    subscriptions.push(unsubscribeFirst, unsubscribeSecond);
    unsubscribeFirst();

    await withLuciaInference(clientId, async () => "Done");
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it.each(["resolved", "rejected"] as const)(
    "a throwing observer cannot alter a %s command or silence other observers",
    async (outcome) => {
      const clientId = `throwing-observer-${outcome}`;
      subscriptions.push(subscribeLuciaInference(clientId, () => {
        throw new Error("Observer failed with private request context");
      }));
      const observed = observe(clientId);
      const command = deferred<{ content: string }>();
      const operation = vi.fn(() => command.promise);
      const request = withLuciaInference(clientId, operation);

      expect(operation).toHaveBeenCalledOnce();
      expect(getLuciaInferenceCount(clientId)).toBe(1);
      expect(observed.counts).toEqual([1]);

      if (outcome === "resolved") {
        const response = { content: "Finished" };
        command.resolve(response);
        expect(await request).toBe(response);
      } else {
        const failure = new Error("The original request failed");
        command.reject(failure);
        await expect(request).rejects.toBe(failure);
      }

      expect(getLuciaInferenceCount(clientId)).toBe(0);
      expect(observed.counts).toEqual([1, 0]);
      expect(vi.mocked(logFrontendEvent).mock.calls).toEqual([
        ["error", "Lucia inference listener failed"],
        ["error", "Lucia inference listener failed"],
      ]);
    },
  );
});
