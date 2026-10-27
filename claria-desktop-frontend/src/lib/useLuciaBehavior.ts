import { useCallback, useEffect, useReducer, useState } from "react";
import {
  createLuciaBehavior,
  reduceLuciaBehavior,
  type LuciaBehaviorConfig,
  type LuciaBehaviorEvent,
  type LuciaBehaviorState,
  type LuciaMode,
} from "./luciaBehavior";
import { getLuciaInferenceCount, subscribeLuciaInference } from "./luciaInference";

export interface LuciaBehaviorRuntime {
  now: () => number;
  random: () => number;
}

const browserRuntime: LuciaBehaviorRuntime = {
  now: () => performance.now(),
  random: () => Math.random(),
};

interface TimedEvent {
  event: LuciaBehaviorEvent;
  now: number;
  roll: number;
}

function isWritingTarget(target: EventTarget | null): boolean {
  if (target instanceof HTMLTextAreaElement) {
    return !target.readOnly && !target.disabled;
  }
  if (target instanceof HTMLInputElement) {
    return (
      !target.readOnly &&
      !target.disabled &&
      ["text", "search", "email", "url", "tel", "number"].includes(target.type)
    );
  }
  return target instanceof HTMLElement && target.isContentEditable;
}

/** Mount with the opted-in client scene; unmounting releases every listener and timer. */
export function useLuciaBehavior(
  config: LuciaBehaviorConfig,
  clientId: string,
  runtime: LuciaBehaviorRuntime = browserRuntime,
) {
  const [motionPreference] = useState(() =>
    window.matchMedia("(prefers-reduced-motion: reduce)"),
  );
  const [state, dispatch] = useReducer(
    (previous: LuciaBehaviorState, action: TimedEvent) =>
      reduceLuciaBehavior(previous, action.event, action.now, config, action.roll),
    undefined,
    () => {
      const now = runtime.now();
      const initial = reduceLuciaBehavior(
        createLuciaBehavior(now),
        {
          type: "environment",
          active: true,
          visible: !document.hidden,
          reducedMotion: motionPreference.matches,
        },
        now,
        config,
      );
      return reduceLuciaBehavior(
        initial,
        { type: "inference", pending: getLuciaInferenceCount(clientId) > 0 },
        now,
        config,
      );
    },
  );
  const send = useCallback(
    (event: LuciaBehaviorEvent) => {
      dispatch({
        event,
        now: runtime.now(),
        // Randomness is sampled outside the reducer, once per scheduled wake.
        roll: event.type === "wake" ? runtime.random() : 0.5,
      });
    },
    [runtime],
  );

  useEffect(() => {
    const updateEnvironment = () =>
      send({
        type: "environment",
        active: true,
        visible: !document.hidden,
        reducedMotion: motionPreference.matches,
      });
    const onInput = (event: Event) => {
      if (isWritingTarget(event.target)) send({ type: "typing" });
    };
    document.addEventListener("input", onInput, true);
    document.addEventListener("visibilitychange", updateEnvironment);
    motionPreference.addEventListener("change", updateEnvironment);
    return () => {
      document.removeEventListener("input", onInput, true);
      document.removeEventListener("visibilitychange", updateEnvironment);
      motionPreference.removeEventListener("change", updateEnvironment);
    };
  }, [motionPreference, send]);

  useEffect(() => {
    const updateInference = () =>
      send({ type: "inference", pending: getLuciaInferenceCount(clientId) > 0 });
    const unsubscribe = subscribeLuciaInference(clientId, updateInference);
    updateInference();
    return unsubscribe;
  }, [clientId, send]);

  useEffect(() => {
    if (state.nextWakeAt === null) return;
    const deadline = state.nextWakeAt;
    let timer: number;
    const wake = () => {
      const remaining = deadline - runtime.now();
      // Browser timer rounding can wake slightly before a monotonic deadline.
      // Keep ownership of that deadline instead of consuming an ignored event.
      if (remaining > 0) {
        timer = window.setTimeout(wake, Math.max(1, Math.ceil(remaining)));
        return;
      }
      send({ type: "wake" });
    };
    timer = window.setTimeout(wake, Math.max(0, Math.ceil(deadline - runtime.now())));
    return () => window.clearTimeout(timer);
  }, [runtime, send, state.nextWakeAt]);

  const dropSock = useCallback(() => send({ type: "sock" }), [send]);
  const setMode = useCallback((mode: LuciaMode) => send({ type: "mode", mode }), [send]);
  const bark = useCallback(() => send({ type: "bark" }), [send]);
  const feed = useCallback(() => send({ type: "feed" }), [send]);
  const reset = useCallback(() => send({ type: "reset" }), [send]);
  return { state, dropSock, setMode, bark, feed, reset };
}
