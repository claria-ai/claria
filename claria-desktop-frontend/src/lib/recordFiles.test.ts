import { describe, expect, it } from "vitest";
import {
  AUDIO_EXTENSIONS,
  CHAT_HISTORY_PREFIX,
  basename,
  chatHistoryLabel,
  chatIdFromFilename,
  fileExtension,
  isAudioSidecar,
  isChatHistory,
  versionKeyFor,
} from "./recordFiles";

describe("fileExtension", () => {
  it.each([
    ["notes.txt", "txt"],
    ["Session.M4A", "m4a"],
    ["archive.tar.gz", "gz"],
    ["README", "readme"],
    ["", ""],
  ])("reads %s as %s", (filename, expected) => {
    expect(fileExtension(filename)).toBe(expected);
  });

  it("treats a dotfile's name as its extension", () => {
    // Documenting the behaviour rather than endorsing it: record filenames
    // are user-supplied and a leading dot is not special-cased anywhere.
    expect(fileExtension(".m4a")).toBe("m4a");
  });
});

describe("isAudioSidecar", () => {
  it.each([...AUDIO_EXTENSIONS])("accepts .%s", (ext) => {
    expect(isAudioSidecar(`session.${ext}`)).toBe(true);
  });

  it("is case-insensitive", () => {
    expect(isAudioSidecar("Session.M4A")).toBe(true);
  });

  it.each(["intake.txt", "report.pdf", "letter.docx", "notes", "a.mp3.txt"])(
    "rejects %s",
    (filename) => {
      expect(isAudioSidecar(filename)).toBe(false);
    },
  );

  it("gates on the audio extension, not on a .text suffix", () => {
    // The preview modal is opened with the audio filename; the backend
    // appends `.text` itself. A sidecar name must not take the audio path.
    expect(isAudioSidecar("session.m4a")).toBe(true);
    expect(isAudioSidecar("session.m4a.text")).toBe(false);
  });
});

describe("versionKeyFor", () => {
  it("points audio files at their transcript sidecar", () => {
    expect(versionKeyFor("session.m4a")).toBe("session.m4a.text");
  });

  it("leaves non-audio files alone", () => {
    expect(versionKeyFor("intake.txt")).toBe("intake.txt");
    expect(versionKeyFor("report.pdf")).toBe("report.pdf");
  });

  it("is idempotent for a sidecar name", () => {
    expect(versionKeyFor(versionKeyFor("session.m4a"))).toBe(
      "session.m4a.text",
    );
  });
});

describe("chat history filenames", () => {
  const key = `${CHAT_HISTORY_PREFIX}0f8b3c1e-1234-4a5b-9c8d-ffeeddccbbaa.json`;

  it("recognises the prefix", () => {
    expect(isChatHistory(key)).toBe(true);
    expect(isChatHistory("intake.txt")).toBe(false);
  });

  it("recovers the chat id", () => {
    expect(chatIdFromFilename(key)).toBe(
      "0f8b3c1e-1234-4a5b-9c8d-ffeeddccbbaa",
    );
  });

  it("truncates the label to eight characters", () => {
    expect(chatHistoryLabel(key)).toBe("0f8b3c1e...");
  });

  it("leaves a short id untruncated", () => {
    expect(chatHistoryLabel(`${CHAT_HISTORY_PREFIX}abc.json`)).toBe("abc");
  });

  it("leaves an exactly-eight-character id untruncated", () => {
    expect(chatHistoryLabel(`${CHAT_HISTORY_PREFIX}12345678.json`)).toBe(
      "12345678",
    );
  });
});

describe("basename", () => {
  it("takes the last path segment", () => {
    expect(basename("/Users/x/Documents/session.m4a")).toBe("session.m4a");
  });

  it("handles Windows paths", () => {
    expect(basename("C:\\Users\\x\\Documents\\session.m4a")).toBe("session.m4a");
  });

  it("returns a bare filename unchanged", () => {
    expect(basename("session.m4a")).toBe("session.m4a");
  });
});
