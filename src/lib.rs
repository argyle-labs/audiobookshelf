//! audiobookshelf service backend — Audiobook + podcast server.
//!
//! Implements `ServiceBackend` so the generic `service.*` tools
//! (deploy/backup/restore/configure/status/connect/sync) drive audiobookshelf. No
//! `#[orca_tool]`s — the only orca dep is `plugin-toolkit`. Modeled on the
//! nfs StorageBackend. See orca/docs/PLUGIN-PROGRAM.md.
#![allow(clippy::disallowed_types)]

use plugin_toolkit::service::{
    BoxFuture, Endpoint, Runtime, ServiceBackend, ServiceCapability, ServiceError, ServiceStatus,
    WorkloadSpec,
};

/// audiobookshelf backend. Holds only the provider name; per-instance endpoint/creds
/// come from the `Endpoint` the generic `service.*` tools hand each op.
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
        _ep: &'a Endpoint,
    ) -> BoxFuture<'a, Result<WorkloadSpec, ServiceError>> {
        // TODO: describe the audiobookshelf workload (image/template, ports, mounts,
        // env) for the chosen runtime. The deploy target turns this into a
        // compose service / LXC config / VM. See deploy-target::WorkloadSpec.
        Box::pin(async move { Err(ServiceError::unimplemented("audiobookshelf.workload_spec")) })
    }

    fn configure<'a>(
        &'a self,
        _ep: &'a Endpoint,
        _config: &'a str,
    ) -> BoxFuture<'a, Result<(), ServiceError>> {
        // TODO: apply audiobookshelf-specific config idempotently.
        Box::pin(async move { Err(ServiceError::unimplemented("audiobookshelf.configure")) })
    }

    fn status<'a>(
        &'a self,
        _ep: &'a Endpoint,
    ) -> BoxFuture<'a, Result<ServiceStatus, ServiceError>> {
        // TODO: real health/diagnostics.
        Box::pin(async move { Err(ServiceError::unimplemented("audiobookshelf.status")) })
    }
}

// ── Media served_by facet ────────────────────────────────────────────────────
//
// Audiobookshelf is not only a deployable *service* — it *serves* audiobooks and
// podcasts. It registers a `media` backend per served type so orca's generic
// `media served-by` surface resolves the reachable URL (and, later, per-user
// credentials) for device setup from ONE place. See orca#408 / #404.

use plugin_toolkit::clap; // the endpoint_resource! tools emit unqualified `clap::` paths
use plugin_toolkit::http::{Client as HttpClient, Response};
use plugin_toolkit::media::{
    Capability, MediaBackend, MediaError, MediaIdentity, MediaType, MediaUnit, MediaUrl, SeriesRef,
    Source, SourceKind,
};
use plugin_toolkit::serde;

/// Endpoint registry for the audiobookshelf server orca talks to: `(base_url,
/// token)` keyed by `name`. `endpoint_resource!` emits the row struct, the
/// `endpoint_db` accessors, schema fragment, and the
/// `audiobookshelf.{list,detail,create,update,delete}` CRUD tools in one shot —
/// the same pattern jellyfin/plex use. The media facet resolves its URL from an
/// enabled row here at call time.
#[plugin_toolkit::endpoint_resource(plugin = "audiobookshelf")]
pub struct AudiobookshelfEndpoint {
    pub name: String,
    pub base_url: String,
    #[secret]
    pub token: String,
    pub enabled: bool,
}

/// A media `served_by` backend for one media type Audiobookshelf serves. One is
/// registered per served type (audiobooks, podcasts) — the builder gives each a
/// type-qualified invoke prefix so they never collide.
#[derive(Debug, Clone)]
pub struct AbsMedia {
    media_type: MediaType,
}

impl AbsMedia {
    /// Register Audiobookshelf as the server (`served_by`) for `media_type`.
    pub fn served(media_type: MediaType) -> Self {
        Self { media_type }
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
    /// A server that hands out a reachable URL + a converged library view.
    /// `credentials` (per-user, orca-managed) lands with the served_by credential
    /// brokerage (orca#406).
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::ServedBy, Capability::Url, Capability::Units]
    }
    /// Built at startup (outside the capability sink), so this must not touch the
    /// db — the real reachable URL is resolved lazily in [`url`](Self::url).
    fn endpoint(&self) -> String {
        String::new()
    }

    /// Resolve the reachable base URL from the first enabled configured endpoint.
    /// Runs inside an `Invoke` (cap sink active), so the `endpoint_db` read is
    /// valid here. `media served-by --media-type audiobooks` returns this URL.
    async fn url(&self) -> Result<MediaUrl, MediaError> {
        let cfg = self.resolve_config()?;
        Ok(MediaUrl {
            primary: cfg.base_url,
            alternates: Vec::new(),
        })
    }

    /// This backend's partial convergence view: every library item Audiobookshelf
    /// serves for this media type, as a [`MediaUnit`] carrying its identity
    /// (title + series + ASIN/ISBN external ids, normalized via the media
    /// identity helpers) and an `AppStream` source pointing at the item. Core
    /// merges these with other backends' views (see `media.unit-list`).
    async fn units(&self) -> Result<Vec<MediaUnit>, MediaError> {
        let cfg = self.resolve_config()?;
        AbsClient::new(&cfg).units(self.media_type).await
    }
}

