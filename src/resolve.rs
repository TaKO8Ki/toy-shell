use crate::parser::{Span, Word};
use crate::shell::Shell;
use tracing::debug;

pub fn resolve_alias(shell: &Shell, argv: &[Word]) -> Vec<Word> {
    debug!("aliases={:?}", shell.aliases());
    argv
        // Get the first word.
        .get(0)
        // Get the first span in the first word.
        .and_then(|word| word.spans().get(0))
        // Make sure that the span is a literal (not parameters, etc.).
        .and_then(|span| match span {
            Span::Literal(lit) => Some(lit),
            _ => None,
        })
        // The very first span is literal. Search the registered aliases.
        .and_then(|lit| shell.lookup_alias(lit.as_str()))
        .map(|alias_str| {
            // Found the alias. Split the alias string by whitespace into words.
            let mut alias_words: Vec<Word> = alias_str
                .trim()
                .split(' ')
                .map(|w| {
                    let span = Span::Literal(w.to_owned());
                    Word(vec![span])
                })
                .collect();

            // Append argv except the first word (alias name).
            for arg in argv.iter().skip(1) {
                alias_words.push(arg.clone());
            }

            alias_words
        })
        // Failed to expand alias. Return argv as it is.
        .unwrap_or_else(|| argv.to_owned())
}
