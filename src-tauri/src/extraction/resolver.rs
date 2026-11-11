use crate::extraction::{ExtractionCompleteness, ScreenPoint, ScreenRect};
use serde::{Deserialize, Serialize};

pub const MAX_SENTENCE_WINDOW_UTF16: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Utf16Range {
    pub location: usize,
    pub length: usize,
}

impl Utf16Range {
    pub fn new(location: usize, length: usize) -> Self {
        Self { location, length }
    }

    pub fn end(&self) -> usize {
        self.location + self.length
    }

    pub fn contains_offset(&self, offset: usize) -> bool {
        offset >= self.location && offset < self.end()
    }
}

pub fn slice_utf16(units: &[u16], range: Utf16Range) -> Option<String> {
    if range.end() > units.len() {
        return None;
    }
    String::from_utf16(&units[range.location..range.end()]).ok()
}

/// Extracts a word around a UTF-16 offset or range with ±1 character edge tolerance
/// and tolerance for system ranges that include adjacent punctuation or spaces.
pub fn resolve_word_in_utf16(
    text: &str,
    reported_range: Utf16Range,
    allow_edge_tolerance: bool,
) -> Option<(Utf16Range, String)> {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.is_empty() || reported_range.location >= units.len() {
        return None;
    }

    // 1. Direct hit at reported start
    let mut anchor = None;
    if is_word_unit(units[reported_range.location]) {
        anchor = Some(reported_range.location);
    }

    // 2. Scan inside reported range (handles ranges like `" word."` or `"(word)"`)
    if anchor.is_none() && reported_range.length > 1 {
        let scan_end = reported_range.end().min(units.len());
        for idx in reported_range.location..scan_end {
            if is_word_unit(units[idx]) {
                anchor = Some(idx);
                break;
            }
        }
    }

    // 3. ±1 character edge tolerance for slight cursor overhang on punctuation/space
    if anchor.is_none() && allow_edge_tolerance {
        if reported_range.location > 0 && is_word_unit(units[reported_range.location - 1]) {
            anchor = Some(reported_range.location - 1);
        } else if reported_range.location + 1 < units.len()
            && is_word_unit(units[reported_range.location + 1])
        {
            anchor = Some(reported_range.location + 1);
        }
    }

    let center = anchor?;
    let mut start = center;
    while start > 0 && is_word_unit(units[start - 1]) {
        start -= 1;
    }
    let mut end = center + 1;
    while end < units.len() && is_word_unit(units[end]) {
        end += 1;
    }

    // Absorb internal apostrophes ("don't", "user’s") and hyphens ("well-known")
    while end + 1 < units.len() && is_word_joiner(units[end]) && is_word_unit(units[end + 1]) {
        end += 1;
        while end < units.len() && is_word_unit(units[end]) {
            end += 1;
        }
    }
    while start >= 2 && is_word_joiner(units[start - 1]) && is_word_unit(units[start - 2]) {
        start -= 1;
        while start > 0 && is_word_unit(units[start - 1]) {
            start -= 1;
        }
    }

    let range = Utf16Range::new(start, end - start);
    let word = slice_utf16(&units, range)?;
    Some((range, word))
}

fn is_word_unit(unit: u16) -> bool {
    let Some(ch) = char::from_u32(unit as u32) else {
        // Surrogate code units are non-word boundaries (e.g. emoji)
        return false;
    };
    ch.is_alphabetic() || ch.is_ascii_digit() || is_combining_mark(ch)
}

fn is_combining_mark(ch: char) -> bool {
    let code = ch as u32;
    (0x0300..=0x036F).contains(&code)
        || (0x1AB0..=0x1AFF).contains(&code)
        || (0x1DC0..=0x1DFF).contains(&code)
        || (0x20D0..=0x20FF).contains(&code)
        || (0xFE20..=0xFE2F).contains(&code)
}

fn is_word_joiner(unit: u16) -> bool {
    matches!(unit, 0x0027 | 0x2019 | 0x2018 | 0x002D | 0x2010 | 0x2011)
}

// ============================================================================
// Line-Break Normalizer with Bidirectional UTF-16 Mapping
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedContext {
    pub text: String,
    pub units: Vec<u16>,
    origins: Vec<Utf16Range>,
}