impl AbsMedia {
    /// Load the first enabled configured endpoint as an [`AbsConfig`]. The
    /// `endpoint_db` read is only valid inside an `Invoke` (cap sink active), so
    /// this is called from the async verbs, never from `endpoint()`.
    fn resolve_config(&self) -> Result<AbsConfig, MediaError> {
        let rows = endpoint_db::list()
            .map_err(|e| MediaError::Transport(format!("read endpoints: {e}")))?;
        let ep = rows
            .into_iter()
            .find(|r| r.enabled)
            .ok_or_else(|| MediaError::NotFound("no enabled audiobookshelf endpoint".into()))?;
        Ok(AbsConfig {
            base_url: ep.base_url,
            token: ep.token,
        })
    }
}

// ── Audiobookshelf HTTP client (library view) ────────────────────────────────

/// Resolved connection to one Audiobookshelf server.
struct AbsConfig {
    base_url: String,
    token: String,
}

/// Minimal Audiobookshelf API client: Bearer-token GET + JSON decode, enough for
/// the `units()` library view. Grows request-by-request as more verbs land.
struct AbsClient<'a> {
    cfg: &'a AbsConfig,
    http: HttpClient,
}

impl<'a> AbsClient<'a> {
    fn new(cfg: &'a AbsConfig) -> Self {
        Self {
            cfg,
            http: HttpClient::new(),
        }
    }

    async fn get(&self, path: &str) -> Result<Response, MediaError> {
        self.http
            .get(format!(
                "{}{}",
                self.cfg.base_url.trim_end_matches('/'),
                path
            ))
            .header("authorization", format!("Bearer {}", self.cfg.token))
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| MediaError::Transport(format!("GET {path}: {e}")))
    }

    async fn libraries(&self) -> Result<Vec<AbsLibrary>, MediaError> {
        let resp = self.get("/api/libraries").await?;
        Ok(resp
            .json::<LibrariesResp>()
            .map_err(|e| MediaError::Transport(format!("decode libraries: {e}")))?
            .libraries)
    }

    async fn library_items(&self, library_id: &str) -> Result<Vec<AbsItem>, MediaError> {
        let resp = self
            .get(&format!("/api/libraries/{library_id}/items"))
            .await?;
        Ok(resp
            .json::<ItemsResp>()
            .map_err(|e| MediaError::Transport(format!("decode items: {e}")))?
            .results)
    }

    /// The convergence view for `media_type`: every item in the matching-kind
    /// libraries (ABS `mediaType` book/podcast), mapped to a [`MediaUnit`] with
    /// title + series + ASIN/ISBN identity and an `AppStream` source. The
    /// `MediaBackend::units` verb delegates here so the whole libraries→items→
    /// map path is exercised against a mock server without the endpoint db.
    async fn units(&self, media_type: MediaType) -> Result<Vec<MediaUnit>, MediaError> {
        let want = match media_type {
            MediaType::Audiobooks => "book",
            MediaType::Podcasts => "podcast",
            _ => return Ok(Vec::new()),
        };
        let mut units = Vec::new();
        for lib in self
            .libraries()
            .await?
            .into_iter()
            .filter(|l| l.media_type == want)
        {
            for item in self.library_items(&lib.id).await? {
                let md = item.media.metadata;
                let Some(title) = md.title.filter(|t| !t.is_empty()) else {
                    continue;
                };
                let mut identity = MediaIdentity {
                    title,
                    year: None,
                    external_ids: Vec::new(),
                    series: md.series_name.map(|name| SeriesRef {
                        name,
                        sequence: None,
                    }),
                };
                if let Some(asin) = md.asin.filter(|s| !s.is_empty()) {
                    identity = identity.with_external_id("asin", &asin, media_type);
                }
                if let Some(isbn) = md.isbn.filter(|s| !s.is_empty()) {
                    identity = identity.with_external_id("isbn", &isbn, media_type);
                }
                units.push(MediaUnit {
                    media_type,
                    identity,
                    variants: Vec::new(),
                    sources: vec![Source {
                        method: SourceKind::AppStream,
                        by: "audiobookshelf".to_string(),
                        url: Some(format!(
                            "{}/item/{}",
                            self.cfg.base_url.trim_end_matches('/'),
                            item.id
                        )),
                    }],
                });
            }
        }
        Ok(units)
    }
}

#[derive(serde::Deserialize)]
#[serde(crate = "plugin_toolkit::serde")]
struct LibrariesResp {
    #[serde(default)]
    libraries: Vec<AbsLibrary>,
}

#[derive(serde::Deserialize)]
#[serde(crate = "plugin_toolkit::serde")]
struct AbsLibrary {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "mediaType")]
    media_type: String,
}

