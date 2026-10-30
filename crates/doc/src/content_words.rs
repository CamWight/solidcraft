//! The words a drawing puts on its paper, in the drawing's own language (`Drawing::language`). They
//! belong to the drawing, so the same drawing gives the same bytes under any interface language, and
//! the interface's language never reaches this table.

/// A word a drawing's content may print.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    /// After a dimension that is an upper limit.
    Max,
    /// After a dimension that is a lower limit.
    Min,
}

/// A word as a drawing prints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Printed {
    pub text: &'static str,
    /// False when the language has no word here and this is the English one, which a reply says.
    pub in_language: bool,
}

/// Each language with words of its own, by its primary subtag, with the English row first.
const WORDS: &[(&str, &[(Word, &str)])] = &[("en", &[(Word::Max, "MAX"), (Word::Min, "MIN")]), ("de", &[(Word::Max, "MAX"), (Word::Min, "MIN")])];

/// `word` in `language`, a language tag read by its primary subtag: English when the language has no
/// such word, or the tag no language.
pub fn word(language: &str, word: Word) -> Printed {
    let primary = language.split('-').next().unwrap_or_default().to_ascii_lowercase();
    let row = |subtag: &str| WORDS.iter().find(|(language, _)| *language == subtag).and_then(|(_, row)| row.iter().find(|(w, _)| *w == word));
    match row(&primary) {
        Some((_, text)) => Printed { text, in_language: true },
        None => Printed { text: row("en").map_or("", |(_, text)| text), in_language: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_reads_the_primary_subtag_and_says_when_it_is_english() {
        assert_eq!(word("de-DE", Word::Max), Printed { text: "MAX", in_language: true });
        assert_eq!(word("DE", Word::Min), Printed { text: "MIN", in_language: true });
        assert_eq!(word("en-GB", Word::Max), Printed { text: "MAX", in_language: true });
        assert_eq!(word("ja", Word::Min), Printed { text: "MIN", in_language: false });
    }

    #[test]
    fn a_malformed_tag_is_english_and_says_so() {
        for tag in ["", "-", "-de", "de_DE", "\u{0}", "\u{e9}\u{e9}", &"x".repeat(4096)] {
            assert_eq!(word(tag, Word::Max), Printed { text: "MAX", in_language: false }, "{tag:?}");
        }
    }

    #[test]
    fn every_language_has_every_word() {
        for (language, row) in WORDS {
            for wanted in [Word::Max, Word::Min] {
                assert!(row.iter().any(|(w, _)| *w == wanted), "{language} lacks {wanted:?}");
            }
        }
    }
}