impl NormalizedContext {
    /// Fold single line breaks (and surrounding horizontal whitespace) into a single space,
    /// while preserving 2+ consecutive line breaks as hard paragraph boundaries.
    pub fn normalize(raw: &str) -> Self {
        let src: Vec<u16> = raw.encode_utf16().collect();
        let mut units = Vec::with_capacity(src.len());
        let mut origins = Vec::with_capacity(src.len());

        let mut i = 0;
        while i < src.len() {
            if !is_utf16_whitespace(src[i]) {
                units.push(src[i]);
                origins.push(Utf16Range::new(i, 1));
                i += 1;
                continue;
            }

            let mut end = i;
            let mut line_breaks = 0usize;
            while end < src.len() && is_utf16_whitespace(src[end]) {
                if is_utf16_linebreak(src[end]) {
                    line_breaks += 1;
                    // Treat CRLF as a single line break
                    if src[end] == 0x000D && end + 1 < src.len() && src[end + 1] == 0x000A {
                        end += 1;
                    }
                }
                end += 1;
            }

            if line_breaks == 1 {
                // Soft line wrap -> single space, preserving full origin span
                units.push(0x0020);
                origins.push(Utf16Range::new(i, end - i));
            } else {
                for (offset, &u) in src.iter().enumerate().take(end).skip(i) {
                    units.push(u);
                    origins.push(Utf16Range::new(offset, 1));
                }
            }
            i = end;
        }

        let text = String::from_utf16_lossy(&units);
        Self {
            text,
            units,
            origins,
        }
    }

    pub fn original_range(&self, normalized: Utf16Range) -> Option<Utf16Range> {
        if normalized.length == 0 || normalized.end() > self.origins.len() {
            return None;
        }
        let first = self.origins[normalized.location];
        let last = self.origins[normalized.end() - 1];
        Some(Utf16Range::new(first.location, last.end() - first.location))
    }

    pub fn normalized_offset(&self, original_offset: usize) -> Option<usize> {
        self.origins
            .iter()
            .position(|r| r.contains_offset(original_offset))
    }
}

fn is_utf16_whitespace(u: u16) -> bool {
    char::from_u32(u as u32).is_some_and(|c| c.is_whitespace())
}

fn is_utf16_linebreak(u: u16) -> bool {
    matches!(u, 0x000A | 0x000D | 0x0085 | 0x2028 | 0x2029)
}

// ============================================================================
// Abbreviation-Safe Cross-Platform Sentence Resolver
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentenceWindow {
    pub text: String,
    pub starts_at_element_start: bool,
    pub ends_at_element_end: bool,
}

impl SentenceWindow {
    pub fn bounded(text: impl Into<String>, window_start: usize, element_end: Option<usize>) -> Self {
        let text = text.into();
        let len_utf16 = text.encode_utf16().count();
        Self {
            text,
            starts_at_element_start: window_start == 0,
            ends_at_element_end: element_end.is_some_and(|end| window_start + len_utf16 >= end),
        }
    }

