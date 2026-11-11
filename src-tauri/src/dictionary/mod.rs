use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordSense {
    pub part_of_speech: String,
    pub glosses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordEntry {
    pub word: String,
    pub senses: Vec<WordSense>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
struct Record {
    word: String,
    senses: Vec<WordSense>,
}

#[derive(Debug, Clone, Default)]
pub struct WordBook {
    records: HashMap<String, Record>,
    lemmas: HashMap<String, String>,
}

impl WordBook {
    pub const GLOSSES_PER_PART_OF_SPEECH: usize = 2;
    pub const TOTAL_GLOSSES: usize = 3;

    pub fn from_tsv(entries_tsv: &str, lemmas_tsv: &str) -> Self {
        Self {
            records: Self::parse_entries(entries_tsv),
            lemmas: Self::parse_lemmas(lemmas_tsv),
        }
    }

    pub fn from_files(entries_path: &Path, lemmas_path: &Path) -> Option<Self> {
        let entries_tsv = fs::read_to_string(entries_path).ok()?;
        let lemmas_tsv = fs::read_to_string(lemmas_path).ok()?;
        Some(Self::from_tsv(&entries_tsv, &lemmas_tsv))
    }

    pub fn bundled() -> Self {
        const EMBEDDED_ENTRIES: &str = include_str!("../../resources/WordBook.tsv");
        const EMBEDDED_LEMMAS: &str = include_str!("../../resources/WordBookLemmas.tsv");
        Self::from_tsv(EMBEDDED_ENTRIES, EMBEDDED_LEMMAS)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Look up one English word with possessive stripping and lemma fallback.
    /// Exact words always take precedence over inflected lemma mappings.
    pub fn lookup(&self, raw_word: &str) -> Option<WordEntry> {
        let key = Self::normalize_key(raw_word)?;
        let record = self.find_record(&key)?;
        let (senses, truncated) = Self::cap_senses(&record.senses);
        Some(WordEntry {
            word: record.word.clone(),
            senses,
            truncated,
        })
    }

    fn find_record(&self, key: &str) -> Option<&Record> {
        if let Some(rec) = self.records.get(key) {
            return Some(rec);
        }
        if let Some(lemma) = self.lemmas.get(key) {
            if let Some(rec) = self.records.get(lemma) {
                return Some(rec);
            }
        }
        // Secondary fallback for common regular inflections not listed in lemmas table
        if let Some(stem) = key.strip_suffix("ies") {
            let candidate = format!("{stem}y");
            if let Some(rec) = self.records.get(&candidate) {
                return Some(rec);
            }
        }
        if let Some(stem) = key.strip_suffix('s') {
            if stem.len() >= 2 {
                if let Some(rec) = self.records.get(stem) {
                    return Some(rec);
                }
            }
        }
        None
    }

    pub fn normalize_key(word: &str) -> Option<String> {
        let trimmed = word
            .trim()
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '“' | '”' | '‘' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.'
                        | ';' | ':' | '!' | '?'
                )
            });
        let mut lower = trimmed.to_lowercase();
        if let Some(stripped) = lower.strip_suffix("'s").or_else(|| lower.strip_suffix("’s")) {
            lower = stripped.to_string();
        }
        if lower.is_empty() || lower.chars().any(|c| c.is_whitespace()) {
            return None;
        }
        Some(lower)
    }

    fn cap_senses(senses: &[WordSense]) -> (Vec<WordSense>, bool) {
        let shown = senses.len().min(Self::TOTAL_GLOSSES);
        if shown == 0 {
            return (Vec::new(), false);
        }
        let base = Self::TOTAL_GLOSSES / shown;
        let mut spare = Self::TOTAL_GLOSSES - base * shown;
        let mut result = Vec::with_capacity(shown);
        let mut dropped = senses.len() > shown;

        for sense in senses.iter().take(shown) {
            let mut budget = base;
            if spare > 0 {
                budget += 1;
                spare -= 1;
            }
            let take = budget
                .min(Self::GLOSSES_PER_PART_OF_SPEECH)
                .min(sense.glosses.len());
            if take < sense.glosses.len() {
                dropped = true;
            }
            result.push(WordSense {
                part_of_speech: sense.part_of_speech.clone(),
                glosses: sense.glosses.iter().take(take).cloned().collect(),
            });
        }
        (result, dropped)
    }

    fn parse_entries(entries_tsv: &str) -> HashMap<String, Record> {
        let mut records = HashMap::with_capacity(45_000);
        for line in entries_tsv.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 3 || fields[0].is_empty() {
                continue;
            }
            let mut senses = Vec::new();
            let mut idx = 1;
            while idx + 1 < fields.len() {
                let glosses: Vec<String> = fields[idx + 1]
                    .split(';')
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(String::from)
                    .collect();
                if !glosses.is_empty() {
                    senses.push(WordSense {
                        part_of_speech: fields[idx].trim().to_string(),
                        glosses,
                    });
                }
                idx += 2;
            }
            if !senses.is_empty() {
                records.insert(
                    fields[0].to_lowercase(),
                    Record {
                        word: fields[0].to_string(),
                        senses,
                    },
                );
            }
        }
        records
    }

    fn parse_lemmas(lemmas_tsv: &str) -> HashMap<String, String> {
        let mut map = HashMap::with_capacity(36_000);
        for line in lemmas_tsv.lines() {
            let mut parts = line.split('\t');
            let (Some(inflected), Some(base), None) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if !inflected.is_empty() && !base.is_empty() {
                map.insert(inflected.to_lowercase(), base.to_lowercase());
            }
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const SAMPLE_ENTRIES: &str = "\
abandon\tvt.\t放弃;抛弃;遗弃\tn.\t放任;无拘束;狂热
settings\tn.\t设置面板;环境背景
setting\tn.\t设置;镶嵌;背景
scale\tn.\t规模;刻度;鳞片\tvt.\t攀登;按比例缩放\tvi.\t剥落;生水垢\tadj.\t成比例的
";

    const SAMPLE_LEMMAS: &str = "\
abandoned\tabandon
abandoning\tabandon
settings\tsetting
";

    #[test]
    fn exact_word_beats_inflected_lemma() {
        let book = WordBook::from_tsv(SAMPLE_ENTRIES, SAMPLE_LEMMAS);
        let entry = book.lookup("Settings").expect("settings should resolve");
        assert_eq!(entry.word, "settings");
        assert_eq!(entry.senses[0].glosses, vec!["设置面板", "环境背景"]);
    }

    #[test]
    fn lemma_resolves_inflected_forms_and_possessives() {
        let book = WordBook::from_tsv(SAMPLE_ENTRIES, SAMPLE_LEMMAS);
        let entry = book.lookup("abandoning").expect("lemma should resolve");
        assert_eq!(entry.word, "abandon");
        // 2 POS share 3 total glosses: first POS gets 2, second gets 1, truncated = true
        assert_eq!(entry.senses.len(), 2);
        assert_eq!(entry.senses[0].glosses.len(), 2);
        assert_eq!(entry.senses[1].glosses.len(), 1);
        assert!(entry.truncated);

        let possessive = book.lookup("Setting’s").expect("possessive should strip");
        assert_eq!(possessive.word, "setting");
    }

    #[test]
    fn caps_three_parts_of_speech_fairly_and_rejects_phrases() {
        let book = WordBook::from_tsv(SAMPLE_ENTRIES, SAMPLE_LEMMAS);
        let entry = book.lookup("scale").expect("scale should resolve");
        assert_eq!(entry.senses.len(), 3);
        assert_eq!(entry.senses[0].glosses.len(), 1);
        assert_eq!(entry.senses[1].glosses.len(), 1);
        assert_eq!(entry.senses[2].glosses.len(), 1);
        assert!(entry.truncated);

        assert!(book.lookup("abandon ship").is_none());
        assert!(book.lookup("   ").is_none());
    }

    #[test]
    fn bundled_wordbook_loads_and_queries_under_5ms() {
        let book = WordBook::bundled();
        assert!(book.len() > 40_000, "expected >40k entries, got {}", book.len());

        let start = Instant::now();
        for _ in 0..100 {
            let res = book.lookup("abbreviates");
            assert!(res.is_some());
        }
        let avg_us = start.elapsed().as_micros() / 100;
        assert!(avg_us < 5_000, "lookup took {avg_us}us, expected < 5ms");
    }
}
