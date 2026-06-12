//! Spoken-entity formatting for dictation — deterministic, conservative.
//!
//! People dictate numbers and times in words: "twenty five dollars", "three
//! pm", "ten o'clock". The written forms ("$25", "3 PM", "10 o'clock") read
//! better, and a deterministic pass produces them reliably — no model, no
//! guesswork. Like the voice-command grammar, the whole design is built to
//! NEVER misfire on ordinary speech:
//!
//! 1. **Anchor-required.** A number is only reformatted when it sits directly
//!    next to a strong unit anchor — `dollars`/`cents`, `am`/`pm`/`o'clock`.
//!    Bare numbers are never touched, so "two hundred K tokens", "sixty
//!    minutes", "bump the score to eighty" pass through unchanged.
//! 2. **Presence-conditional → never invention.** It only rewrites words that
//!    were actually spoken (it reformats "twenty five dollars" that is THERE
//!    into "$25"); it never adds a figure that wasn't said. The output is only
//!    digits / `$` / `:` / separated `AM`-`PM`, which are neither fact words
//!    (colours/days/months) nor identifier-shaped tokens, so it can't trip the
//!    no-invention guards (`introduced_fact_word`, `invented_technical_token`).
//! 3. **Byte-identical no-op when nothing matches**, and idempotent (running
//!    it on already-formatted text returns the same text).
//!
//! Scope is deliberately the highest-confidence, lowest-ambiguity subset
//! (currency + clock times). Spoken emails/URLs ("name at host dot com") and
//! "twenty"-prefixed years are intentionally OUT — "at"/"dot"/"twenty" are too
//! common in prose to disambiguate without a model, exactly the ambiguous
//! cases the plan says to skip.

/// A unit anchor that licenses reformatting the number before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Dollars,
    Cents,
    Am,
    Pm,
    Oclock,
    None,
}

/// One number/unit atom plus the source word-span it came from. A hyphenated
/// word ("twenty-five") splits into several atoms that all point at the one
/// span, so the byte-range replacement still covers the whole written word.
struct Atom {
    core: String,
    span: usize,
}

/// Lowercased alphanumeric "squash" — for matching unit words regardless of
/// internal punctuation ("a.m." → `am`, "o'clock" → `oclock`).
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// The core of a word: lowercased, with leading/trailing non-alphanumerics
/// trimmed but internal structure ("twenty-five", "a.m") kept.
fn core_lower(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_lowercase()
}

fn unit_kind(core: &str) -> Unit {
    match squash(core).as_str() {
        "dollars" | "dollar" => Unit::Dollars,
        "cents" | "cent" => Unit::Cents,
        "am" => Unit::Am,
        "pm" => Unit::Pm,
        "oclock" => Unit::Oclock,
        _ => Unit::None,
    }
}

/// 0–19, or `None`.
fn ones_teens(w: &str) -> Option<u64> {
    Some(match w {
        "zero" | "oh" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        _ => return None,
    })
}