    pub fn fragment(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            starts_at_element_start: false,
            ends_at_element_end: false,
        }
    }

    pub fn complete_block(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            starts_at_element_start: true,
            ends_at_element_end: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SentenceBoundary {
    Complete,
    ClippedStart,
    ClippedEnd,
    MissingTerminator,
    MissingBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSentence {
    pub original_range: Utf16Range,
    pub text: String,
    pub completeness: ExtractionCompleteness,
    pub boundary: SentenceBoundary,
}

pub struct SentenceResolver;

impl SentenceResolver {
    pub fn resolve(window: &SentenceWindow, utf16_offset: usize) -> Option<ResolvedSentence> {
        let norm = NormalizedContext::normalize(&window.text);
        if norm.units.is_empty() {
            return None;
        }
        let mut target_offset = norm.normalized_offset(utf16_offset)?;
        // If offset landed on a single space right next to text (e.g. ±1 edge tolerance), nudge left
        if is_utf16_whitespace(norm.units[target_offset])
            && !is_utf16_linebreak(norm.units[target_offset])
            && target_offset > 0
            && !is_utf16_whitespace(norm.units[target_offset - 1])
        {
            target_offset -= 1;
        }

        let spans = split_sentence_spans(&norm.units);
        let span = spans
            .into_iter()
            .find(|r| r.contains_offset(target_offset))?;

        let trimmed = trim_utf16_range(&norm.units, span);
        if trimmed.length == 0 || !trimmed.contains_offset(target_offset) {
            return None;
        }

        let sentence = slice_utf16(&norm.units, trimmed)?;
        let original_range = norm.original_range(trimmed)?;
        let boundary = Self::classify_boundary(trimmed, &norm.units, window);
        let completeness = if boundary == SentenceBoundary::Complete {
            ExtractionCompleteness::Complete
        } else {
            ExtractionCompleteness::Partial
        };

        Some(ResolvedSentence {
            original_range,
            text: sentence,
            completeness,
            boundary,
        })
    }

    fn classify_boundary(
        range: Utf16Range,
        norm_units: &[u16],
        window: &SentenceWindow,
    ) -> SentenceBoundary {
        if range.location == 0 {
            if !window.starts_at_element_start {
                return SentenceBoundary::ClippedStart;
            }
        } else if !has_proven_left_boundary(norm_units, range.location) {
            return SentenceBoundary::MissingBoundary;
        }

        if !ends_with_terminator(&norm_units[range.location..range.end()]) {
            if range.end() >= norm_units.len() && !window.ends_at_element_end {
                return SentenceBoundary::ClippedEnd;
            }
            return SentenceBoundary::MissingTerminator;
        }

        SentenceBoundary::Complete
    }
}

fn split_sentence_spans(units: &[u16]) -> Vec<Utf16Range> {
    let mut spans = Vec::new();
    let mut seg_start = 0usize;
    let mut i = 0usize;

    while i < units.len() {
        // Hard paragraph break (any surviving newline in normalized units)
        if is_utf16_linebreak(units[i]) {
            if i > seg_start {
                spans.push(Utf16Range::new(seg_start, i - seg_start));
            }
            while i < units.len() && is_utf16_whitespace(units[i]) {
                i += 1;
            }
            seg_start = i;
            continue;
        }

        if is_sentence_terminator(units[i]) {
            // Check whether this period is an abbreviation, decimal, domain, or ellipsis
            if units[i] == b'.' as u16 {
                // Consume ellipsis "..."
                if i + 1 < units.len() && units[i + 1] == b'.' as u16 {
                    while i + 1 < units.len() && units[i + 1] == b'.' as u16 {
                        i += 1;
                    }
                } else if is_non_terminal_period(units, seg_start, i) {
                    i += 1;
                    continue;
                }
            }

            // Absorb trailing closers (quotes, parens, brackets)
            let mut end = i + 1;
            while end < units.len() && is_closer_unit(units[end]) {
                end += 1;
            }

            // For ASCII `.` / `!` / `?`, require end-of-text or whitespace/CJK after closers
            let cjk_term = matches!(units[i], 0x3002 | 0xFF01 | 0xFF1F); // 。！？
            let followed_by_boundary =
                end >= units.len() || is_utf16_whitespace(units[end]) || cjk_term;

            if followed_by_boundary {
                spans.push(Utf16Range::new(seg_start, end - seg_start));
                // Advance past horizontal spaces (stop before linebreaks so paragraph logic handles them)
                let mut next = end;
                while next < units.len()
                    && is_utf16_whitespace(units[next])
                    && !is_utf16_linebreak(units[next])
                {
                    next += 1;
                }
                seg_start = next;
                i = next;
                continue;
            }
        }

        i += 1;
    }

    if seg_start < units.len() {
        spans.push(Utf16Range::new(seg_start, units.len() - seg_start));
    }

    spans
}

fn is_non_terminal_period(units: &[u16], seg_start: usize, dot_idx: usize) -> bool {
    // 1. Immediately followed by non-whitespace (after optional quote? No, e.g. `3.14`, `example.com`, `U.S.A.`)
    if dot_idx + 1 < units.len() {
        let next = units[dot_idx + 1];
        if !is_utf16_whitespace(next) && !is_closer_unit(next) {
            return true;
        }
    }

    // Extract token preceding dot_idx
    let mut word_start = dot_idx;
    while word_start > seg_start {
        let prev = units[word_start - 1];
        if is_word_unit(prev) || prev == b'.' as u16 {
            word_start -= 1;
        } else {
            break;
        }
    }
    let Some(token) = slice_utf16(units, Utf16Range::new(word_start, dot_idx - word_start)) else {
        return false;
    };
    if token.is_empty() {
        return false;
    }

    let lower = token.to_lowercase();

    // Known title, academic, bibliographic, and Latin abbreviations
    const ABBREVIATIONS: &[&str] = &[
        "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "mt", "vs", "etc", "e.g", "i.e",
        "cf", "al", "fig", "figs", "eq", "eqs", "vol", "vols", "no", "nos", "pp", "ch", "sec",
        "inc", "ltd", "co", "corp", "dept", "univ", "assn", "bros", "jan", "feb", "mar", "apr",
        "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec", "approx", "est", "min", "max",
        "u.s", "u.k", "e.u", "ph.d", "m.d", "b.a", "m.a", "m.s",
    ];
    if ABBREVIATIONS.contains(&lower.as_str()) {
        return true;
    }

    // Single uppercase initial (e.g., "A. M. Turing", "J. K. Rowling") when NOT at the end of a clause
    let chars: Vec<char> = token.chars().collect();
    if chars.len() == 1 && chars[0].is_ascii_uppercase() {
        // Look ahead past whitespace: if followed by an uppercase word, treat as initial
        let mut k = dot_idx + 1;
        while k < units.len() && is_utf16_whitespace(units[k]) && !is_utf16_linebreak(units[k]) {
            k += 1;
        }
        if k < units.len() {
            if let Some(ch) = char::from_u32(units[k] as u32) {
                if ch.is_ascii_uppercase() {
                    return true;
                }
            }
        }
    }

    // Next non-space character starts with lowercase -> almost certainly mid-sentence abbreviation (e.g. "op. cit. in")
    let mut k = dot_idx + 1;
    while k < units.len() && is_utf16_whitespace(units[k]) && !is_utf16_linebreak(units[k]) {
        k += 1;
    }
    if k < units.len() {
        if let Some(ch) = char::from_u32(units[k] as u32) {
            if ch.is_ascii_lowercase() {
                return true;
            }
        }
    }

    false
}

fn has_proven_left_boundary(units: &[u16], location: usize) -> bool {
    if location == 0 {
        return true;
    }
    let mut idx = location as isize - 1;
    while idx >= 0 && is_utf16_whitespace(units[idx as usize]) {
        if is_utf16_linebreak(units[idx as usize]) {
            return true;
        }
        idx -= 1;
    }
    while idx >= 0 && is_closer_unit(units[idx as usize]) {
        idx -= 1;
    }
    idx >= 0 && is_sentence_terminator(units[idx as usize])
}

fn ends_with_terminator(slice: &[u16]) -> bool {
    let mut idx = slice.len() as isize - 1;
    while idx >= 0 && is_closer_unit(slice[idx as usize]) {
        idx -= 1;
    }
    idx >= 0 && is_sentence_terminator(slice[idx as usize])
}

fn trim_utf16_range(units: &[u16], range: Utf16Range) -> Utf16Range {
    let mut start = range.location;
    let mut end = range.end();
    while start < end && is_utf16_whitespace(units[start]) {
        start += 1;
    }
    while end > start && is_utf16_whitespace(units[end - 1]) {
        end -= 1;
    }
    Utf16Range::new(start, end - start)
}

fn is_sentence_terminator(u: u16) -> bool {
    matches!(
        u,
        0x002E | 0x0021 | 0x003F | 0x2026 | 0x3002 | 0xFF01 | 0xFF1F
    )
}

fn is_closer_unit(u: u16) -> bool {
    matches!(
        u,
        0x0022 | 0x0027 | 0x201D | 0x2019 | 0x0029 | 0x005D | 0x00BB | 0xFF09 | 0x3011 | 0x300D
    )
}

// ============================================================================
// Column-Segmented OCR Multiline Clustering (Replaces Legacy `isAloneInItsBand`)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrWord {
    pub text: String,
    pub rect: ScreenRect,
    pub confidence: f32,
    pub trailing: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub text: String,
    pub words: Vec<OcrWord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OcrPointHit {
    pub word: OcrWord,
    pub sentence_text: Option<String>,
    pub sentence_completeness: ExtractionCompleteness,
    pub sentence_rect: ScreenRect,
}

#[derive(Debug, Clone)]
struct ColumnRun {
    words: Vec<OcrWord>,
    rect: ScreenRect,
}

pub struct OcrClusterer;

impl OcrClusterer {
    /// Lowered threshold so small anti-aliased dark-mode fonts are not discarded.
    pub const MIN_CONFIDENCE: f32 = 0.45;
    /// Pointer hit tolerance in logical points.
    pub const HIT_TOLERANCE_PT: f64 = 4.0;
    /// Inline horizontal snap floor (±6pt minimum) to keep slightly spaced words joined.
    pub const MIN_INLINE_JOIN_GAP_PT: f64 = 6.0;
    /// Max lines assembled in one paragraph context around a hovered word.
    pub const MAX_CONTEXT_LINES: usize = 8;

    /// Pick the recognized word under `point` and simultaneously cluster its column
    /// paragraph so both word + cross-line sentence context are returned in one pass.
    pub fn extract_at_point(point: ScreenPoint, lines: &[OcrLine]) -> Option<OcrPointHit> {
        let all_words: Vec<&OcrWord> = lines
            .iter()
            .flat_map(|l| l.words.iter())
            .filter(|w| w.confidence >= Self::MIN_CONFIDENCE && w.rect.is_valid())
            .collect();

        // Prefer exact box hit first; otherwise closest center within tolerance
        let hit = all_words
            .iter()
            .copied()
            .filter(|w| {
                w.rect
                    .inset_by(-Self::HIT_TOLERANCE_PT, -Self::HIT_TOLERANCE_PT)
                    .contains(point)
            })
            .min_by(|a, b| {
                let da = ScreenPoint::new(a.rect.mid_x(), a.rect.mid_y()).distance_to(point);
                let db = ScreenPoint::new(b.rect.mid_x(), b.rect.mid_y()).distance_to(point);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })?
            .clone();

        // Segment all lines into intra-column runs (splitting wide horizontal column gutters)
        let runs = Self::segment_into_runs(lines);
        let (paragraph_runs, clipped_top, clipped_bottom) =
            Self::cluster_column_paragraph(&hit, &runs);

        if paragraph_runs.is_empty() {
            return Some(OcrPointHit {
                sentence_rect: hit.rect,
                word: hit,
                sentence_text: None,
                sentence_completeness: ExtractionCompleteness::Partial,
            });
        }

        let (assembled, word_ranges, flat_words) = Self::assemble_runs(&paragraph_runs);
        let hit_idx = flat_words.iter().position(|w| *w == hit);

        if let Some(idx) = hit_idx {
            let hit_utf16_offset = word_ranges[idx].location;
            let starts_proven = !clipped_top
                && assembled
                    .chars()
                    .find(|c| !c.is_whitespace())
                    .is_some_and(|c| c.is_uppercase() || ('\u{4E00}'..='\u{9FFF}').contains(&c));
            let window = SentenceWindow {
                text: assembled.clone(),
                starts_at_element_start: starts_proven,
                ends_at_element_end: !clipped_bottom,
            };
            if let Some(resolved) = SentenceResolver::resolve(&window, hit_utf16_offset) {
                let covered_rect = word_ranges
                    .iter()
                    .zip(flat_words.iter())
                    .filter(|(r, _)| {
                        r.location < resolved.original_range.end()
                            && resolved.original_range.location < r.end()
                    })
                    .fold(hit.rect, |acc, (_, w)| acc.union(w.rect));

                return Some(OcrPointHit {
                    word: hit,
                    sentence_text: Some(resolved.text),
                    sentence_completeness: resolved.completeness,
                    sentence_rect: covered_rect,
                });
            }
        }

        // Fallback to the hit's single column run if no multi-sentence resolution applied
        let single_run = paragraph_runs
            .iter()
            .find(|r| r.words.contains(&hit))
            .cloned()
            .unwrap_or(ColumnRun {
                rect: hit.rect,
                words: vec![hit.clone()],
            });
        let fallback_line = single_run
            .words
            .iter()
            .map(|w| format!("{}{}", w.text, w.trailing))
            .collect::<Vec<_>>()
            .join(" ");

        Some(OcrPointHit {
            word: hit,
            sentence_text: Some(fallback_line.trim().to_string()),
            sentence_completeness: ExtractionCompleteness::Partial,
            sentence_rect: single_run.rect,
        })
    }

    /// Assemble full text from a region screenshot (for `Alt+Shift+S` / `⌃⌘S` region capture).
    pub fn assemble_region_text(lines: &[OcrLine]) -> Option<String> {
        let runs = Self::segment_into_runs(lines);
        if runs.is_empty() {
            return None;
        }
        let median_h = Self::median_height(&runs);
        let mut output = String::new();
        let mut prev_rect: Option<ScreenRect> = None;

        for run in &runs {
            let line_text = run
                .words
                .iter()
                .map(|w| format!("{}{}", w.text, w.trailing))
                .collect::<Vec<_>>()
                .join(" ");
            let trimmed = line_text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(prev) = prev_rect {
                let v_gap = run.rect.min_y() - prev.max_y();
                if v_gap > median_h * 1.85 {
                    output.push_str("\n\n");
                } else {
                    output.push(' ');
                }
            }
            output.push_str(trimmed);
            prev_rect = Some(run.rect);
        }

        let clean = output.trim().to_string();
        (!clean.is_empty()).then_some(clean)
    }

    /// Split raw OCR lines by wide horizontal gaps into distinct column runs,
    /// sorted top-to-bottom (top-left origin: smaller Y is higher on screen).
    fn segment_into_runs(lines: &[OcrLine]) -> Vec<ColumnRun> {
        let mut runs = Vec::new();

        for line in lines {
            let mut usable: Vec<OcrWord> = line
                .words
                .iter()
                .filter(|w| w.confidence >= Self::MIN_CONFIDENCE && w.rect.is_valid())
                .cloned()
                .collect();
            if usable.is_empty() {
                continue;
            }
            usable.sort_by(|a, b| {
                a.rect
                    .min_x()
                    .partial_cmp(&b.rect.min_x())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut current = vec![usable[0].clone()];
            for word in usable.into_iter().skip(1) {
                let prev = current.last().expect("non-empty");
                if Self::joins_inline(&prev.rect, &word.rect) {
                    current.push(word);
                } else {
                    let rect = Self::bounding_box(&current);
                    runs.push(ColumnRun {
                        words: current,
                        rect,
                    });
                    current = vec![word];
                }
            }
            let rect = Self::bounding_box(&current);
            runs.push(ColumnRun {
                words: current,
                rect,
            });
        }

        // Merge words split into separate OCR observations on the exact same visual row & column
        runs.sort_by(|a, b| {
            let y_diff = (a.rect.mid_y() - b.rect.mid_y()).abs();
            let min_h = a.rect.height.min(b.rect.height);
            if y_diff <= min_h * 0.45 {
                a.rect
                    .min_x()
                    .partial_cmp(&b.rect.min_x())
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else {
                a.rect
                    .mid_y()
                    .partial_cmp(&b.rect.mid_y())
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        });

        let mut merged: Vec<ColumnRun> = Vec::with_capacity(runs.len());
        for run in runs {
            if let Some(last) = merged.last_mut() {
                if Self::overlaps_vertically(&last.rect, &run.rect)
                    && Self::joins_inline(&last.rect, &run.rect)
                {
                    last.words.extend(run.words);
                    last.rect = Self::bounding_box(&last.words);
                    continue;
                }
            }
            merged.push(run);
        }

        merged
    }

    /// Lock onto the column containing `hit` and expand upward/downward within that column ONLY.
    /// Sidebars, line numbers, or table cells in other X-bands are ignored instead of blocking expansion!
    fn cluster_column_paragraph(
        hit: &OcrWord,
        runs: &[ColumnRun],
    ) -> (Vec<ColumnRun>, bool, bool) {
        let Some(hit_idx) = runs.iter().position(|r| r.words.contains(hit)) else {
            return (Vec::new(), false, false);
        };
        let hit_run = &runs[hit_idx];
        let median_h = Self::median_height(runs).max(hit_run.rect.height).max(8.0);

        // Filter runs belonging to the SAME vertical column as `hit_run`
        // (ignores sidebar / gutter / secondary column items on the same Y band)
        let col_candidates: Vec<&ColumnRun> = runs
            .iter()
            .filter(|cand| Self::belongs_to_same_column(&hit_run.rect, &cand.rect, median_h))
            .collect();

        let Some(col_hit_pos) = col_candidates
            .iter()
            .position(|r| r.words.contains(hit))
        else {
            return (vec![hit_run.clone()], false, false);
        };

        // Estimate column left margin from full-ish lines in this column
        let mut left_edges: Vec<f64> = col_candidates
            .iter()
            .map(|r| r.rect.min_x())
            .collect();
        left_edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let col_left_margin = left_edges
            .first()
            .copied()
            .unwrap_or(hit_run.rect.min_x());

        let mut block = vec![col_candidates[col_hit_pos].clone()];

        // Expand upward
        let mut anchor = col_candidates[col_hit_pos];
        let mut up = col_hit_pos;
        let mut clipped_top = false;
        while up > 0 {
            let prev = col_candidates[up - 1];
            if Self::overlaps_vertically(&prev.rect, &anchor.rect) {
                up -= 1;
                continue;
            }
            if Self::is_paragraph_continuation(
                &prev.rect,
                &anchor.rect,
                col_left_margin,
                median_h,
            ) {
                if block.len() >= Self::MAX_CONTEXT_LINES {
                    clipped_top = true;
                    break;
                }
                block.insert(0, prev.clone());
                anchor = prev;
                up -= 1;
            } else {
                break;
            }
        }

        // Expand downward
        anchor = col_candidates[col_hit_pos];
        let mut down = col_hit_pos + 1;
        let mut clipped_bottom = false;
        while down < col_candidates.len() {
            let next = col_candidates[down];
            if Self::overlaps_vertically(&anchor.rect, &next.rect) {
                down += 1;
                continue;
            }
            if Self::is_paragraph_continuation(
                &anchor.rect,
                &next.rect,
                col_left_margin,
                median_h,
            ) {
                if block.len() >= Self::MAX_CONTEXT_LINES {
                    clipped_bottom = true;
                    break;
                }
                block.push(next.clone());
                anchor = next;
                down += 1;
            } else {
                break;
            }
        }

        (block, clipped_top, clipped_bottom)
    }

    fn belongs_to_same_column(hit_box: &ScreenRect, cand_box: &ScreenRect, median_h: f64) -> bool {
        let overlap = hit_box.max_x().min(cand_box.max_x()) - hit_box.min_x().max(cand_box.min_x());
        if overlap > 0.0 {
            return true;
        }
        // Also allow if left margins align closely (e.g. hit was on an indented short line or vice versa)
        (hit_box.min_x() - cand_box.min_x()).abs() <= 3.5 * median_h
    }

    /// Checks whether `lower` continues the paragraph of `upper` inside the same column:
    /// - Vertical gap up to `1.85 × median_h` (supports modern web `1.5~2.0` line-height).
    /// - Left-aligned continuation: allows first-line indent (`upper.min_x` indented up to `3.5 × median_h`)
    ///   AND short tail lines (`lower` has only 1-2 short words like `"done."` whose `min_x` matches column margin).
    fn is_paragraph_continuation(
        upper: &ScreenRect,
        lower: &ScreenRect,
        col_left_margin: f64,
        median_h: f64,
    ) -> bool {
        let ref_h = median_h.max(upper.height).max(lower.height);
        let v_gap = lower.min_y() - upper.max_y();
        if v_gap < -0.35 * ref_h || v_gap > 1.85 * ref_h {
            return false;
        }

        // Left alignment check (supports indented first line + short tail line!)
        let upper_left_delta = (upper.min_x() - col_left_margin).abs();
        let lower_left_delta = (lower.min_x() - col_left_margin).abs();
        let pair_left_delta = (upper.min_x() - lower.min_x()).abs();

        // Standard left-aligned or first-line-indented paragraph
        let left_aligned = (upper_left_delta <= 3.5 * ref_h && lower_left_delta <= 1.8 * ref_h)
            || pair_left_delta <= 2.0 * ref_h;

        // Or centered/wide overlap (>50% of narrower, provided narrower isn't a drifted sidebar)
        let overlap = upper.max_x().min(lower.max_x()) - upper.min_x().max(lower.min_x());
        let narrower = upper.width.min(lower.width);
        let strong_overlap = narrower > 0.0 && overlap >= 0.5 * narrower && pair_left_delta <= 4.0 * ref_h;

        // If `upper` is clearly a short tail line (much narrower than `lower` AND ends with sentence terminator?
        // Geometry alone: if `upper.max_x` stops far short of `lower.max_x` by > 6 * ref_h while `lower` is indented,
        // `lower` starts a new indented paragraph!)
        let new_indented_para =
            lower.min_x() - col_left_margin > 1.2 * ref_h && lower.min_x() - upper.min_x() > 1.2 * ref_h;
        if new_indented_para {
            return false;
        }

        left_aligned || strong_overlap
    }

    fn assemble_runs(runs: &[ColumnRun]) -> (String, Vec<Utf16Range>, Vec<OcrWord>) {
        let mut text = String::new();
        let mut utf16_len = 0usize;
        let mut ranges = Vec::new();
        let mut flat_words = Vec::new();

        'outer: for run in runs {
            let mut first_in_line = true;
            for word in &run.words {
                let piece = format!("{}{}", word.text, word.trailing);
                let sep = if text.is_empty() {
                    ""
                } else if first_in_line {
                    "\n"
                } else {
                    " "
                };
                let sep_len = sep.encode_utf16().count();
                let piece_len = piece.encode_utf16().count();
                if utf16_len + sep_len + piece_len > MAX_SENTENCE_WINDOW_UTF16 {
                    break 'outer;
                }
                text.push_str(sep);
                text.push_str(&piece);
                ranges.push(Utf16Range::new(utf16_len + sep_len, piece_len));
                flat_words.push(word.clone());
                utf16_len += sep_len + piece_len;
                first_in_line = false;
            }
        }

        (text, ranges, flat_words)
    }

    fn joins_inline(left: &ScreenRect, right: &ScreenRect) -> bool {
        let gap = right.min_x() - left.max_x();
        let ref_h = left.height.max(right.height);
        // Column gutter threshold: >= max(6pt, 0.85 * line_height) keeps inline words together
        // while splitting multi-column layouts / sidebars (> ~18-24pt gap).
        gap >= -0.4 * ref_h && gap <= Self::MIN_INLINE_JOIN_GAP_PT.max(0.85 * ref_h)
    }

    fn overlaps_vertically(a: &ScreenRect, b: &ScreenRect) -> bool {
        let overlap = a.max_y().min(b.max_y()) - a.min_y().max(b.min_y());
        let shorter = a.height.min(b.height);
        shorter > 0.0 && overlap >= shorter * 0.45
    }

    fn bounding_box(words: &[OcrWord]) -> ScreenRect {
        let Some(first) = words.first() else {
            return ScreenRect::default();
        };
        words
            .iter()
            .skip(1)
            .fold(first.rect, |acc, w| acc.union(w.rect))
    }

    fn median_height(runs: &[ColumnRun]) -> f64 {
        if runs.is_empty() {
            return 16.0;
        }
        let mut heights: Vec<f64> = runs.iter().map(|r| r.rect.height).collect();
        heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        heights[heights.len() / 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_word(text: &str, trailing: &str, x: f64, y: f64, w: f64, h: f64) -> OcrWord {
        OcrWord {
            text: text.to_string(),
            trailing: trailing.to_string(),
            rect: ScreenRect::new(x, y, w, h),
            confidence: 0.92,
        }
    }

    #[test]
    fn word_extraction_handles_punctuation_spans_and_edge_tolerance() {
        let sample = "Hover over (well-known) words, don't fail!";
        // System AX range covers `"words,"` including trailing comma
        let (_, word) = resolve_word_in_utf16(sample, Utf16Range::new(24, 6), true).unwrap();
        assert_eq!(word, "words");

        // System AX range covers `"("` right before `well-known` -> ±1 tolerance catches `well-known`
        let (_, hyphenated) = resolve_word_in_utf16(sample, Utf16Range::new(11, 1), true).unwrap();
        assert_eq!(hyphenated, "well-known");

        // Contraction `don't` stays intact
        let (_, contraction) = resolve_word_in_utf16(sample, Utf16Range::new(31, 2), true).unwrap();
        assert_eq!(contraction, "don't");
    }

    #[test]
    fn sentence_resolver_preserves_abbreviations_decimals_domains_and_wraps() {
        let raw = "First para.\n\nDr. Smith paid $3.14 on example.com\nfor a well-tested v2.0 tool! Next sentence.";
        let window = SentenceWindow::complete_block(raw);
        // Offset pointing inside `"tool"` on the wrapped second line
        let offset = raw.encode_utf16().position(|u| u == b'v' as u16).unwrap();
        let res = SentenceResolver::resolve(&window, offset).expect("should resolve sentence");
        assert_eq!(
            res.text,
            "Dr. Smith paid $3.14 on example.com for a well-tested v2.0 tool!"
        );
        assert_eq!(res.completeness, ExtractionCompleteness::Complete);
        assert_eq!(res.boundary, SentenceBoundary::Complete);
    }

    #[test]
    fn ocr_clusterer_ignores_sidebar_and_handles_wide_line_spacing_plus_short_tail() {
        // Layout:
        // Left sidebar at x=20..110 on the EXACT same rows as main article at x=260..680!
        // Line height = 16pt, vertical gap = 26pt (1.625x line height — legacy 0.9x rejected this!).
        // Line 3 is a 1-word short tail `"instantly."` (x=260..330, width=70pt vs 420pt — legacy 50% overlap rejected!).
        let lines = vec![
            OcrLine {
                text: "Overview Cross-platform translation bridges".into(),
                words: vec![
                    mk_word("Overview", "", 20.0, 100.0, 80.0, 16.0), // Sidebar item!
                    mk_word("Cross-platform", "", 260.0, 100.0, 130.0, 16.0),
                    mk_word("translation", "", 396.0, 100.0, 100.0, 16.0),
                    mk_word("bridges", "", 502.0, 100.0, 75.0, 16.0),
                ],
            },
            OcrLine {
                text: "Settings native accessibility trees and local OCR".into(),
                words: vec![
                    mk_word("Settings", "", 20.0, 142.0, 75.0, 16.0), // Sidebar item on row 2!
                    mk_word("native", "", 260.0, 142.0, 60.0, 16.0),
                    mk_word("accessibility", "", 325.0, 142.0, 110.0, 16.0),
                    mk_word("trees", "", 440.0, 142.0, 50.0, 16.0),
                    mk_word("and", "", 495.0, 142.0, 35.0, 16.0),
                    mk_word("local", "", 535.0, 142.0, 45.0, 16.0),
                    mk_word("OCR", "", 585.0, 142.0, 45.0, 16.0),
                ],
            },
            OcrLine {
                text: "Shortcuts instantly.".into(),
                words: vec![
                    mk_word("Shortcuts", "", 20.0, 184.0, 85.0, 16.0), // Sidebar item on row 3!
                    mk_word("instantly", ".", 260.0, 184.0, 72.0, 16.0), // Short tail line!
                ],
            },
        ];

        // Hover over `"bridges"` on line 1 (x=520, y=108)
        let hit = OcrClusterer::extract_at_point(ScreenPoint::new(520.0, 108.0), &lines)
            .expect("should hit word");
        assert_eq!(hit.word.text, "bridges");
        assert_eq!(
            hit.sentence_text.as_deref(),
            Some("Cross-platform translation bridges native accessibility trees and local OCR instantly.")
        );
        assert_eq!(hit.sentence_completeness, ExtractionCompleteness::Complete);
    }
}
