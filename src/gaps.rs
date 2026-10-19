use std::collections::{BTreeMap, BTreeSet};

use plugin_toolkit::prelude::*;

use crate::api::Item;

const GAP_MAX_NUMBER: i64 = 1000;

#[orca_struct]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqRange {
    pub from: i64,
    pub to: i64,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookRef {
    pub item_id: String,
    pub title: String,
    pub sequence: Option<String>,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesGaps {
    pub series_id: String,
    pub name: String,
    pub library_id: String,
    pub books: Vec<BookRef>,
    pub sequences: Vec<String>,
    pub gaps: Vec<SeqRange>,
    pub duplicates: Vec<String>,
    pub unsequenced: Vec<BookRef>,
    pub omnibus: Vec<String>,
}

impl SeriesGaps {
    pub fn has_findings(&self) -> bool {
        !(self.gaps.is_empty() && self.duplicates.is_empty() && self.unsequenced.is_empty())
    }
}

#[orca_struct]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GapsSummary {
    pub libraries: usize,
    pub series: usize,
    pub books: usize,
    pub series_with_gaps: usize,
    pub missing: i64,
    pub series_with_duplicates: usize,
    pub duplicate_sequences: usize,
    pub unsequenced_books: usize,
    pub omnibus_books: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Seq {
    Number(f64),
    Range(i64, i64),
    Other(String),
}

fn parse_seq(s: &str) -> Seq {
    let s = s.trim();
    if let Ok(n) = s.parse::<f64>() {
        if n.is_finite() {
            return Seq::Number(n);
        }
    }
    if let Some((a, b)) = s.split_once(['-', '–']) {
        if let (Ok(a), Ok(b)) = (a.trim().parse::<i64>(), b.trim().parse::<i64>()) {
            if a <= b {
                return Seq::Range(a, b);
            }
        }
    }
    Seq::Other(s.to_lowercase())
}

fn seq_key(s: &Seq) -> String {
    match s {
        Seq::Number(n) => n.to_string(),
        Seq::Range(a, b) => format!("{a}-{b}"),
        Seq::Other(o) => o.clone(),
    }
}

fn seq_order(s: &Seq) -> (u8, f64) {
    match s {
        Seq::Number(n) => (0, *n),
        Seq::Range(a, _) => (0, *a as f64),
        Seq::Other(_) => (1, 0.0),
    }
}

pub fn analyze(
    series_id: &str,
    name: &str,
    library_id: &str,
    mut books: Vec<BookRef>,
) -> SeriesGaps {
    let parsed: Vec<Option<Seq>> = books
        .iter()
        .map(|b| {
            b.sequence
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(parse_seq)
        })
        .collect();
    let mut order: Vec<usize> = (0..books.len()).collect();
    order.sort_by(|&a, &b| match (&parsed[a], &parsed[b]) {
        (Some(x), Some(y)) => seq_order(x)
            .0
            .cmp(&seq_order(y).0)
            .then(seq_order(x).1.total_cmp(&seq_order(y).1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => books[a].title.cmp(&books[b].title),
    });
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut covered: BTreeSet<i64> = BTreeSet::new();
    let mut sequences = Vec::new();
    let mut duplicates = Vec::new();
    let mut omnibus = Vec::new();
    for &i in &order {
        let Some(seq) = &parsed[i] else { continue };
        let key = seq_key(seq);
        let n = counts.entry(key.clone()).or_default();
        *n += 1;
        if *n == 2 {
            duplicates.push(key.clone());
        }
        sequences.push(key.clone());
        match seq {
            Seq::Number(v) if v.fract() == 0.0 && *v >= 0.0 && *v < GAP_MAX_NUMBER as f64 => {
                covered.insert(*v as i64);
            }
            Seq::Range(a, b) if *a >= 0 && *b < GAP_MAX_NUMBER => {
                covered.extend(*a..=*b);
                omnibus.push(key);
            }
            _ => {}
        }
    }
    let mut gaps: Vec<SeqRange> = Vec::new();
    if let (Some(&first), Some(&last)) = (covered.first(), covered.last()) {
        let lo = if first <= 5 { 1 } else { first };
        for n in (lo..=last).filter(|n| !covered.contains(n)) {
            match gaps.last_mut() {
                Some(r) if r.to == n - 1 => r.to = n,
                _ => gaps.push(SeqRange { from: n, to: n }),
            }
        }
    }
    books = order.iter().map(|&i| books[i].clone()).collect();
    let unsequenced = books
        .iter()
        .filter(|b| b.sequence.as_deref().is_none_or(|s| s.trim().is_empty()))
        .cloned()
        .collect();
    SeriesGaps {
        series_id: series_id.to_string(),
        name: name.to_string(),
        library_id: library_id.to_string(),
        books,
        sequences,
        gaps,
        duplicates,
        unsequenced,
        omnibus,
    }
}

pub fn group(items: &[Item]) -> BTreeMap<(String, String), Vec<BookRef>> {
    let mut out: BTreeMap<(String, String), Vec<BookRef>> = BTreeMap::new();
    for item in items {
        for s in &item.series {
            out.entry((item.library_id.clone(), s.name.clone()))
                .or_default()
                .push(BookRef {
                    item_id: item.id.clone(),
                    title: item.title.clone(),
                    sequence: s.sequence.clone(),
                });
        }
    }
    out
}

pub fn summarize(libraries: usize, series: &[SeriesGaps]) -> GapsSummary {
    let mut s = GapsSummary {
        libraries,
        series: series.len(),
        ..Default::default()
    };
    for g in series {
        s.books += g.books.len();
        s.series_with_gaps += usize::from(!g.gaps.is_empty());
        s.missing += g.gaps.iter().map(|r| r.to - r.from + 1).sum::<i64>();
        s.series_with_duplicates += usize::from(!g.duplicates.is_empty());
        s.duplicate_sequences += g.duplicates.len();
        s.unsequenced_books += g.unsequenced.len();
        s.omnibus_books += g.omnibus.len();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seqs: &[Option<&str>]) -> SeriesGaps {
        let books = seqs
            .iter()
            .enumerate()
            .map(|(i, s)| BookRef {
                item_id: format!("li_{i}"),
                title: format!("T{i}"),
                sequence: s.map(String::from),
            })
            .collect();
        analyze("s1", "S", "lib", books)
    }

    fn r(from: i64, to: i64) -> SeqRange {
        SeqRange { from, to }
    }

    #[test]
    fn reports_missing_runs_from_one() {
        let g = run(&[Some("2"), Some("5"), Some("6"), Some("9")]);
        assert_eq!(g.gaps, vec![r(1, 1), r(3, 4), r(7, 8)]);
        assert_eq!(g.sequences, vec!["2", "5", "6", "9"]);
        assert!(g.has_findings());
    }

    #[test]
    fn complete_run_has_no_findings() {
        let g = run(&[Some("3"), Some("1"), Some("2")]);
        assert!(g.gaps.is_empty());
        assert!(!g.has_findings());
        assert_eq!(g.books[0].sequence.as_deref(), Some("1"));
    }

    #[test]
    fn duplicates_compare_numerically() {
        let g = run(&[Some("1"), Some("1.0"), Some("2"), Some("2"), Some("2")]);
        assert_eq!(g.duplicates, vec!["1", "2"]);
        assert!(g.gaps.is_empty());
    }

    #[test]
    fn decimals_are_novellas_not_gaps() {
        let g = run(&[Some("1"), Some("1.5"), Some("2"), Some("2.5"), Some("4")]);
        assert_eq!(g.gaps, vec![r(3, 3)]);
        assert!(g.duplicates.is_empty());
        assert_eq!(g.sequences, vec!["1", "1.5", "2", "2.5", "4"]);
    }

    #[test]
    fn decimals_alone_report_no_gaps() {
        assert!(run(&[Some("0.5"), Some("1.5")]).gaps.is_empty());
    }

    #[test]
    fn ranges_are_omnibus_and_cover_their_span() {
        let g = run(&[Some("1-3"), Some("4"), Some("6"), Some("7–9")]);
        assert_eq!(g.omnibus, vec!["1-3", "7-9"]);
        assert_eq!(g.gaps, vec![r(5, 5)]);
    }

    #[test]
    fn unsequenced_books_are_listed() {
        let g = run(&[Some("1"), None, Some(" "), Some("2")]);
        assert_eq!(g.unsequenced.len(), 2);
        assert_eq!(g.sequences, vec!["1", "2"]);
        assert!(g.gaps.is_empty());
        assert!(g.has_findings());
    }

    #[test]
    fn gaps_start_at_lowest_when_above_five() {
        assert_eq!(run(&[Some("6"), Some("8")]).gaps, vec![r(7, 7)]);
        assert_eq!(run(&[Some("5"), Some("6")]).gaps, vec![r(1, 4)]);
    }

    #[test]
    fn numbers_from_one_thousand_are_ignored() {
        assert_eq!(
            run(&[Some("1"), Some("3"), Some("1000")]).gaps,
            vec![r(2, 2)]
        );
        assert_eq!(run(&[Some("1"), Some("999")]).gaps, vec![r(2, 998)]);
    }

    #[test]
    fn year_like_numbers_do_not_explode_gaps() {
        let g = run(&[Some("1"), Some("2019")]);
        assert!(g.gaps.is_empty());
    }

    #[test]
    fn summary_totals_missing_counts() {
        let a = run(&[Some("1"), Some("4")]);
        let b = run(&[Some("1"), Some("1"), None, Some("1-2")]);
        let s = summarize(1, &[a, b]);
        assert_eq!(s.series, 2);
        assert_eq!(s.books, 6);
        assert_eq!(s.missing, 2);
        assert_eq!(s.series_with_gaps, 1);
        assert_eq!(s.duplicate_sequences, 1);
        assert_eq!(s.unsequenced_books, 1);
        assert_eq!(s.omnibus_books, 1);
    }
}
