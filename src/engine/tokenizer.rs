use rust_stemmers::{Algorithm, Stemmer};
use sha2::{Digest, Sha256};

pub struct Tokenizer;

impl Tokenizer {
    pub fn tokenize(text: &str, strategy: &str) -> Vec<String> {
        match strategy {
            "exact" => vec![text.to_string()],
            "hash" => {
                let mut hasher = Sha256::new();
                hasher.update(text);
                let result = hasher.finalize();
                vec![hex::encode(result)]
            }
            "term" => Self::tokenize_term(text, false), // unstemmed (strict parity with Dgraph)
            "fulltext" => Self::tokenize_term(text, true), // stemmed
            "trigram" => {
                let text = text.to_lowercase();
                if text.len() < 3 {
                    return vec![];
                }
                let chars: Vec<char> = text.chars().collect();
                chars
                    .windows(3)
                    .map(|w| w.iter().collect::<String>())
                    .collect()
            }
            "datetime" => {
                // Parse ISO string and extract parts
                // year, month, day, hour
                // Dependencies: chrono used?
                // Or just regex parsing since format is strict ISO from helper?
                // Dgraph extracts: "2023", "10", "05", "14".
                // We'll just split by non-digits.
                let parts: Vec<&str> = text
                    .split(|c: char| !c.is_numeric())
                    .filter(|s| !s.is_empty())
                    .collect();
                if parts.is_empty() {
                    return vec![];
                }
                // parts[0] = year
                // parts[1] = month
                // parts[2] = day
                // parts[3] = hour
                // ..
                parts.iter().map(|s| s.to_string()).collect()
            }
            _ => vec![], // Unknown strategy
        }
    }

    fn tokenize_term(text: &str, stem: bool) -> Vec<String> {
        let stemmer = Stemmer::create(Algorithm::English);

        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .filter_map(|t| {
                // Dgraph behavior for term: does it use stop words? Docs say "The term tokenizer... splits the value into terms... It does not support stemming."
                // It likely does NOT remove stop words for term index, only for fulltext.
                // But for now, let's keep stop words removal to avoid index bloat, or check flag?
                // Let's pass a `use_stop_words` flag? Or assume stem=true implies fulltext implies stop words.

                if stem {
                    // Fulltext: Stop Words + Stemming
                    match t.as_str() {
                        "a" | "an" | "and" | "are" | "as" | "at" | "be" | "but" | "by" | "for"
                        | "if" | "in" | "into" | "is" | "it" | "no" | "not" | "of" | "on"
                        | "or" | "such" | "that" | "the" | "their" | "then" | "there" | "these"
                        | "they" | "this" | "to" | "was" | "will" | "with" => None,
                        _ => Some(stemmer.stem(&t).to_string()),
                    }
                } else {
                    // Term: No Stop Words removal (standard tokenization), No Stemming
                    Some(t)
                }
            })
            .collect()
    }
}