/// 20–90 by ten, or `None`.
fn tens(w: &str) -> Option<u64> {
    Some(match w {
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

fn is_number_word(core: &str) -> bool {
    !core.is_empty() && core.chars().all(|c| c.is_ascii_digit())
        || ones_teens(core).is_some()
        || tens(core).is_some()
        || core == "hundred"
        || core == "thousand"
}

/// Parse a spoken cardinal from the front of `atoms` (e.g. "twenty five" → 25,
/// "two thousand five hundred" → 2500, "500" → 500). Returns the value and how
/// many atoms it consumed, or `None` if the first atom isn't a number.
fn parse_cardinal(atoms: &[Atom]) -> Option<(u64, usize)> {
    if let Some(first) = atoms.first() {
        if !first.core.is_empty() && first.core.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(v) = first.core.parse::<u64>() {
                return Some((v, 1));
            }
        }
    }
    let mut total = 0u64;
    let mut current = 0u64;
    let mut any = false;
    let mut consumed = 0;
    for (k, atom) in atoms.iter().enumerate() {
        let w = atom.core.as_str();
        if let Some(v) = ones_teens(w) {
            current += v;
            any = true;
        } else if let Some(v) = tens(w) {
            current += v;
            any = true;
        } else if w == "hundred" {
            current = if current == 0 { 100 } else { current * 100 };
            any = true;
        } else if w == "thousand" {
            total += if current == 0 { 1000 } else { current * 1000 };
            current = 0;
            any = true;
        } else if w == "and" && any && atoms.get(k + 1).is_some_and(|a| is_number_word(&a.core)) {
            // "two hundred and five" — skip the connective, keep parsing.
        } else {
            break;
        }
        consumed = k + 1;
    }
    if !any {
        return None;
    }
    Some((total + current, consumed))
}

/// A single hour word/digit, 1–12, or `None` (rejects tens/`hundred` so a
/// price like "twenty dollars" can't be read as an hour).
fn single_hour(core: &str) -> Option<u64> {
    let v = if !core.is_empty() && core.chars().all(|c| c.is_ascii_digit()) {
        core.parse::<u64>().ok()?
    } else {
        ones_teens(core)?
    };
    (1..=12).contains(&v).then_some(v)
}

/// Currency: `<cardinal> dollars [and <cardinal> cents]` → `$N` / `$N.CC`.
/// Returns the LAST atom index it consumed and the replacement text.
fn match_currency(atoms: &[Atom], a: usize) -> Option<(usize, String)> {
    let (amount, na) = parse_cardinal(&atoms[a..])?;
    let unit = a + na;
    if unit_kind(&atoms.get(unit)?.core) != Unit::Dollars {
        return None;
    }
    let mut last = unit;
    let mut repl = format!("${amount}");
    // Optional "... and N cents".
    if atoms.get(unit + 1).is_some_and(|x| x.core == "and") {
        if let Some((cents, cn)) = parse_cardinal(&atoms[unit + 2..]) {
            let ci = unit + 2 + cn;
            if cents <= 99 && unit_kind(&atoms.get(ci)?.core) == Unit::Cents {
                repl = format!("${amount}.{cents:02}");
                last = ci;
            }
        }
    }
    Some((last, repl))
}

/// Clock: `<hour> [<minute>] am|pm` → `H[:MM] AM|PM`, or `<hour> o'clock` →
/// `H o'clock`. The am/pm/o'clock anchor is required.
fn match_time(atoms: &[Atom], a: usize) -> Option<(usize, String)> {
    let hour = single_hour(&atoms.get(a)?.core)?;
    // Optional minute, but ONLY when it is immediately followed by am/pm
    // (otherwise "three thirty meeting" is not a time).
    let mut minute = None;
    let mut unit = a + 1;
    if let Some((m, c)) = parse_cardinal(&atoms[(a + 1).min(atoms.len())..]) {
        let after = a + 1 + c;
        if m <= 59 && matches!(unit_kind(&atoms.get(after)?.core), Unit::Am | Unit::Pm) {
            minute = Some(m);
            unit = after;
        }
    }
    match unit_kind(&atoms.get(unit)?.core) {
        Unit::Am | Unit::Pm => {
            let mer = if unit_kind(&atoms[unit].core) == Unit::Am {
                "AM"
            } else {
                "PM"
            };
            let repl = match minute {
                Some(m) => format!("{hour}:{m:02} {mer}"),
                None => format!("{hour} {mer}"),
            };
            Some((unit, repl))
        }
        // o'clock takes no minutes.
        Unit::Oclock if minute.is_none() => Some((unit, format!("{hour} o'clock"))),
        _ => None,
    }
}

/// Byte ranges of the whitespace-separated words in `text`.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                spans.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    spans
}

fn leading_punct(word: &str) -> String {
    word.chars()
        .take_while(|c| !c.is_ascii_alphanumeric())
        .collect()
}

fn trailing_punct(word: &str) -> String {
    let trailing: String = word
        .chars()
        .rev()
        .take_while(|c| !c.is_ascii_alphanumeric())
        .collect();
    trailing.chars().rev().collect()
}

/// Reformat spoken currency and clock times into their written forms. Returns
/// the input unchanged when no anchored pattern is present (byte-identical).
pub fn normalize_entities(text: &str) -> String {
    let spans = word_spans(text);
    if spans.is_empty() {
        return text.to_string();
    }
    let words: Vec<&str> = spans.iter().map(|&(s, e)| &text[s..e]).collect();
    // Split each word's core on '-' into atoms tagged with their span.
    let mut atoms: Vec<Atom> = Vec::new();
    for (si, w) in words.iter().enumerate() {
        for part in core_lower(w).split('-') {
            if !part.is_empty() {
                atoms.push(Atom {
                    core: part.to_string(),
                    span: si,
                });
            }
        }
    }

    // (start_byte, end_byte, replacement) — non-overlapping, left to right.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i < atoms.len() {
        if let Some((last, repl)) = match_currency(&atoms, i).or_else(|| match_time(&atoms, i)) {
            let first_span = atoms[i].span;
            let last_span = atoms[last].span;
            let lead = leading_punct(words[first_span]);
            let trail = trailing_punct(words[last_span]);
            edits.push((
                spans[first_span].0,
                spans[last_span].1,
                format!("{lead}{repl}{trail}"),
            ));
            i = last + 1;
        } else {
            i += 1;
        }
    }

    if edits.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for (start, end, repl) in edits {
        out.push_str(&text[pos..start]);
        out.push_str(&repl);
        pos = end;
    }
    out.push_str(&text[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::normalize_entities;

    #[test]
    fn currency_words_and_digits() {
        assert_eq!(
            normalize_entities("it costs twenty five dollars"),
            "it costs $25"
        );
        assert_eq!(
            normalize_entities("anything over five hundred dollars"),
            "anything over $500"
        );
        assert_eq!(normalize_entities("just 25 dollars"), "just $25");
        assert_eq!(normalize_entities("one dollar please"), "$1 please");
        assert_eq!(normalize_entities("five dollars and fifty cents"), "$5.50");
        assert_eq!(
            normalize_entities("two thousand five hundred dollars"),
            "$2500"
        );
    }

    #[test]
    fn clock_times() {
        assert_eq!(normalize_entities("meet at three pm"), "meet at 3 PM");
        assert_eq!(
            normalize_entities("call at three thirty pm"),
            "call at 3:30 PM"
        );
        assert_eq!(
            normalize_entities("by eleven am tomorrow"),
            "by 11 AM tomorrow"
        );
        assert_eq!(normalize_entities("starts at 3 pm"), "starts at 3 PM");
        assert_eq!(
            normalize_entities("the ten o'clock standup"),
            "the 10 o'clock standup"
        );
        // Minute that isn't followed by am/pm is not a time.
        assert_eq!(
            normalize_entities("three thirty meeting"),
            "three thirty meeting"
        );
    }

    #[test]
    fn flagship_examples_from_the_plan() {
        assert_eq!(normalize_entities("three pm friday"), "3 PM friday");
        assert_eq!(normalize_entities("twenty five dollars"), "$25");
    }

    #[test]
    fn punctuation_is_preserved() {
        assert_eq!(normalize_entities("it was twenty dollars."), "it was $20.");
        assert_eq!(
            normalize_entities("meet at three pm, then lunch"),
            "meet at 3 PM, then lunch"
        );
        assert_eq!(normalize_entities("pm. note"), "pm. note"); // no number → no fire
    }

    #[test]
    fn hyphenated_number_words() {
        assert_eq!(normalize_entities("twenty-five dollars"), "$25");
    }

    #[test]
    fn bare_numbers_without_an_anchor_are_untouched() {
        for s in [
            "increase the context window to two hundred K tokens",
            "bump the TTL to sixty minutes",
            "the purchase order for forty units and mark it net thirty",
            "bump its lead score to eighty",
            "step one then step two",
            "the meeting is on the fifteenth",
        ] {
            assert_eq!(normalize_entities(s), s, "must not fire on: {s}");
        }
    }

    #[test]
    fn no_match_is_byte_identical_and_idempotent() {
        let plain = "just a normal sentence with no figures in it";
        assert_eq!(normalize_entities(plain), plain);
        // Idempotent: a formatted result re-normalizes to itself.
        let once = normalize_entities("twenty five dollars at three thirty pm");
        assert_eq!(once, "$25 at 3:30 PM");
        assert_eq!(normalize_entities(&once), once);
    }

    #[test]
    fn output_does_not_trip_the_no_invention_guards() {
        // The whole point of the digits/$/AM-PM output: it must survive the
        // rewrite guards that compare output against the spoken input.
        let input = "it was twenty five dollars at three pm";
        let output = normalize_entities(input); // "it was $25 at 3 PM"
        assert!(
            crate::dictation::introduced_fact_word(&output, input).is_none(),
            "entity output tripped the fact-word guard"
        );
        assert!(
            crate::dictation::invented_technical_token(&output, input, &[]).is_none(),
            "entity output tripped the technical-token guard"
        );
    }

    #[test]
    fn corpus_only_touches_legitimate_currency() {
        // Across the realism corpus, the ONLY change entity formatting makes is
        // the one true currency mention (j15: "five hundred dollars" → "$500").
        // Every other file — full of bare numbers, ordinals, and technical
        // content — is left byte-identical.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/probe-afm/transcripts");
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read transcripts dir {}: {e}", dir.display()));
        let mut changed = Vec::new();
        let mut checked = 0;
        for entry in entries {
            let path = entry.expect("dir entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if !(name.starts_with('j') || name.starts_with('t')) || !name.ends_with(".txt") {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read transcript");
            let normalized = normalize_entities(&content);
            if normalized != content {
                changed.push(name.clone());
                if name.starts_with("j15") {
                    assert!(normalized.contains("$500"), "j15 should produce $500");
                    assert!(!normalized.contains("five hundred dollars"));
                }
            }
            checked += 1;
        }
        assert!(
            checked >= 30,
            "expected the full corpus, only saw {checked} files"
        );
        assert_eq!(
            changed,
            vec!["j15-biz-qb-mangled.txt".to_string()],
            "entity formatting changed an unexpected corpus file"
        );
    }
}
