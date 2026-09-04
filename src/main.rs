//! Dynamic (subprocess) entrypoint for the audiobookshelf plugin.
//!
//! Audiobookshelf is a **multi-facet** plugin: a deployable `service` AND a
//! media `served_by` server for audiobooks + podcasts. The unified
//! [`Plugin`](plugin_toolkit::plugin::Plugin) builder advertises all three
//! facets from one binary (one merged `backends` array, one composed dispatch),
//! plus the `audiobookshelf.*` endpoint-registry tools. Referencing the lib's
//! types here force-links its `#[orca_tool]` inventory so it survives linking.
plugin_toolkit::instrument::bootstrap!();

fn main() -> plugin_toolkit::anyhow::Result<()> {
    use plugin_toolkit::media::MediaType;
    plugin_toolkit::plugin::Plugin::named("audiobookshelf")
        .version(env!("CARGO_PKG_VERSION"))
        .tools(["audiobookshelf."])
        .service(audiobookshelf::AudiobookshelfBackend::new("audiobookshelf"))
        .media(audiobookshelf::AbsMedia::served(MediaType::Audiobooks))
        .media(audiobookshelf::AbsMedia::served(MediaType::Podcasts))
        .serve()
}
