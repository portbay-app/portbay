//! Inline voice commands for dictation — deterministic, false-trigger-proof.
//!
//! While dictating, people say structural commands mid-flow: "new line",
//! "new paragraph", "bullet point" / "new point", "steps" / "next step",
//! "scratch that". Competitors (Wispr Flow, superwhisper) act on these;
//! PortBay does too — but the hard part is
//! *awareness*: the parser must NEVER fire on dictated CONTENT that merely
//! contains a command word ("add a new line to the config", "don't scratch
//! that surface", "send the email to John").
//!
//! The design that makes false triggers structurally impossible:
//!
//! 1. **Deterministic grammar, not LLM intent.** A command is recognized by
//!    exact match, never by a model "deciding" what the user meant. It fits
//!    the same probe discipline as the rest of the dictation engine and can't
//!    hallucinate an action.
//! 2. **Whole-clause anchoring.** A command fires ONLY when it stands alone as
//!    an entire utterance unit — a clause bounded by sentence punctuation
//!    (`. ! ? ;`), a newline, or the string ends. "New paragraph." fires;
//!    "Add a new line to the file." does not, because the clause is the whole
//!    sentence, not just the command. (Run-on speech with no boundaries — the
//!    raw macOS-dictation style — therefore never fires anything, which is the
//!    conservative default the no-invention bias demands.)
//! 3. **Byte-identical no-op when nothing fires.** If no clause is exactly a
//!    command, the input is returned UNCHANGED. Normal dictation is never
//!    perturbed, and the regression corpus (`scripts/probe-afm/transcripts/`,
//!    `j01`–`j18` + `t1`–`t21`) is guaranteed untouched — the acceptance bar.
//!
//! The formatting commands run as a PRE-PASS before the rewrite (so the
//! structural markers compose with `build_prompt`'s layout rules rather than
//! fighting them); `scratch that` removes the preceding sentence from the
//! transcript text. Both are pure string transforms — no app/field dispatch —
//! so they apply identically on the raw-paste and polished paths.

use crate::dictation::squash;

/// A recognized inline command. Deliberately small and high-value — the set
/// grows only behind the same zero-false-trigger gate (commands-as-content
/// pairs in `transcripts/cmd-*`, the corpus regression test below).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    /// A single line break.
    NewLine,
    /// A blank line — a paragraph break. Also resets the numbered-list count.
    NewParagraph,
    /// Start a bulleted line (`- `).
    Bullet,
    /// Start the next numbered line (`1. `, `2. `, …) — "steps", "next step".
    NumberedItem,
    /// Remove the sentence just dictated (generalizes the rewrite's spoken
    /// self-correction to a live command).
    ScratchPrevious,
}

/// Strong clause terminators — the only boundaries a command may sit between.
/// Deliberately excludes the comma: comma-bounded "okay, send, now" is too
/// loose a boundary and would widen the false-trigger surface.
const TERMINATORS: [char; 5] = ['.', '!', '?', ';', '\n'];

fn is_terminator(c: char) -> bool {
    TERMINATORS.contains(&c)
}

/// Map a squashed standalone clause to a command, if it is EXACTLY one. The
/// clause is `squash`ed first (lowercase, alphanumerics only) so spacing and
/// punctuation variants collapse ("New Line" / "new-line" → `newline`); the
/// match is then exact, so a command only ever resolves when the clause is
/// nothing but the command phrase.
fn command_for(squashed_clause: &str) -> Option<Command> {
    match squashed_clause {
        "newline" | "linebreak" => Some(Command::NewLine),
        "newparagraph" | "paragraphbreak" => Some(Command::NewParagraph),
        "bullet" | "bulletpoint" | "newbullet" | "nextbullet" | "newpoint" | "nextpoint"
        | "point" => Some(Command::Bullet),
        "step" | "steps" | "nextstep" | "newstep" | "numberedlist" | "number" | "nextnumber" => {
            Some(Command::NumberedItem)
        }
        "scratchthat"
        | "deletethat"
        | "removethat"
        | "scratchthelastsentence"
        | "deletethelastsentence" => Some(Command::ScratchPrevious),
        _ => None,
    }
}

/// One parsed clause: its trimmed text and the terminator that closed it
/// (`None` at end of input).
struct Clause<'a> {
    text: &'a str,
    term: Option<char>,
}

