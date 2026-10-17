plugin_toolkit::instrument::bootstrap!();

#[allow(unused_imports)]
use audiobookshelf::tools as _;

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
