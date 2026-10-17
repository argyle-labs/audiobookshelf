use plugin_toolkit::prelude::*;

use crate::api::{Abs, Item, Library, MetadataPatch, NameOnly, Series, SeriesEntry};
use crate::audit::{self, SeriesFinding};
use crate::endpoint_db;

pub async fn connect(name: &str) -> Result<Abs> {
    let token = plugin_toolkit::secrets::resolve_scoped("audiobookshelf", name, "token")?;
    let row = endpoint_db::require(name)?;
    let base_url = route::resolve_reachable(name, &row.routes, false).await?;
    Abs::new(&base_url, token.expose_secret())
}

async fn book_libraries(abs: &Abs, library_id: Option<&str>) -> Result<Vec<Library>> {
    if let Some(id) = library_id {
        return Ok(vec![abs.library(id).await?]);
    }
    Ok(abs
        .libraries()
        .await?
        .into_iter()
        .filter(|l| l.media_type == "book")
        .collect())
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub id: String,
    pub error: String,
}

#[orca_struct(args)]
pub struct EndpointArgs {
    /// Registered audiobookshelf endpoint name.
    #[arg(long)]
    pub name: String,
}

/// List libraries with media type and metadata provider.
#[orca_tool(domain = "audiobookshelf", verb = "libraries.list", role = "read")]
async fn audiobookshelf_libraries_list(args: EndpointArgs, _ctx: &ToolCtx) -> Result<Vec<Library>> {
    connect(&args.name).await?.libraries().await
}

#[orca_struct(args)]
pub struct ScanArgs {
    /// Registered audiobookshelf endpoint name.
    #[arg(long)]
    pub name: String,
    /// Library id.
    #[arg(long)]
    pub library_id: String,
    /// Re-scan every file, not only changed ones.
    #[arg(long)]
    #[serde(default)]
    pub force: bool,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    pub library_id: String,
    pub force: bool,
    pub started: bool,
}

/// Start a library scan (`POST /api/libraries/{id}/scan`). The scan runs in
/// the background on the server.
#[orca_tool(
    domain = "audiobookshelf",
    verb = "library.scan",
    role = "admin",
    execute_gated = true
)]
async fn audiobookshelf_library_scan(args: ScanArgs, _ctx: &ToolCtx) -> Result<ScanReport> {
    connect(&args.name)
        .await?
        .scan(&args.library_id, args.force)
        .await?;
    Ok(ScanReport {
        library_id: args.library_id,
        force: args.force,
        started: true,
    })
}

#[orca_struct(args)]
pub struct ItemsArgs {
    /// Registered audiobookshelf endpoint name.
    #[arg(long)]
    pub name: String,
    /// Library id. Omitted, every book library.
    #[arg(long)]
    pub library_id: Option<String>,
}

/// List library items with title, authors, narrators, series + sequence, ASIN
/// and ISBN. Omitting `library_id` covers every book library, which makes this
/// the inventory read for ebook and audiobook parity.
#[orca_tool(domain = "audiobookshelf", verb = "items.list", role = "read")]
async fn audiobookshelf_items_list(args: ItemsArgs, _ctx: &ToolCtx) -> Result<Vec<Item>> {
    let abs = connect(&args.name).await?;
    let mut out = Vec::new();
    for lib in book_libraries(&abs, args.library_id.as_deref()).await? {
        out.extend(abs.items(&lib.id).await?);
    }
    Ok(out)
}

/// List series per library with their book counts. Omitting `library_id`
/// covers every book library.
#[orca_tool(domain = "audiobookshelf", verb = "series.list", role = "read")]
async fn audiobookshelf_series_list(args: ItemsArgs, _ctx: &ToolCtx) -> Result<Vec<Series>> {
    let abs = connect(&args.name).await?;
    let mut out = Vec::new();
    for lib in book_libraries(&abs, args.library_id.as_deref()).await? {
        out.extend(abs.series(&lib.id).await?);
    }
    Ok(out)
}

