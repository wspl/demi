//! `xtask logos` (`crates/backend-providers/src/llm/vendors.rs`): refreshes the
//! marks of the vendors the backend offers from models.dev into the web
//! app's logos directory.
//!
//! The vendors are the ones the backend's vendor catalog offers, read from
//! models.dev's document as the backend reads it. Each vendor's mark is
//! models.dev's `logos/<vendor>.svg`, which models.dev answers with a
//! generic sparkle for a vendor it has no mark for; the sparkle is what it
//! answers for a vendor that does not exist, and a vendor whose mark is the
//! sparkle gets no file, so it shows its initial. The web app paints a mark
//! as a mask in the text's color (`VendorMark`), which keeps only its shape:
//! a mark the mask cannot draw gets no file either, and shows its initial,
//! for the reason [`Unusable`] names. The marks kept by hand
//! are listed in [`HAND_KEPT`] with their sources; the command never writes
//! or removes them, and removes every other mark it did not write.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_backend_providers::llm::vendors::VendorCatalog;
use demi_provider_common::models_dev::ModelsDevClient;
use demi_shared_types::SystemClock;
use reqwest::Url;
use tokio_util::sync::CancellationToken;
use resvg::{tiny_skia, usvg};

/// The marks' directory, from the repository's root.
const LOGOS: &str = "packages/web-ui/src/settings/logos";

/// The list of the marks kept by hand, in the marks' directory: one line
/// each, the file's name and then its source; `#` starts a comment.
const HAND_KEPT: &str = "hand-kept.txt";

/// A vendor ID models.dev does not have, whose mark is its placeholder.
const NO_SUCH_VENDOR: &str = "demi-no-such-vendor";

const MODELS_DEV: &str = "https://models.dev/";

#[derive(clap::Args)]
pub struct Options {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("models.dev: {0}")]
    ModelsDev(String),
    #[error("{url}: {message}")]
    Fetch { url: Url, message: String },
    #[error("another logger is installed, so usvg's warnings about a mark cannot be read")]
    Logger,
    #[error("{}: {source}", .path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// What a refresh changed, by vendor ID; `unmarked` are the offered
/// vendors models.dev has no mark for, and `unusable` those whose mark the
/// web app cannot paint, with the reason.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refresh {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub unmarked: Vec<String>,
    pub unusable: Vec<(String, Unusable)>,
}

/// Why a mark cannot be painted as a mask, which keeps a mark's shape and
/// drops its colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unusable {
    /// usvg cannot read it.
    Unreadable,
    /// usvg cannot read one of its colors, such as CSS's `color()`, and
    /// would judge it painted in black.
    UnreadColor,
    /// It holds a raster image, whose whole box the mask fills.
    Raster,
    /// It is drawn in more than one paint, such as a white glyph on a dark
    /// disc, which the mask merges into one shape.
    Colors,
    /// It covers nearly its whole box, which the mask paints as a solid
    /// tile: a light badge behind a dark glyph, for one, whose color usvg
    /// cannot always tell apart (it reads CSS's `color()` as black).
    Solid,
    /// It is more than twice as wide as tall or tall as wide, a wordmark
    /// that a square tile shrinks out of sight.
    Wordmark,
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreadable => "not an SVG usvg reads",
            Self::UnreadColor => "a color usvg cannot read",
            Self::Raster => "a raster image",
            Self::Colors => "several colors",
            Self::Solid => "a solid box",
            Self::Wordmark => "a wordmark",
        })
    }
}

pub fn run(_options: Options) -> Result<(), Error> {
    let client = demi_shared_artifacts::download_client().map_err(|error| Error::ModelsDev(error.to_string()))?;
    let base = Url::parse(MODELS_DEV).expect("models.dev's address is a URL");
    let directory = crate::repository().join(LOGOS);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| Error::Io {
            path: directory.clone(),
            source,
        })?;
    let refresh = runtime.block_on(refresh(&client, &base, &directory))?;
    for (label, ids) in [
        ("Added", &refresh.added),
        ("Updated", &refresh.updated),
        ("Removed", &refresh.removed),
        ("No mark", &refresh.unmarked),
    ] {
        if !ids.is_empty() {
            println!("{label}: {}", ids.join(", "));
        }
    }
    if !refresh.unusable.is_empty() {
        let marks: Vec<String> = refresh
            .unusable
            .iter()
            .map(|(id, why)| format!("{id} ({why})"))
            .collect();
        println!("Mark left out: {}", marks.join(", "));
    }
    if refresh.added.is_empty() && refresh.updated.is_empty() && refresh.removed.is_empty() {
        println!("The marks are up to date.");
    }
    Ok(())
}