/// Split input into clauses at the strong terminators, keeping each
/// terminator with the clause it closed so content punctuation survives
/// reconstruction.
fn split_clauses(input: &str) -> Vec<Clause<'_>> {
    let mut clauses = Vec::new();
    let mut start = 0;
    for (i, c) in input.char_indices() {
        if TERMINATORS.contains(&c) {
            clauses.push(Clause {
                text: &input[start..i],
                term: Some(c),
            });
            start = i + c.len_utf8();
        }
    }
    if start < input.len() {
        clauses.push(Clause {
            text: &input[start..],
            term: None,
        });
    }
    clauses
}

/// Trailing inline whitespace (spaces/tabs) only — never newlines, which
/// carry structural meaning here.
fn trim_inline_trailing(out: &mut String) {
    while out.ends_with(' ') || out.ends_with('\t') {
        out.pop();
    }
}

/// "Scratch that": drop the most recent sentence already emitted. Trims
/// trailing whitespace, drops the scratched sentence's OWN terminator (so the
/// search lands on the boundary before it, not its own period), removes back
/// to that boundary (or the start), then tidies the new tail.
fn scratch_previous(out: &mut String) {
    let end = out.trim_end().len();
    out.truncate(end);
    while out.chars().next_back().is_some_and(is_terminator) {
        out.pop();
    }
    let cut = out
        .rfind(is_terminator)
        .map(|i| i + 1) // terminators are all single-byte ASCII
        .unwrap_or(0);
    out.truncate(cut);
    let end = out.trim_end().len();
    out.truncate(end);
}