#[orca_struct(args)]
pub struct UpdateArgs {
    /// Registered audiobookshelf endpoint name.
    #[arg(long)]
    pub name: String,
    /// Library item id.
    #[arg(long)]
    pub item_id: String,
    #[arg(long)]
    pub title: Option<String>,
    /// Replaces the author list. Comma-separated on the CLI.
    #[arg(long, value_delimiter = ',')]
    pub authors: Option<Vec<String>>,
    /// Series name. Replaces the item's series list with this one entry.
    #[arg(long)]
    pub series: Option<String>,
    /// Sequence within `series`.
    #[arg(long, requires = "series")]
    pub sequence: Option<String>,
    /// Remove every series from the item.
    #[arg(long, conflicts_with = "series")]
    #[serde(default)]
    pub clear_series: bool,
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateReport {
    pub item_id: String,
    pub dry_run: bool,
    pub before: Item,
    pub title: Option<String>,
    pub authors: Option<Vec<String>>,
    pub series: Option<Vec<SeriesEntry>>,
    pub updated: bool,
}

pub fn build_patch(args: &UpdateArgs, before: &Item) -> Result<MetadataPatch> {
    if args
        .authors
        .as_ref()
        .is_some_and(|a| a.iter().any(|n| n.trim().is_empty()))
    {
        bail!("author names must not be empty");
    }
    let series = if args.clear_series {
        Some(Vec::new())
    } else {
        args.series.as_ref().map(|name| {
            vec![SeriesEntry {
                name: name.trim().to_string(),
                sequence: args.sequence.as_ref().map(|s| s.trim().to_string()),
            }]
        })
    };
    Ok(MetadataPatch {
        title: args.title.clone().filter(|t| *t != before.title),
        authors: args
            .authors
            .as_ref()
            .map(|a| a.iter().map(|n| n.trim().to_string()).collect::<Vec<_>>())
            .filter(|a| *a != before.authors)
            .map(|a| a.into_iter().map(|name| NameOnly { name }).collect()),
        series: series.filter(|s| *s != before.series),
    })
}

/// Update an item's title, authors, and series + sequence
/// (`PATCH /api/items/{id}/media`). Fields left out are untouched; fields
/// already matching are not sent. `dryRun` returns the change without writing.
#[orca_tool(
    domain = "audiobookshelf",
    verb = "item.update",
    role = "admin",
    execute_gated = false,
    dry_run_aware = true
)]
async fn audiobookshelf_item_update(args: UpdateArgs, ctx: &ToolCtx) -> Result<UpdateReport> {
    let abs = connect(&args.name).await?;
    update_item(&abs, args, ctx.dry_run()).await
}

pub async fn update_item(abs: &Abs, args: UpdateArgs, dry_run: bool) -> Result<UpdateReport> {
    let before = abs.item(&args.item_id).await?;
    let patch = build_patch(&args, &before)?;
    let empty = patch == MetadataPatch::default();
    let updated = if dry_run || empty {
        false
    } else {
        abs.update_metadata(&args.item_id, &patch).await?
    };
    Ok(UpdateReport {
        item_id: args.item_id,
        dry_run,
        before,
        title: patch.title,
        authors: patch
            .authors
            .map(|a| a.into_iter().map(|n| n.name).collect()),
        series: patch.series,
        updated,
    })
}

#[orca_struct(args)]
pub struct MatchArgs {
    /// Registered audiobookshelf endpoint name.
    #[arg(long)]
    pub name: String,
    /// Library item id, to match one item.
    #[arg(long, conflicts_with = "library_id")]
    pub item_id: Option<String>,
    /// Library id, to match every item in it.
    #[arg(long)]
    pub library_id: Option<String>,
    /// Metadata provider. Omitted, the library's configured provider.
    #[arg(long)]
    pub provider: Option<String>,
    /// Overwrite existing details with the provider's (single item only).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    #[serde(default = "yes")]
    pub override_details: bool,
}

fn yes() -> bool {
    true
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct MatchReport {
    pub item_id: Option<String>,
    pub library_id: String,
    pub provider: String,
    pub updated: bool,
    pub started: bool,
}

/// Match metadata against the provider (Audible by default) and apply it:
/// one item (`POST /api/items/{id}/match`) or a whole library
/// (`GET /api/libraries/{id}/matchall`, which runs in the background).
/// Run `series.audit` first to see what would change.
#[orca_tool(
    domain = "audiobookshelf",
    verb = "item.match",
    role = "admin",
    execute_gated = true
)]
async fn audiobookshelf_item_match(args: MatchArgs, _ctx: &ToolCtx) -> Result<MatchReport> {
    let abs = connect(&args.name).await?;
    match (args.item_id, args.library_id) {
        (Some(item_id), None) => {
            let item = abs.item(&item_id).await?;
            let provider = match args.provider {
                Some(p) => p,
                None => abs.library(&item.library_id).await?.provider,
            };
            let updated = abs
                .match_item(&item, &provider, args.override_details)
                .await?;
            Ok(MatchReport {
                item_id: Some(item_id),
                library_id: item.library_id,
                provider,
                updated,
                started: false,
            })
        }
        (None, Some(library_id)) => {
            if args.provider.is_some() {
                bail!("a library match uses the library's provider; drop provider");
            }
            let lib = abs.library(&library_id).await?;
            abs.match_library(&library_id).await?;
            Ok(MatchReport {
                item_id: None,
                library_id,
                provider: lib.provider,
                updated: false,
                started: true,
            })
        }
        _ => bail!("pass exactly one of item_id or library_id"),
    }
}

#[orca_struct(args)]
pub struct AuditArgs {
    /// Registered audiobookshelf endpoint name.
    #[arg(long)]
    pub name: String,
    /// Library id. Omitted, every book library.
    #[arg(long)]
    pub library_id: Option<String>,
    /// Audit only these item ids. Comma-separated on the CLI.
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub item_ids: Vec<String>,
    /// Stop after this many items (one provider lookup each). Default 50, max 500.
    #[arg(long)]
    pub limit: Option<usize>,
    /// Include items the provider returned nothing for.
    #[arg(long)]
    #[serde(default)]
    pub include_unmatched: bool,
}

const AUDIT_DEFAULT: usize = 50;
const AUDIT_MAX: usize = 500;