#[derive(serde::Deserialize)]
#[serde(crate = "plugin_toolkit::serde")]
struct ItemsResp {
    #[serde(default)]
    results: Vec<AbsItem>,
}

#[derive(serde::Deserialize)]
#[serde(crate = "plugin_toolkit::serde")]
struct AbsItem {
    #[serde(default)]
    id: String,
    #[serde(default)]
    media: AbsItemMedia,
}

#[derive(serde::Deserialize, Default)]
#[serde(crate = "plugin_toolkit::serde")]
struct AbsItemMedia {
    #[serde(default)]
    metadata: AbsMetadata,
}

#[derive(serde::Deserialize, Default)]
#[serde(crate = "plugin_toolkit::serde")]
struct AbsMetadata {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    asin: Option<String>,
    #[serde(default)]
    isbn: Option<String>,
    #[serde(default, rename = "seriesName")]
    series_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Pins the ABS `/api/libraries/{id}/items` JSON shape this backend maps from —
    // a rename (`mediaType`, `seriesName`) or nesting change breaks the units view.
    #[test]
    fn abs_item_json_decodes_the_metadata_fields() {
        let raw = plugin_toolkit::serde_json::json!({
            "results": [{
                "id": "li_abc",
                "media": { "metadata": {
                    "title": "The Way of Kings",
                    "asin": "B0041JKFJW",
                    "isbn": null,
                    "seriesName": "The Stormlight Archive"
                }}
            }]
        });
        let parsed: ItemsResp = plugin_toolkit::serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.results.len(), 1);
        let md = &parsed.results[0].media.metadata;
        assert_eq!(md.title.as_deref(), Some("The Way of Kings"));
        assert_eq!(md.asin.as_deref(), Some("B0041JKFJW"));
        assert_eq!(md.isbn, None);
        assert_eq!(md.series_name.as_deref(), Some("The Stormlight Archive"));
        assert_eq!(parsed.results[0].id, "li_abc");
    }

    // ── Integration: AbsClient against a mock Audiobookshelf server ────────────
    // Exercises the real HTTP path (Bearer auth + endpoint paths) and the full
    // libraries→items→MediaUnit mapping without the endpoint db.
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn units_maps_library_items_over_http() {
        let server = MockServer::start().await;
        let json = plugin_toolkit::serde_json::json!(
        {"libraries": [
            {"id": "lib_books", "mediaType": "book"},
            {"id": "lib_pods",  "mediaType": "podcast"}
        ]});
        // /api/libraries — Bearer auth asserted.
        Mock::given(method("GET"))
            .and(path("/api/libraries"))
            .and(header("authorization", "Bearer tok123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json))
            .mount(&server)
            .await;
        // Only the book library's items should be fetched for Audiobooks.
        Mock::given(method("GET"))
            .and(path("/api/libraries/lib_books/items"))
            .and(header("authorization", "Bearer tok123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                plugin_toolkit::serde_json::json!({"results": [{
                    "id": "li_wok",
                    "media": {"metadata": {
                        "title": "The Way of Kings",
                        "asin": "B0041JKFJW",
                        "isbn": "0-7653-2635-5",
                        "seriesName": "The Stormlight Archive"
                    }}
                }]}),
            ))
            .mount(&server)
            .await;

        let cfg = AbsConfig {
            base_url: server.uri(),
            token: "tok123".into(),
        };
        let units = AbsClient::new(&cfg)
            .units(MediaType::Audiobooks)
            .await
            .expect("units ok");

        assert_eq!(units.len(), 1, "only the book library maps");
        let u = &units[0];
        assert_eq!(u.media_type, MediaType::Audiobooks);
        assert_eq!(u.identity.title, "The Way of Kings");
        assert_eq!(
            u.identity.series.as_ref().map(|s| s.name.as_str()),
            Some("The Stormlight Archive")
        );
        // ASIN normalized upper; ISBN-10 normalized to ISBN-13 by #409 helpers.
        let ids: Vec<(&str, &str)> = u
            .identity
            .external_ids
            .iter()
            .map(|e| (e.source.as_str(), e.id.as_str()))
            .collect();
        assert!(ids.contains(&("asin", "B0041JKFJW")), "ids={ids:?}");
        assert!(ids.contains(&("isbn", "9780765326355")), "ids={ids:?}");
        // AppStream source points at the item on this server.
        let src = &u.sources[0];
        assert_eq!(src.by, "audiobookshelf");
        assert_eq!(
            src.url.as_deref(),
            Some(&*format!("{}/item/li_wok", server.uri()))
        );
    }

    #[tokio::test]
    async fn units_are_empty_for_a_type_abs_does_not_serve() {
        let server = MockServer::start().await;
        // No mounts needed: movies short-circuits before any request.
        let cfg = AbsConfig {
            base_url: server.uri(),
            token: "t".into(),
        };
        let units = AbsClient::new(&cfg).units(MediaType::Movies).await.unwrap();
        assert!(units.is_empty());
    }
}