/// Writes into `directory` the mark of each vendor the backend offers from
/// the models.dev at `base`, and removes the marks that are neither kept by
/// hand nor written now.
pub async fn refresh(client: &reqwest::Client, base: &Url, directory: &Path) -> Result<Refresh, Error> {
    let document = base.join("api.json").expect("a relative path joins");
    let catalog = VendorCatalog::new(ModelsDevClient::new(client.clone(), document, Arc::new(SystemClock)));
    let vendors = catalog
        .vendors()
        .await
        .map_err(|error| Error::ModelsDev(error.to_string()))?;
    PaintWarnings::install()?;
    let placeholder = fetch(client, &mark_url(base, NO_SUCH_VENDOR)).await?;
    let hand_kept = hand_kept(directory)?;
    let mut refresh = Refresh::default();
    let mut written = BTreeSet::new();
    for vendor in vendors {
        let file = format!("{}.svg", vendor.id);
        if hand_kept.contains(&file) {
            continue;
        }
        let mark = fetch(client, &mark_url(base, &vendor.id)).await?;
        if mark == placeholder {
            refresh.unmarked.push(vendor.id);
            continue;
        }
        if let Some(why) = unusable(&mark) {
            refresh.unusable.push((vendor.id, why));
            continue;
        }
        let path = directory.join(&file);
        let kept = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(Error::Io { path, source }),
        };
        if kept.as_deref() != Some(&mark[..]) {
            std::fs::write(&path, &mark).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            match kept {
                Some(_) => refresh.updated.push(vendor.id),
                None => refresh.added.push(vendor.id),
            }
        }
        written.insert(file);
    }
    let entries = std::fs::read_dir(directory).map_err(|source| Error::Io {
        path: directory.to_owned(),
        source,
    })?;
    let mut stale = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: directory.to_owned(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".svg")
            && !hand_kept.contains(&name)
            && !written.contains(&name)
        {
            stale.push((id.to_owned(), entry.path()));
        }
    }
    stale.sort();
    for (id, path) in stale {
        std::fs::remove_file(&path).map_err(|source| Error::Io { path, source })?;
        refresh.removed.push(id);
    }
    refresh.added.sort();
    refresh.updated.sort();
    refresh.unmarked.sort();
    refresh.unusable.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(refresh)
}

/// The side a mark is drawn at to measure how much of its box it covers.
const PROBE_SIDE: f32 = 64.0;

/// The share of its box past which a mark paints as a solid tile: nine
/// tenths, where a badge with a small glyph cut out of it reads as a blob;
/// a glyph cut from a disc or a square, as nearai's, covers less.
const SOLID: f32 = 0.9;

/// Why the web app cannot paint `mark` as a mask, if it cannot.
fn unusable(mark: &[u8]) -> Option<Unusable> {
    let (tree, unread) = PaintWarnings::during(|| usvg::Tree::from_data(mark, &usvg::Options::default()));
    let Ok(tree) = tree else {
        return Some(Unusable::Unreadable);
    };
    if unread {
        return Some(Unusable::UnreadColor);
    }
    let size = tree.size();
    let ratio = size.width() / size.height();
    if !(0.5..=2.0).contains(&ratio) {
        return Some(Unusable::Wordmark);
    }
    let mut colors = BTreeSet::new();
    if let Err(why) = colors_of(tree.root(), &mut colors) {
        return Some(why);
    }
    if colors.len() > 1 {
        return Some(Unusable::Colors);
    }
    (coverage(&tree) > SOLID).then_some(Unusable::Solid)
}

/// usvg's warnings that it could not read a paint, the one way it tells: it
/// logs the warning and paints the shape black or not at all.
struct PaintWarnings;

static PAINT_WARNINGS: PaintWarnings = PaintWarnings;

thread_local! {
    /// Whether this thread reads a mark, and whether a paint failed.
    static UNREAD_PAINT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

impl PaintWarnings {
    /// Makes this the process's logger, once; another logger would take
    /// the warnings.
    fn install() -> Result<(), Error> {
        static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let installed = *INSTALLED.get_or_init(|| {
            let set = log::set_logger(&PAINT_WARNINGS).is_ok();
            if set {
                log::set_max_level(log::LevelFilter::Warn);
            }
            set
        });
        if installed { Ok(()) } else { Err(Error::Logger) }
    }

    /// Runs `read` and answers what it returned and whether usvg could
    /// not read a paint meanwhile.
    fn during<T>(read: impl FnOnce() -> T) -> (T, bool) {
        UNREAD_PAINT.set(Some(false));
        let value = read();
        (value, UNREAD_PAINT.take() == Some(true))
    }
}

impl log::Log for PaintWarnings {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn && metadata.target().starts_with("usvg")
    }

    fn log(&self, record: &log::Record<'_>) {
        let paint = record.args().to_string().starts_with("Failed to parse fill value")
            || record.args().to_string().starts_with("Failed to parse stroke value");
        if self.enabled(record.metadata()) && paint {
            UNREAD_PAINT.with(|unread| {
                if unread.get().is_some() {
                    unread.set(Some(true));
                }
            });
        }
    }

    fn flush(&self) {}
}