pub fn audit_limit(limit: Option<usize>) -> Result<usize> {
    match limit.unwrap_or(AUDIT_DEFAULT) {
        n if n > AUDIT_MAX => bail!("limit {n} exceeds the maximum of {AUDIT_MAX}"),
        n => Ok(n),
    }
}

#[orca_struct]
#[derive(Debug, Clone, PartialEq)]
pub struct AuditReport {
    pub checked: usize,
    pub findings: Vec<SeriesFinding>,
    pub failed: Vec<Failure>,
}

/// Read-only series audit: look each item up with its library's metadata
/// provider (`GET /api/search/books`, ASIN first) and report items whose
/// series or sequence disagrees with the provider's. Writes nothing.
#[orca_tool(domain = "audiobookshelf", verb = "series.audit", role = "read")]
async fn audiobookshelf_series_audit(args: AuditArgs, _ctx: &ToolCtx) -> Result<AuditReport> {
    let abs = connect(&args.name).await?;
    let mut report = AuditReport {
        checked: 0,
        findings: Vec::new(),
        failed: Vec::new(),
    };
    let limit = audit_limit(args.limit)?;
    'libs: for lib in book_libraries(&abs, args.library_id.as_deref()).await? {
        for item in abs.items(&lib.id).await? {
            if report.checked >= limit {
                break 'libs;
            }
            if !args.item_ids.is_empty() && !args.item_ids.contains(&item.id) {
                continue;
            }
            report.checked += 1;
            match abs.search_books(&item, &lib.provider).await {
                Ok(results) => {
                    if let Some(f) = audit::diff(&item, audit::best_match(&item, &results)) {
                        if args.include_unmatched || f.drift != audit::SeriesDrift::NoProviderMatch
                        {
                            report.findings.push(f);
                        }
                    }
                }
                Err(e) => report.failed.push(Failure {
                    id: item.id.clone(),
                    error: format!("{e:#}"),
                }),
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> Item {
        Item {
            id: "li_1".into(),
            library_id: "lib".into(),
            media_type: "book".into(),
            title: "Dune".into(),
            subtitle: None,
            authors: vec!["Frank Herbert".into()],
            narrators: Vec::new(),
            series: vec![SeriesEntry {
                name: "Dune".into(),
                sequence: Some("1".into()),
            }],
            asin: None,
            isbn: None,
            published_year: None,
        }
    }

    fn args() -> UpdateArgs {
        UpdateArgs {
            name: "abs".into(),
            item_id: "li_1".into(),
            title: None,
            authors: None,
            series: None,
            sequence: None,
            clear_series: false,
        }
    }

    #[test]
    fn patch_sends_only_changed_fields() {
        let mut a = args();
        a.title = Some("Dune".into());
        a.authors = Some(vec!["Frank Herbert".into()]);
        a.series = Some("Dune".into());
        a.sequence = Some("1".into());
        assert_eq!(build_patch(&a, &item()).unwrap(), MetadataPatch::default());
        a.sequence = Some("2".into());
        let p = build_patch(&a, &item()).unwrap();
        assert_eq!(p.series.unwrap()[0].sequence.as_deref(), Some("2"));
        assert_eq!(p.title, None);
    }

    #[test]
    fn rejects_empty_author_names() {
        let mut a = args();
        a.authors = Some(vec!["A".into(), " ".into()]);
        assert!(build_patch(&a, &item()).is_err());
    }

    #[test]
    fn audit_limit_defaults_and_caps() {
        assert_eq!(audit_limit(None).unwrap(), 50);
        assert_eq!(audit_limit(Some(500)).unwrap(), 500);
        assert!(audit_limit(Some(501)).is_err());
    }

    #[test]
    fn clear_series_sends_an_empty_list() {
        let mut a = args();
        a.clear_series = true;
        assert_eq!(build_patch(&a, &item()).unwrap().series, Some(Vec::new()));
    }

    #[test]
    fn patch_serializes_to_the_abs_media_shape() {
        let mut a = args();
        a.authors = Some(vec!["A".into(), "B".into()]);
        a.series = Some("S".into());
        let v = plugin_toolkit::serde_json::to_value(build_patch(&a, &item()).unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"authors": [{"name": "A"}, {"name": "B"}], "series": [{"name": "S", "sequence": null}]})
        );
    }

    #[test]
    fn tools_declare_roles_and_gating() {
        let role = |n| plugin_toolkit::dispatch::required_role(n);
        for n in [
            "audiobookshelf.libraries.list",
            "audiobookshelf.items.list",
            "audiobookshelf.series.list",
            "audiobookshelf.series.audit",
        ] {
            assert_eq!(role(n), Some("read"), "{n}");
        }
        for n in [
            "audiobookshelf.library.scan",
            "audiobookshelf.item.update",
            "audiobookshelf.item.match",
        ] {
            assert_eq!(role(n), Some("admin"), "{n}");
        }
        let gated = plugin_toolkit::dispatch::execute_gated_names();
        assert!(gated.contains(&"audiobookshelf.library.scan"));
        assert!(gated.contains(&"audiobookshelf.item.match"));
        assert!(!gated.contains(&"audiobookshelf.item.update"));
    }
}
