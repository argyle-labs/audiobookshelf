//! audiobookshelf service backend — Audiobook + podcast server.
//!
//! Implements `ServiceBackend` so the generic `service.*` tools
//! (deploy/backup/restore/configure/status/connect/sync) drive audiobookshelf. No
//! `#[orca_tool]`s — the only orca dep is `plugin-toolkit`. Modeled on the
//! nfs StorageBackend. See orca/docs/PLUGIN-PROGRAM.md.
#![allow(clippy::disallowed_types)]

use plugin_toolkit::service::{
    BoxFuture, Routes, Runtime, ServiceBackend, ServiceCapability, ServiceError, ServiceStatus,
    WorkloadSpec,
};

/// audiobookshelf backend. Holds only the provider name; per-instance routes/creds
/// come from the `Routes` the generic `service.*` tools hand each op.
#[derive(Debug, Clone)]
pub struct AudiobookshelfBackend {
    provider: &'static str,
}

impl AudiobookshelfBackend {
    pub fn new(provider: &'static str) -> Self {
        Self { provider }
    }
}

impl ServiceBackend for AudiobookshelfBackend {
    fn provider(&self) -> &str {
        self.provider
    }

    /// Runtimes audiobookshelf can be placed on. `service.deploy` hands the
    /// `workload_spec` below to a matching deploy target — this backend never
    /// drives pct/docker itself (that mechanic lives in the deploy-target domain).
    fn runtimes(&self) -> Vec<Runtime> {
        vec![Runtime::Docker, Runtime::Podman, Runtime::Lxc, Runtime::Vm]
    }

    fn capabilities(&self) -> Vec<ServiceCapability> {
        vec![
            ServiceCapability::Deploy,
            ServiceCapability::Backup,
            ServiceCapability::Restore,
            ServiceCapability::Configure,
            ServiceCapability::Status,
        ]
    }

    fn default_port(&self) -> u16 {
        13378
    }

    /// In-workload paths holding config/data. This is ALL audiobookshelf declares for
    /// backup — the generic pluggable backup (tar for containers/LXC, PBS for
    /// Proxmox guests when available) snapshots these. No backup/restore code
    /// here; those are inherited from ServiceBackend's defaults.
    fn data_paths(&self) -> Vec<String> {
        vec!["/config".to_string()]
    }

    fn workload_spec<'a>(
        &'a self,
        _runtime: Runtime,
        _instance: &'a str,
        _routes: &'a Routes,
    ) -> BoxFuture<'a, Result<WorkloadSpec, ServiceError>> {
        // TODO: describe the audiobookshelf workload (image/template, ports, mounts,
        // env) for the chosen runtime. The deploy target turns this into a
        // compose service / LXC config / VM. See deploy-target::WorkloadSpec.
        Box::pin(async move { Err(ServiceError::unimplemented("audiobookshelf.workload_spec")) })
    }

    fn configure<'a>(
        &'a self,
        _instance: &'a str,
        _routes: &'a Routes,
        _config: &'a str,
    ) -> BoxFuture<'a, Result<(), ServiceError>> {
        // TODO: apply audiobookshelf-specific config idempotently.
        Box::pin(async move { Err(ServiceError::unimplemented("audiobookshelf.configure")) })
    }

    fn status<'a>(
        &'a self,
        _instance: &'a str,
        _routes: &'a Routes,
    ) -> BoxFuture<'a, Result<ServiceStatus, ServiceError>> {
        // TODO: real health/diagnostics.
        Box::pin(async move { Err(ServiceError::unimplemented("audiobookshelf.status")) })
    }
}

pub mod api;
pub mod audit;
pub mod tools;

use plugin_toolkit::clap;
use plugin_toolkit::media::{
    Capability, MediaBackend, MediaError, MediaIdentity, MediaType, MediaUnit, MediaUrl, SeriesRef,
    Source, SourceKind,
};

#[plugin_toolkit::endpoint_resource(plugin = "audiobookshelf")]
pub struct AudiobookshelfEndpoint {
    pub name: String,
    pub base_url: String,
    #[secret]
    pub token: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct AbsMedia {
    media_type: MediaType,
}

impl AbsMedia {
    pub fn served(media_type: MediaType) -> Self {
        Self { media_type }
    }