/// Gathers the colors `group` is painted in; a raster image, or a gradient
/// or pattern, which the mask cannot keep either, ends the walk.
fn colors_of(group: &usvg::Group, colors: &mut BTreeSet<(u8, u8, u8)>) -> Result<(), Unusable> {
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => colors_of(group, colors)?,
            usvg::Node::Image(_) => return Err(Unusable::Raster),
            usvg::Node::Path(path) => {
                let fill = path.fill().map(usvg::Fill::paint);
                let stroke = path.stroke().map(usvg::Stroke::paint);
                for paint in fill.into_iter().chain(stroke) {
                    let usvg::Paint::Color(color) = paint else {
                        return Err(Unusable::Colors);
                    };
                    colors.insert((color.red, color.green, color.blue));
                }
            }
            // Built without text, usvg reads no text into the tree: a
            // mark's text the browser draws itself, in its one paint.
            usvg::Node::Text(_) => {}
        }
    }
    Ok(())
}

/// The share of its box `tree` paints, drawn as the mask draws it: every
/// pixel at least half opaque counts.
fn coverage(tree: &usvg::Tree) -> f32 {
    let size = tree.size();
    let scale = PROBE_SIDE / size.width().max(size.height());
    let (width, height) = ((size.width() * scale).round(), (size.height() * scale).round());
    // Both sides are between 32 and 64 pixels, as the ratio is checked.
    let Some(mut pixmap) = tiny_skia::Pixmap::new(width as u32, height as u32) else {
        return 0.0;
    };
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let painted = pixmap.data().chunks_exact(4).filter(|pixel| pixel[3] >= 128).count();
    painted as f32 / (width * height)
}

fn mark_url(base: &Url, vendor_id: &str) -> Url {
    base.join(&format!("logos/{vendor_id}.svg"))
        .expect("a vendor ID joins as a path")
}

/// The file models.dev answers at `url`; any status but success stops the
/// refresh, so a failed request never removes a mark.
async fn fetch(client: &reqwest::Client, url: &Url) -> Result<Vec<u8>, Error> {
    let failed = |error: demi_shared_artifacts::Error| Error::Fetch {
        url: url.clone(),
        message: error.to_string(),
    };
    let request = demi_shared_artifacts::Download::new(url.as_str());
    let (received, bytes) =
        demi_shared_artifacts::download_bytes(client, request, &CancellationToken::new())
            .await
            .map_err(failed)?;
    received.plain().map_err(failed)?;
    Ok(bytes)
}