/// Apply inline voice commands to a dictated transcript. Returns the input
/// unchanged when no clause is exactly a command (the overwhelming common
/// case). See the module docs for the anchoring/no-false-trigger contract.
pub fn apply_voice_commands(input: &str) -> String {
    enum Item<'a> {
        Content(&'a str, Option<char>),
        Cmd(Command),
    }

    let mut items = Vec::new();
    let mut any_command = false;
    for clause in split_clauses(input) {
        let trimmed = clause.text.trim();
        if trimmed.is_empty() {
            // A bare terminator / whitespace clause carries nothing; dropping
            // it only matters once some other clause is a command (otherwise
            // we return the input verbatim below).
            continue;
        }
        match command_for(&squash(trimmed)) {
            Some(cmd) => {
                any_command = true;
                items.push(Item::Cmd(cmd));
            }
            None => items.push(Item::Content(trimmed, clause.term)),
        }
    }

    // No command anywhere → guarantee a byte-identical no-op. This is the
    // safety property the corpus regression test pins.
    if !any_command {
        return input.to_string();
    }

    let mut out = String::new();
    // Running count for numbered-list items; a paragraph break starts a new
    // list context (back to 1).
    let mut number = 0u32;
    for item in items {
        match item {
            Item::Content(text, term) => {
                if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(text);
                // Keep real sentence punctuation; a newline terminator becomes
                // an ordinary clause break (the command markers own layout).
                match term {
                    Some('\n') | None => {}
                    Some(t) => out.push(t),
                }
            }
            Item::Cmd(Command::NewLine) => {
                trim_inline_trailing(&mut out);
                out.push('\n');
            }
            Item::Cmd(Command::NewParagraph) => {
                number = 0;
                trim_inline_trailing(&mut out);
                out.push_str("\n\n");
            }
            Item::Cmd(Command::Bullet) => {
                trim_inline_trailing(&mut out);
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("- ");
            }
            Item::Cmd(Command::NumberedItem) => {
                number += 1;
                trim_inline_trailing(&mut out);
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&format!("{number}. "));
            }
            Item::Cmd(Command::ScratchPrevious) => scratch_previous(&mut out),
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::apply_voice_commands;

    // --- Commands fire when they stand alone as a clause ------------------

    #[test]
    fn new_line_and_paragraph_fire_as_standalone_clauses() {
        assert_eq!(
            apply_voice_commands("Add the import at the top. New line. Then call init."),
            "Add the import at the top.\nThen call init."
        );
        assert_eq!(
            apply_voice_commands("First section done. New paragraph. Second section starts."),
            "First section done.\n\nSecond section starts."
        );
    }

    #[test]
    fn bullet_starts_a_bulleted_line() {
        assert_eq!(
            apply_voice_commands(
                "Here are the items. Bullet point. Install deps. Bullet point. Run migrations."
            ),
            "Here are the items.\n- Install deps.\n- Run migrations."
        );
        // "New point" / "next point" are bullet synonyms.
        assert_eq!(
            apply_voice_commands("Recap. New point. Faster boot. Next point. Smaller binary."),
            "Recap.\n- Faster boot.\n- Smaller binary."
        );
    }

    #[test]
    fn numbered_items_count_up_and_reset_on_paragraph() {
        assert_eq!(
            apply_voice_commands(
                "Steps. Install the deps. Next step. Run the migrations. Next step. Seed the data."
            ),
            "1. Install the deps.\n2. Run the migrations.\n3. Seed the data."
        );
        // A paragraph break starts a fresh numbered context.
        assert_eq!(
            apply_voice_commands("Steps. First thing. New paragraph. Steps. Other thing."),
            "1. First thing.\n\n1. Other thing."
        );
    }

    #[test]
    fn scratch_that_removes_the_previous_sentence() {
        assert_eq!(
            apply_voice_commands("The meeting is Tuesday. Scratch that. The meeting is Wednesday."),
            "The meeting is Wednesday."
        );
        // Delete-that is an accepted synonym.
        assert_eq!(
            apply_voice_commands("Ship it Friday. Delete that. Ship it Monday."),
            "Ship it Monday."
        );
    }

    // --- Commands do NOT fire when embedded in content (the hard part) ----

    #[test]
    fn command_words_in_content_never_fire() {
        // Each command word, embedded in a real sentence, must pass through
        // byte-identical.
        for s in [
            "Add a new line to the config file and save it.",
            "We need a new paragraph in the contract about liability.",
            "The bullet point on slide three is wrong.",
            "Please don't scratch that surface with the tool.",
            "Send the email to John about the deploy.",
            "Push the feature branch and open a pull request.",
            // The common-word commands ("steps", "point", "number") embedded
            // in real sentences must stay content — the clause-exact match is
            // what makes this safe.
            "The next step is to deploy the worker.",
            "That is a fair point about the latency.",
            "The phone number is already on file.",
            "Outline the steps before you start coding.",
        ] {
            assert_eq!(apply_voice_commands(s), s, "must not fire on: {s}");
        }
    }

    #[test]
    fn run_on_speech_with_no_boundaries_is_untouched() {
        // The raw macOS-dictation style: no terminators, so the whole thing is
        // one clause and nothing can match — returned verbatim.
        let s = "okay so for the deploy first update the readme then push the release \
                 and also add a new line where the imports are and send it";
        assert_eq!(apply_voice_commands(s), s);
    }

    #[test]
    fn empty_and_no_command_inputs_are_identity() {
        assert_eq!(apply_voice_commands(""), "");
        let s = "Just a normal sentence. And another one.";
        assert_eq!(apply_voice_commands(s), s);
    }

    // --- The realism bar: zero spurious firing across the probe corpus ----

    #[test]
    fn corpus_never_triggers_a_command() {
        // Every real dictation in the probe corpus (j01–j18 + t1–t21,
        // including the "cancel that" self-correction transcripts) must pass
        // through unchanged — no command may be extracted from real content.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/probe-afm/transcripts");
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read transcripts dir {}: {e}", dir.display()));
        let mut checked = 0;
        for entry in entries {
            let path = entry.expect("dir entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            // The corpus proper (j*/t*); the cmd-* fixtures are asserted
            // separately (they intentionally do/don't fire).
            if !(name.starts_with('j') || name.starts_with('t')) || !name.ends_with(".txt") {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read transcript");
            assert_eq!(
                apply_voice_commands(&content),
                content,
                "spurious command extracted from corpus file {name}"
            );
            checked += 1;
        }
        // Guard against the test silently passing because it found nothing.
        assert!(
            checked >= 30,
            "expected the full corpus, only saw {checked} files"
        );
    }

    #[test]
    fn cmd_content_fixtures_never_fire() {
        // The "commands-as-content" half of the cmd-* pairs: command words
        // inside real sentences, which must pass through unchanged.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/probe-afm/transcripts");
        for name in [
            "cmd-newline-content.txt",
            "cmd-scratch-content.txt",
            "cmd-bullet-content.txt",
            "cmd-send-content.txt",
        ] {
            let content = std::fs::read_to_string(dir.join(name))
                .unwrap_or_else(|e| panic!("read fixture {name}: {e}"));
            assert_eq!(
                apply_voice_commands(&content),
                content,
                "content fixture {name} wrongly fired a command"
            );
        }
    }
}
