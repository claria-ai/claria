//! Delta pacing: what the reader sees, and when, for each cadence.

use claria_bedrock::pacing::{DeltaPacer, StreamPacing};

/// Drive a pacer with `deltas` and collect everything it released, ending
/// with the final flush.
fn paced(pacing: StreamPacing, deltas: &[&str]) -> Vec<String> {
    let mut pacer = DeltaPacer::new(pacing);
    let mut chunks: Vec<String> = deltas.iter().filter_map(|d| pacer.push(d)).collect();
    chunks.extend(pacer.flush());
    chunks
}

#[test]
fn token_pacing_forwards_every_delta_untouched() {
    assert_eq!(
        paced(StreamPacing::Token, &["Hel", "lo, ", "world."]),
        vec!["Hel", "lo, ", "world."]
    );
}

#[test]
fn token_pacing_skips_empty_deltas() {
    assert_eq!(paced(StreamPacing::Token, &["", "a", ""]), vec!["a"]);
}

#[test]
fn paragraph_pacing_holds_text_until_the_paragraph_closes() {
    let chunks = paced(
        StreamPacing::Paragraph,
        &[
            "The first ",
            "paragraph.",
            "\n",
            "\nThe second ",
            "paragraph.",
        ],
    );
    assert_eq!(
        chunks,
        vec!["The first paragraph.\n\n", "The second paragraph."]
    );
}

/// A paragraph break split across two deltas is still a break — the pacer
/// looks at accumulated text, not at delta boundaries.
#[test]
fn paragraph_pacing_releases_at_the_last_break_it_can_see() {
    let chunks = paced(StreamPacing::Paragraph, &["one\n\ntwo\n\nthree"]);
    assert_eq!(chunks, vec!["one\n\ntwo\n\n", "three"]);
}

/// Prose that never closes a paragraph — a long single-paragraph answer, a
/// fenced code block, a list joined by single newlines — must not be held
/// back until the turn ends.
#[test]
fn paragraph_pacing_releases_text_that_never_closes_a_paragraph() {
    let long = "word ".repeat(2_000);
    // Wire-sized deltas, the way the service actually produces them.
    let deltas: Vec<&str> = (0..long.len().div_ceil(20))
        .map(|index| &long[index * 20..((index + 1) * 20).min(long.len())])
        .collect();

    let chunks = paced(StreamPacing::Paragraph, &deltas);
    assert!(
        chunks.len() > 1,
        "an unbroken run was held whole: {} chunk(s)",
        chunks.len()
    );
    assert_eq!(chunks.concat(), long, "text was lost or reordered");
    assert!(
        chunks[..chunks.len() - 1]
            .iter()
            .all(|chunk| chunk.ends_with(' ')),
        "a chunk broke mid-word"
    );
}

/// Every byte pushed in comes back out exactly once, in order. The live
/// bubble and the persisted reply have to agree character for character.
#[test]
fn paragraph_pacing_preserves_the_text_exactly() {
    let deltas = [
        "Assessment\n\n",
        "The client reports ",
        "improved sleep.\n\n- one\n- two\n",
        "\nSigned.",
    ];
    let chunks = paced(StreamPacing::Paragraph, &deltas);
    assert_eq!(chunks.concat(), deltas.concat());
}

#[test]
fn paragraph_pacing_emits_nothing_for_an_empty_stream() {
    assert!(paced(StreamPacing::Paragraph, &[]).is_empty());
}

/// The reader asked for no incremental output at all: the completed reply is
/// the only delivery, so nothing reaches the delta callback — not even on
/// the final flush.
#[test]
fn off_pacing_emits_nothing() {
    assert!(paced(StreamPacing::Off, &["one\n\ntwo", " three"]).is_empty());
}

#[test]
fn paragraph_is_the_default_cadence() {
    assert_eq!(StreamPacing::default(), StreamPacing::Paragraph);
}
