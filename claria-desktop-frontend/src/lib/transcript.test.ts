import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { formatTime, mmSsToSeconds, parseBody, renderBody } from "./transcript";

// The same files `crates/claria-transcribe/tests/body_format.rs` reads. See
// `fixtures/transcript-body/README.md` for the schema and the reasoning.
const FIXTURE_DIR = join(
  dirname(fileURLToPath(import.meta.url)),
  "../../../fixtures/transcript-body"
);

interface ExpectedSegment {
  id: string;
  speaker_label: string | null;
  language_code: string | null;
  start_seconds: number;
  end_seconds: number;
  text: string;
  translation: string | null;
}

interface ExpectedBody {
  description: string;
  speaker_labels: string[];
  segments: ExpectedSegment[];
  rendered: string;
}

function fixtureNames(): string[] {
  return readdirSync(FIXTURE_DIR)
    .filter((f) => f.endsWith(".txt"))
    .map((f) => f.slice(0, -".txt".length))
    .sort();
}

function loadFixture(name: string): { body: string; expected: ExpectedBody } {
  const body = readFileSync(join(FIXTURE_DIR, `${name}.txt`), "utf8");
  const expected = JSON.parse(
    readFileSync(join(FIXTURE_DIR, `${name}.expected.json`), "utf8")
  ) as ExpectedBody;
  return { body, expected };
}

describe("shared transcript fixtures", () => {
  const names = fixtureNames();

  it("finds the shared fixture directory", () => {
    // A rename or a moved directory would otherwise silently reduce this
    // whole suite to zero assertions.
    expect(names.length).toBeGreaterThan(0);
  });

  describe.each(names)("%s", (name) => {
    const { body, expected } = loadFixture(name);

    it("parses to the agreed segments", () => {
      const parsed = parseBody(body);
      const asExpected: ExpectedSegment[] = parsed.segments.map((s) => ({
        id: s.id,
        speaker_label: s.speakerKey,
        language_code: s.languageCode,
        start_seconds: s.startSeconds,
        end_seconds: s.endSeconds,
        text: s.text,
        translation: s.translation,
      }));
      expect(asExpected).toEqual(expected.segments);
    });

    it("interns the agreed speaker labels", () => {
      const parsed = parseBody(body);
      expect([...parsed.labelByKey.keys()]).toEqual(expected.speaker_labels);
    });

    it("renders back to the agreed body", () => {
      const parsed = parseBody(body);
      expect(renderBody(parsed.segments, parsed.labelByKey)).toBe(
        expected.rendered
      );
    });

    it("is stable under a second round trip", () => {
      // Rendering is a normalisation, so applying it twice must not keep
      // moving. An editor saves on every visit; drift here compounds.
      const once = roundTrip(body);
      expect(roundTrip(once)).toBe(once);
    });
  });
});

function roundTrip(body: string): string {
  const parsed = parseBody(body);
  return renderBody(parsed.segments, parsed.labelByKey);
}

describe("speaker renames", () => {
  it("rewrites every header carrying the renamed label", () => {
    const { body } = loadFixture("shared_speaker_label");
    const parsed = parseBody(body);
    parsed.labelByKey.set("Clinician", "Dr. Okafor");
    const rendered = renderBody(parsed.segments, parsed.labelByKey);
    expect(rendered).toContain("[Dr. Okafor 00:00–00:04]");
    expect(rendered).toContain("[Dr. Okafor 00:10–00:14]");
    expect(rendered).not.toContain("Clinician");
  });

  it("leaves other speakers alone", () => {
    const { body } = loadFixture("diarized_two_speakers");
    const parsed = parseBody(body);
    parsed.labelByKey.set("Clinician", "Dr. Okafor");
    const rendered = renderBody(parsed.segments, parsed.labelByKey);
    expect(rendered).toContain("[Dr. Okafor 00:00–00:04]");
    expect(rendered).toContain("[Patient 00:04–00:09]");
  });
});

describe("time formatting", () => {
  it("pads to two digits below the hour", () => {
    expect(formatTime(0)).toBe("00:00");
    expect(formatTime(9)).toBe("00:09");
    expect(formatTime(605)).toBe("10:05");
  });

  it("lets the minute field grow past two digits", () => {
    expect(formatTime(6000)).toBe("100:00");
    expect(formatTime(6090)).toBe("101:30");
  });

  it("round-trips through mm:ss", () => {
    for (const seconds of [0, 4, 59, 60, 605, 5999, 6000, 6090]) {
      expect(mmSsToSeconds(formatTime(seconds))).toBe(seconds);
    }
  });
});