    async fn connect(&self) -> Result<api::Abs, MediaError> {
        let rows = endpoint_db::list()
            .map_err(|e| MediaError::Transport(format!("read endpoints: {e}")))?;
        let ep = rows
            .into_iter()
            .find(|r| r.enabled)
            .ok_or_else(|| MediaError::NotFound("no enabled audiobookshelf endpoint".into()))?;
        tools::connect(&ep.name)
            .await
            .map_err(|e| MediaError::Transport(format!("{e:#}")))
    }
}

#[plugin_toolkit::orca_async]
impl MediaBackend for AbsMedia {
    fn name(&self) -> &str {
        "audiobookshelf"
    }
    fn media_type(&self) -> MediaType {
        self.media_type
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::ServedBy, Capability::Url, Capability::Units]
    }
    fn endpoint(&self) -> String {
        String::new()
    }

    async fn url(&self) -> Result<MediaUrl, MediaError> {
        Ok(MediaUrl {
            primary: self.connect().await?.base_url().to_string(),
            alternates: Vec::new(),
        })
    }

    async fn units(&self) -> Result<Vec<MediaUnit>, MediaError> {
        let abs = self.connect().await?;
        units(&abs, self.media_type)
            .await
            .map_err(|e| MediaError::Transport(format!("{e:#}")))
    }
}

pub fn abs_media_type(media_type: MediaType) -> Option<&'static str> {
    match media_type {
        MediaType::Audiobooks => Some("book"),
        MediaType::Podcasts => Some("podcast"),
        _ => None,
    }
}

pub async fn units(
    abs: &api::Abs,
    media_type: MediaType,
) -> plugin_toolkit::anyhow::Result<Vec<MediaUnit>> {
    let Some(want) = abs_media_type(media_type) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for lib in abs
        .libraries()
        .await?
        .into_iter()
        .filter(|l| l.media_type == want)
    {
        for item in abs.items(&lib.id).await? {
            if let Some(u) = to_unit(abs.base_url(), &item, media_type) {
                out.push(u);
            }
        }
    }
    Ok(out)
}

