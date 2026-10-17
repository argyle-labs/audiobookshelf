#![allow(clippy::disallowed_types)]

use plugin_toolkit::api_client::ApiClientBuilder;
use plugin_toolkit::prelude::*;
use plugin_toolkit::progenitor_client::QueryParam;
use plugin_toolkit::reqwest::{Client as HttpClient, RequestBuilder};
use plugin_toolkit::serde_json::{self, Value};

const PAGE: usize = 500;

pub struct Abs {
    base_url: String,
    http: HttpClient,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub provider: String,
    pub folders: Vec<String>,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SeriesEntry {
    pub name: String,
    pub sequence: Option<String>,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub library_id: String,
    pub media_type: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub series: Vec<SeriesEntry>,
    pub asin: Option<String>,
    pub isbn: Option<String>,
    pub published_year: Option<String>,
    pub path: Option<String>,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub id: String,
    pub name: String,
    pub library_id: String,
    pub book_count: usize,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderBook {
    pub title: String,
    pub authors: Vec<String>,
    pub series: Vec<SeriesEntry>,
    pub asin: Option<String>,
    pub isbn: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(crate = "plugin_toolkit::serde")]
pub struct MetadataPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<NameOnly>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<SeriesEntry>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(crate = "plugin_toolkit::serde")]
pub struct NameOnly {
    pub name: String,
}

impl Abs {
    pub fn new(base_url: &str, token: &str, insecure: bool) -> Result<Self> {
        let http = ApiClientBuilder::new()
            .bearer(token)?
            .insecure(insecure)
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn send(&self, label: String, req: RequestBuilder) -> Result<Value> {
        let resp = req.send().await.map_err(|e| anyhow!("{label}: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| anyhow!("{label}: {e}"))?;
        if !status.is_success() {
            bail!("{label}: HTTP {} {}", status.as_u16(), body.trim());
        }
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&body).unwrap_or(Value::String(body)))
    }

    async fn get_json(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        let req = query
            .iter()
            .fold(self.http.get(self.url(path)), |r, (k, v)| {
                r.query(&QueryParam::new(k, v))
            });
        self.send(format!("GET {path}"), req).await
    }

    async fn send_json(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let req = match method {
            "POST" => self.http.post(self.url(path)),
            "PATCH" => self.http.patch(self.url(path)),
            _ => bail!("unsupported method {method}"),
        };
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        self.send(format!("{method} {path}"), req).await
    }

    pub async fn libraries(&self) -> Result<Vec<Library>> {
        let v = self.get_json("/api/libraries", &[]).await?;
        Ok(arr(&v["libraries"]).iter().map(parse_library).collect())
    }

    pub async fn library(&self, id: &str) -> Result<Library> {
        check_id(id)?;
        let v = self.get_json(&format!("/api/libraries/{id}"), &[]).await?;
        Ok(parse_library(v.get("library").unwrap_or(&v)))
    }

    pub async fn scan(&self, library_id: &str, force: bool) -> Result<()> {
        check_id(library_id)?;
        let path = if force {
            format!("/api/libraries/{library_id}/scan?force=1")
        } else {
            format!("/api/libraries/{library_id}/scan")
        };
        self.send_json("POST", &path, None).await.map(|_| ())
    }

    pub async fn match_library(&self, library_id: &str) -> Result<()> {
        check_id(library_id)?;
        self.get_json(&format!("/api/libraries/{library_id}/matchall"), &[])
            .await
            .map(|_| ())
    }

    pub async fn items(&self, library_id: &str) -> Result<Vec<Item>> {
        check_id(library_id)?;
        let path = format!("/api/libraries/{library_id}/items");
        let mut out = Vec::new();
        for page in 0.. {
            let v = self
                .get_json(
                    &path,
                    &[("limit", PAGE.to_string()), ("page", page.to_string())],
                )
                .await?;
            let results = arr(&v["results"]);
            out.extend(results.iter().map(|i| parse_item(i, library_id)));
            let total = v["total"].as_u64().unwrap_or(0) as usize;
            if results.len() < PAGE || out.len() >= total {
                break;
            }
        }
        Ok(out)
    }

    pub async fn item(&self, item_id: &str) -> Result<Item> {
        check_id(item_id)?;
        let v = self
            .get_json(
                &format!("/api/items/{item_id}"),
                &[("expanded", "1".into())],
            )
            .await?;
        let lib = v["libraryId"].as_str().unwrap_or_default().to_string();
        Ok(parse_item(&v, &lib))
    }

    pub async fn series(&self, library_id: &str) -> Result<Vec<Series>> {
        check_id(library_id)?;
        let path = format!("/api/libraries/{library_id}/series");
        let mut out = Vec::new();
        for page in 0.. {
            let v = self
                .get_json(
                    &path,
                    &[("limit", PAGE.to_string()), ("page", page.to_string())],
                )
                .await?;
            let results = arr(&v["results"]);
            out.extend(results.iter().map(|s| Series {
                id: str_of(&s["id"]),
                name: str_of(&s["name"]),
                library_id: s["libraryId"].as_str().unwrap_or(library_id).to_string(),
                book_count: arr(&s["books"]).len(),
            }));
            let total = v["total"].as_u64().unwrap_or(0) as usize;
            if results.len() < PAGE || out.len() >= total {
                break;
            }
        }
        Ok(out)
    }

    pub async fn update_metadata(&self, item_id: &str, patch: &MetadataPatch) -> Result<bool> {
        check_id(item_id)?;
        let body = json!({ "metadata": patch });
        let v = self
            .send_json("PATCH", &format!("/api/items/{item_id}/media"), Some(body))
            .await?;
        Ok(v["updated"].as_bool().unwrap_or(false))
    }

    pub async fn match_item(&self, item: &Item, provider: &str) -> Result<bool> {
        check_id(&item.id)?;
        let mut body = json!({
            "provider": provider,
            "title": item.title,
            "author": item.authors.first().cloned().unwrap_or_default(),
        });
        if let Some(asin) = &item.asin {
            body["asin"] = json!(asin);
        }
        if let Some(isbn) = &item.isbn {
            body["isbn"] = json!(isbn);
        }
        let v = self
            .send_json("POST", &format!("/api/items/{}/match", item.id), Some(body))
            .await?;
        Ok(v["updated"].as_bool().unwrap_or(false))
    }

    pub async fn search_books(&self, item: &Item, provider: &str) -> Result<Vec<ProviderBook>> {
        let title = item.asin.clone().unwrap_or_else(|| item.title.clone());
        let v = self
            .get_json(
                "/api/search/books",
                &[
                    ("provider", provider.to_string()),
                    ("title", title),
                    ("author", item.authors.first().cloned().unwrap_or_default()),
                    ("id", item.id.clone()),
                ],
            )
            .await?;
        Ok(arr(&v).iter().map(parse_provider_book).collect())
    }
}

pub fn check_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("invalid audiobookshelf id {id:?}");
    }
    Ok(())
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn opt_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn names(list: &Value, joined: &Value) -> Vec<String> {
    let from_list: Vec<String> = arr(list)
        .iter()
        .filter_map(|a| opt_str(&a["name"]).or_else(|| opt_str(a)))
        .collect();
    if !from_list.is_empty() {
        return from_list;
    }
    joined
        .as_str()
        .map(|s| {
            s.split(", ")
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_series_name(joined: &str) -> Vec<SeriesEntry> {
    joined
        .split(", ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s.rsplit_once(" #") {
            Some((name, seq)) if !seq.trim().is_empty() => SeriesEntry {
                name: name.trim().to_string(),
                sequence: Some(seq.trim().to_string()),
            },
            _ => SeriesEntry {
                name: s.to_string(),
                sequence: None,
            },
        })
        .collect()
}

fn series_of(md: &Value) -> Vec<SeriesEntry> {
    let list = arr(&md["series"]);
    if !list.is_empty() {
        return list
            .iter()
            .filter_map(|s| {
                let name = opt_str(&s["name"]).or_else(|| opt_str(&s["series"]))?;
                Some(SeriesEntry {
                    name,
                    sequence: opt_str(&s["sequence"]),
                })
            })
            .collect();
    }
    if let Some(s) = md["series"].as_str() {
        return parse_series_name(s);
    }
    md["seriesName"]
        .as_str()
        .map(parse_series_name)
        .unwrap_or_default()
}

fn parse_library(l: &Value) -> Library {
    Library {
        id: str_of(&l["id"]),
        name: str_of(&l["name"]),
        media_type: str_of(&l["mediaType"]),
        provider: str_of(&l["provider"]),
        folders: arr(&l["folders"])
            .iter()
            .filter_map(|f| opt_str(&f["fullPath"]))
            .collect(),
    }
}

pub fn parse_item(i: &Value, library_id: &str) -> Item {
    let md = &i["media"]["metadata"];
    Item {
        id: str_of(&i["id"]),
        library_id: i["libraryId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(library_id)
            .to_string(),
        media_type: str_of(&i["mediaType"]),
        title: str_of(&md["title"]),
        subtitle: opt_str(&md["subtitle"]),
        authors: names(&md["authors"], &md["authorName"]),
        narrators: names(&md["narrators"], &md["narratorName"]),
        series: series_of(md),
        asin: opt_str(&md["asin"]),
        isbn: opt_str(&md["isbn"]),
        published_year: opt_str(&md["publishedYear"]),
        path: opt_str(&i["path"]),
    }
}

pub fn parse_provider_book(b: &Value) -> ProviderBook {
    ProviderBook {
        title: str_of(&b["title"]),
        authors: b["author"]
            .as_str()
            .map(|s| {
                s.split(", ")
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        series: series_of(b),
        asin: opt_str(&b["asin"]),
        isbn: opt_str(&b["isbn"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minified_item_splits_joined_series_and_authors() {
        let v = json!({
            "id": "li_1", "libraryId": "lib_1", "mediaType": "book",
            "media": {"metadata": {
                "title": "Oathbringer",
                "authorName": "Brandon Sanderson",
                "seriesName": "The Stormlight Archive #3, Cosmere #7.5",
                "asin": "B06XKFW3P3"
            }}
        });
        let it = parse_item(&v, "x");
        assert_eq!(it.library_id, "lib_1");
        assert_eq!(it.authors, vec!["Brandon Sanderson"]);
        assert_eq!(
            it.series,
            vec![
                SeriesEntry {
                    name: "The Stormlight Archive".into(),
                    sequence: Some("3".into())
                },
                SeriesEntry {
                    name: "Cosmere".into(),
                    sequence: Some("7.5".into())
                },
            ]
        );
        assert_eq!(it.asin.as_deref(), Some("B06XKFW3P3"));
        assert_eq!(it.isbn, None);
    }

    #[test]
    fn expanded_item_reads_structured_series_and_authors() {
        let v = json!({
            "id": "li_2", "libraryId": "lib_1",
            "media": {"metadata": {
                "title": "Dune",
                "authors": [{"id": "a1", "name": "Frank Herbert"}],
                "series": [{"id": "s1", "name": "Dune", "sequence": "1"}, {"id": "s2", "name": "Saga", "sequence": null}]
            }}
        });
        let it = parse_item(&v, "");
        assert_eq!(it.authors, vec!["Frank Herbert"]);
        assert_eq!(it.series[0].sequence.as_deref(), Some("1"));
        assert_eq!(it.series[1].sequence, None);
    }

    #[test]
    fn provider_book_reads_series_series_field() {
        let b = parse_provider_book(&json!({
            "title": "Dune", "author": "Frank Herbert, Someone", "asin": "B002V1OF70",
            "series": [{"series": "Dune", "sequence": "1"}]
        }));
        assert_eq!(b.authors.len(), 2);
        assert_eq!(b.series[0].name, "Dune");
        assert_eq!(b.series[0].sequence.as_deref(), Some("1"));
    }

    #[test]
    fn ids_reject_path_traversal() {
        assert!(check_id("li_abc-123").is_ok());
        assert!(check_id("../etc").is_err());
        assert!(check_id("").is_err());
    }
}
