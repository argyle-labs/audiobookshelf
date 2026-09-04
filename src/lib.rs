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
use plugin_toolkit::media::{Capability, MediaBackend, MediaError, MediaType, MediaUrl};

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
    /// A server that hands out a reachable URL for device setup. `credentials`
    /// (per-user, orca-managed) lands with the served_by credential brokerage
    /// (orca#406); `units` with the library-view slice (orca#408 follow-up).
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::ServedBy, Capability::Url]
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
        let rows = endpoint_db::list()
            .map_err(|e| MediaError::Transport(format!("read endpoints: {e}")))?;
        let ep = rows
            .into_iter()
            .find(|r| r.enabled)
            .ok_or_else(|| MediaError::NotFound("no enabled audiobookshelf endpoint".into()))?;
        Ok(MediaUrl {
            primary: ep.base_url,
            alternates: Vec::new(),
        })
    }
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
    fn media_facet_is_a_served_by_url_backend() {
        let m = AbsMedia::served(MediaType::Audiobooks);
        assert_eq!(m.name(), "audiobookshelf");
        assert_eq!(m.media_type(), MediaType::Audiobooks);
        assert!(m.capabilities().contains(&Capability::ServedBy));
        assert!(m.capabilities().contains(&Capability::Url));
    }
}