pub fn to_unit(base_url: &str, item: &api::Item, media_type: MediaType) -> Option<MediaUnit> {
    if item.title.is_empty() {
        return None;
    }
    let mut identity = MediaIdentity {
        title: item.title.clone(),
        year: item.published_year.as_deref().and_then(|y| y.parse().ok()),
        external_ids: Vec::new(),
        series: item.series.first().map(|s| SeriesRef {
            name: s.name.clone(),
            sequence: s.sequence.clone(),
        }),
    };
    if let Some(asin) = &item.asin {
        identity = identity.with_external_id("asin", asin, media_type);
    }
    if let Some(isbn) = &item.isbn {
        identity = identity.with_external_id("isbn", isbn, media_type);
    }
    Some(MediaUnit {
        media_type,
        identity,
        variants: Vec::new(),
        sources: vec![Source {
            method: SourceKind::AppStream,
            by: "audiobookshelf".to_string(),
            url: Some(format!(
                "{}/item/{}",
                base_url.trim_end_matches('/'),
                item.id
            )),
        }],
        downloaded_by: Vec::new(),
        served_by: vec!["audiobookshelf".to_string()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_toolkit::serde_json::{self, json};
    use std::sync::{Arc, Mutex};

    #[test]
    fn declares_provider() {
        let b = AudiobookshelfBackend::new("audiobookshelf");
        assert_eq!(b.provider(), "audiobookshelf");
    }

    #[test]
    fn media_facet_advertises_served_by_url_and_units() {
        let m = AbsMedia::served(MediaType::Audiobooks);
        assert_eq!(m.name(), "audiobookshelf");
        assert_eq!(m.media_type(), MediaType::Audiobooks);
        for cap in [Capability::ServedBy, Capability::Url, Capability::Units] {
            assert!(m.capabilities().contains(&cap), "missing {cap:?}");
        }
    }

    fn serve<F>(routes: Vec<(&'static str, serde_json::Value)>, body: F) -> Vec<String>
    where
        F: FnOnce(),
    {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        plugin_toolkit::capsink::with_cap_sink(
            Box::new(move |cap: &str, req: &str| {
                assert_eq!(cap, "http.request");
                let req: serde_json::Value = serde_json::from_str(req).unwrap();
                let url = req["url"].as_str().unwrap().to_string();
                let auth = req["headers"].as_array().unwrap().iter().any(|h| {
                    h[0].as_str().unwrap().eq_ignore_ascii_case("authorization")
                        && h[1] == "Bearer tok"
                });
                assert!(auth, "missing bearer on {url}");
                log.lock()
                    .unwrap()
                    .push(format!("{} {url}", req["method"].as_str().unwrap()));
                let path = url.trim_start_matches("http://abs.test");
                let body = routes
                    .iter()
                    .find(|(p, _)| path == *p || path.starts_with(&format!("{p}?")))
                    .map(|(_, b)| b.to_string());
                let (status, body) = match body {
                    Some(b) => (200, b),
                    None => (404, "Not Found".to_string()),
                };
                Ok(json!({"status": status, "headers": [], "body": body.into_bytes()}).to_string())
            }),
            body,
        );
        let out = seen.lock().unwrap().clone();
        out
    }

    fn block<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn abs() -> api::Abs {
        api::Abs::new("http://abs.test/", "tok", false).unwrap()
    }

    #[test]
    fn units_map_book_library_items_over_delegated_http() {
        let mut got = Vec::new();
        let calls = serve(
            vec![
                (
                    "/api/libraries",
                    json!({"libraries": [
                        {"id": "lib_books", "mediaType": "book", "provider": "audible"},
                        {"id": "lib_pods", "mediaType": "podcast"}
                    ]}),
                ),
                (
                    "/api/libraries/lib_books/items",
                    json!({"total": 1, "results": [{
                        "id": "li_wok", "libraryId": "lib_books",
                        "media": {"metadata": {
                            "title": "The Way of Kings",
                            "asin": "B0041JKFJW",
                            "isbn": "0-7653-2635-5",
                            "seriesName": "The Stormlight Archive #1"
                        }}
                    }]}),
                ),
            ],
            || got = block(units(&abs(), MediaType::Audiobooks)).unwrap(),
        );
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(calls[1].contains("limit=500"), "{calls:?}");
        assert_eq!(got.len(), 1);
        let u = &got[0];
        assert_eq!(u.identity.title, "The Way of Kings");
        let s = u.identity.series.as_ref().unwrap();
        assert_eq!(s.name, "The Stormlight Archive");
        assert_eq!(s.sequence.as_deref(), Some("1"));
        let ids: Vec<(&str, &str)> = u
            .identity
            .external_ids
            .iter()
            .map(|e| (e.source.as_str(), e.id.as_str()))
            .collect();
        assert!(ids.contains(&("asin", "B0041JKFJW")), "{ids:?}");
        assert!(ids.contains(&("isbn", "9780765326355")), "{ids:?}");
        assert_eq!(
            u.sources[0].url.as_deref(),
            Some("http://abs.test/item/li_wok")
        );
    }

    #[test]
    fn units_are_empty_for_a_type_abs_does_not_serve() {
        let calls = serve(vec![], || {
            assert!(block(units(&abs(), MediaType::Movies)).unwrap().is_empty())
        });
        assert!(calls.is_empty());
    }

    #[test]
    fn http_errors_surface_with_status() {
        serve(vec![], || {
            let e = block(abs().libraries()).unwrap_err().to_string();
            assert!(e.contains("HTTP 404"), "{e}");
        });
    }

    #[test]
    fn search_uses_asin_and_library_provider() {
        let calls = serve(
            vec![(
                "/api/search/books",
                json!([{"title": "Dune", "asin": "B002V1OF70", "series": [{"series": "Dune", "sequence": "1"}]}]),
            )],
            || {
                let mut it = api::parse_item(
                    &json!({"id": "li_1", "media": {"metadata": {"title": "Dune", "asin": "B002V1OF70"}}}),
                    "lib",
                );
                it.authors = vec!["Frank Herbert".into()];
                let r = block(abs().search_books(&it, "audible")).unwrap();
                assert_eq!(r[0].series[0].name, "Dune");
            },
        );
        assert!(calls[0].contains("provider=audible"), "{calls:?}");
        assert!(calls[0].contains("title=B002V1OF70"), "{calls:?}");
    }

    #[test]
    fn update_patches_item_media() {
        let calls = serve(
            vec![("/api/items/li_1/media", json!({"updated": true}))],
            || {
                let patch = api::MetadataPatch {
                    title: Some("X".into()),
                    ..Default::default()
                };
                assert!(block(abs().update_metadata("li_1", &patch)).unwrap());
            },
        );
        assert_eq!(calls, vec!["PATCH http://abs.test/api/items/li_1/media"]);
    }
}