/// The file names [`HAND_KEPT`] lists in `directory`.
fn hand_kept(directory: &Path) -> Result<BTreeSet<String>, Error> {
    let path = directory.join(HAND_KEPT);
    let list = std::fs::read_to_string(&path).map_err(|source| Error::Io { path, source })?;
    Ok(list
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use demi_shared_artifacts::testing::{Answer, Server};
    use serde_json::json;

    use super::*;

    /// A one-paint mark drawing `shape`, as most of models.dev's are.
    fn mark(shape: &str) -> String {
        format!(r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="currentColor" d="{shape}"/></svg>"#)
    }

    const FRAMED: &str = r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><rect x="1" y="1" width="22" height="22" fill="none" stroke="black" stroke-width="2"/><path d="M8 8h8v8H8z"/></svg>"#;

    fn vendor(id: &str, npm: &str) -> serde_json::Value {
        json!({ "id": id, "name": id, "npm": npm, "api": "https://example.com/v1", "models": {} })
    }

    /// About 0.1 s: one fixture server on this machine.
    #[tokio::test]
    async fn a_refresh_writes_the_offered_marks_a_mask_can_paint_and_removes_the_rest() {
        let sparkle = mark("M12 2l2 8 8 2-8 2-2 8-2-8-8-2 8-2z");
        let compatible = "@ai-sdk/openai-compatible";
        let document = json!({
            "fresh": vendor("fresh", compatible),
            "changed": vendor("changed", "@ai-sdk/anthropic"),
            "same": vendor("same", "@ai-sdk/google"),
            "sparkled": vendor("sparkled", compatible),
            "duotone": vendor("duotone", compatible),
            "banner": vendor("banner", compatible),
            "photo": vendor("photo", compatible),
            "badge": vendor("badge", compatible),
            "framed": vendor("framed", compatible),
            "tile": vendor("tile", compatible),
            // A vendor the backend does not offer: its mark is not fetched.
            "unoffered": vendor("unoffered", "@ai-sdk/amazon-bedrock"),
            // A vendor whose mark is kept by hand.
            "claude": vendor("claude", "@ai-sdk/anthropic"),
        });
        let server = Server::start([
            ("/api.json".to_owned(), Answer::ok(document.to_string())),
            (format!("/logos/{NO_SUCH_VENDOR}.svg"), Answer::ok(sparkle.clone())),
            ("/logos/fresh.svg".to_owned(), Answer::ok(mark("M2 2h20v20H2z"))),
            ("/logos/changed.svg".to_owned(), Answer::ok(mark("M4 4h16v16H4z"))),
            ("/logos/same.svg".to_owned(), Answer::ok(mark("M6 6h12v12H6z"))),
            ("/logos/sparkled.svg".to_owned(), Answer::ok(sparkle)),
            // A white glyph on a black disc, which a mask paints as a disc.
            (
                "/logos/duotone.svg".to_owned(),
                Answer::ok(r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="12" fill="black"/><path fill="white" d="M8 8h8v8H8z"/></svg>"#),
            ),
            // A wordmark five times as wide as tall.
            (
                "/logos/banner.svg".to_owned(),
                Answer::ok(r#"<svg viewBox="0 0 120 24" xmlns="http://www.w3.org/2000/svg"><path d="M0 0h120v24H0z"/></svg>"#),
            ),
            // A PNG in an SVG, whose whole box a mask fills.
            (
                "/logos/photo.svg".to_owned(),
                Answer::ok(r#"<svg viewBox="0 0 1 1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image width="1" height="1" xlink:href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="/></svg>"#),
            ),
            // A plain square, which a mask paints as a solid tile.
            ("/logos/tile.svg".to_owned(), Answer::ok(mark("M0 0h24v24H0z"))),
            // A dark glyph on a light badge whose color usvg reads as black.
            (
                "/logos/badge.svg".to_owned(),
                Answer::ok(r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><rect width="24" height="24" style="fill:color(display-p3 0.96 0.96 0.96)"/><path fill="black" d="M8 8h8v8H8z"/></svg>"#),
            ),
            // A frame around a glyph in one paint, which a mask draws as it is.
            (
                "/logos/framed.svg".to_owned(),
                Answer::ok(FRAMED),
            ),
            ("/logos/unoffered.svg".to_owned(), Answer::ok(mark("M1 1h2v2H1z"))),
            ("/logos/claude.svg".to_owned(), Answer::ok(mark("M3 3h2v2H3z"))),
        ])
        .await;
        let directory = tempfile::tempdir().unwrap();
        let file = |name: &str| directory.path().join(name);
        std::fs::write(file(HAND_KEPT), "# By hand\nclaude.svg  Simple Icons (CC0)\nelevenlabs.svg  Simple Icons\n").unwrap();
        for (name, body) in [
            ("claude.svg", "hand claude".to_owned()),
            ("elevenlabs.svg", "hand elevenlabs".to_owned()),
            ("changed.svg", mark("M0 0h1v1H0z")),
            ("same.svg", mark("M6 6h12v12H6z")),
            // Once a mark, now the placeholder.
            ("sparkled.svg", mark("M0 0h2v2H0z")),
            // Once a mark a mask could paint.
            ("duotone.svg", mark("M0 0h3v3H0z")),
            // A vendor no longer offered.
            ("gone.svg", mark("M0 0h4v4H0z")),
        ] {
            std::fs::write(file(name), body).unwrap();
        }

        let client = demi_shared_artifacts::download_client_allowing_http().unwrap();
        let base = Url::parse(&server.url("/")).unwrap();
        let refresh = refresh(&client, &base, directory.path()).await.unwrap();

        let ids = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            refresh,
            Refresh {
                added: ids(&["framed", "fresh"]),
                updated: ids(&["changed"]),
                removed: ids(&["duotone", "gone", "sparkled"]),
                unmarked: ids(&["sparkled"]),
                unusable: vec![
                    ("badge".to_owned(), Unusable::UnreadColor),
                    ("banner".to_owned(), Unusable::Wordmark),
                    ("duotone".to_owned(), Unusable::Colors),
                    ("photo".to_owned(), Unusable::Raster),
                    ("tile".to_owned(), Unusable::Solid),
                ],
            }
        );
        let mut files: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        files.sort();
        assert_eq!(
            files,
            ["changed.svg", "claude.svg", "elevenlabs.svg", "framed.svg", "fresh.svg", HAND_KEPT, "same.svg"]
        );
        let read = |name: &str| std::fs::read_to_string(file(name)).unwrap();
        assert_eq!(read("fresh.svg"), mark("M2 2h20v20H2z"));
        assert_eq!(read("changed.svg"), mark("M4 4h16v16H4z"));
        assert_eq!(read("claude.svg"), "hand claude");
        assert_eq!(read("elevenlabs.svg"), "hand elevenlabs");
    }
}
