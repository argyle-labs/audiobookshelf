use plugin_toolkit::prelude::*;

use crate::api::{Item, ProviderBook, SeriesEntry};

#[orca_struct]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeriesDrift {
    NoProviderMatch,
    MissingSeries,
    ExtraSeries,
    SequenceDiffers,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesFinding {
    pub item_id: String,
    pub title: String,
    pub drift: SeriesDrift,
    pub current: Vec<SeriesEntry>,
    pub provider: Vec<SeriesEntry>,
    pub provider_asin: Option<String>,
}

fn norm_name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn norm_seq(s: &Option<String>) -> Option<String> {
    let s = s.as_deref()?.trim();
    if s.is_empty() {
        return None;
    }
    match s.parse::<f64>() {
        Ok(n) => Some(n.to_string()),
        Err(_) => Some(s.to_lowercase()),
    }
}

pub fn best_match<'a>(item: &Item, results: &'a [ProviderBook]) -> Option<&'a ProviderBook> {
    if let Some(asin) = &item.asin {
        if let Some(b) = results.iter().find(|b| {
            b.asin
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(asin))
        }) {
            return Some(b);
        }
    }
    results.first()
}

pub fn diff(item: &Item, provider: Option<&ProviderBook>) -> Option<SeriesFinding> {
    let finding = |drift, p: &[SeriesEntry], asin: Option<String>| SeriesFinding {
        item_id: item.id.clone(),
        title: item.title.clone(),
        drift,
        current: item.series.clone(),
        provider: p.to_vec(),
        provider_asin: asin,
    };
    let Some(p) = provider else {
        return Some(finding(SeriesDrift::NoProviderMatch, &[], None));
    };
    let asin = p.asin.clone();
    for want in &p.series {
        let have = item
            .series
            .iter()
            .find(|h| norm_name(&h.name) == norm_name(&want.name));
        match have {
            None => return Some(finding(SeriesDrift::MissingSeries, &p.series, asin)),
            Some(h)
                if norm_seq(&want.sequence).is_some()
                    && norm_seq(&h.sequence) != norm_seq(&want.sequence) =>
            {
                return Some(finding(SeriesDrift::SequenceDiffers, &p.series, asin));
            }
            _ => {}
        }
    }
    if !p.series.is_empty()
        && item.series.iter().any(|h| {
            !p.series
                .iter()
                .any(|w| norm_name(&w.name) == norm_name(&h.name))
        })
    {
        return Some(finding(SeriesDrift::ExtraSeries, &p.series, asin));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, seq: Option<&str>) -> SeriesEntry {
        SeriesEntry {
            id: None,
            name: name.into(),
            sequence: seq.map(String::from),
        }
    }

    fn item(series: Vec<SeriesEntry>, asin: Option<&str>) -> Item {
        Item {
            id: "li_1".into(),
            library_id: "lib".into(),
            media_type: "book".into(),
            title: "T".into(),
            subtitle: None,
            authors: vec!["A".into()],
            narrators: Vec::new(),
            series,
            asin: asin.map(String::from),
            isbn: None,
            published_year: None,
        }
    }

    fn book(series: Vec<SeriesEntry>, asin: Option<&str>) -> ProviderBook {
        ProviderBook {
            title: "T".into(),
            authors: Vec::new(),
            series,
            asin: asin.map(String::from),
            isbn: None,
        }
    }

    #[test]
    fn agreeing_series_ignores_case_spacing_and_numeric_form() {
        let i = item(vec![s("The  Expanse", Some("1.0"))], None);
        let b = book(vec![s("the expanse", Some("1"))], None);
        assert_eq!(diff(&i, Some(&b)), None);
    }

    #[test]
    fn reports_each_drift_kind() {
        let b = book(vec![s("Dune", Some("2"))], Some("B1"));
        let d = |i: Item| diff(&i, Some(&b)).map(|f| f.drift);
        assert_eq!(d(item(vec![], None)), Some(SeriesDrift::MissingSeries));
        assert_eq!(
            d(item(vec![s("Dune", Some("1"))], None)),
            Some(SeriesDrift::SequenceDiffers)
        );
        assert_eq!(
            d(item(vec![s("Dune", Some("2")), s("Other", None)], None)),
            Some(SeriesDrift::ExtraSeries)
        );
        assert_eq!(
            diff(&item(vec![], None), None).map(|f| f.drift),
            Some(SeriesDrift::NoProviderMatch)
        );
    }

    #[test]
    fn provider_without_series_or_sequence_is_not_drift() {
        let i = item(vec![s("Dune", Some("1"))], None);
        assert_eq!(diff(&i, Some(&book(vec![], None))), None);
        assert_eq!(diff(&i, Some(&book(vec![s("Dune", None)], None))), None);
    }

    #[test]
    fn best_match_prefers_the_items_asin() {
        let i = item(vec![], Some("b2"));
        let r = vec![book(vec![], Some("B1")), book(vec![], Some("B2"))];
        assert_eq!(best_match(&i, &r).unwrap().asin.as_deref(), Some("B2"));
        assert_eq!(
            best_match(&item(vec![], None), &r).unwrap().asin.as_deref(),
            Some("B1")
        );
    }
}
