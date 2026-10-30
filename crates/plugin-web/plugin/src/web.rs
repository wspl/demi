//! The `demi web` group.
//!
//! - `search` runs in the plugin, against Exa.
//! - `fetch` runs `fetch.get` of the `demi.fetch` package on the Host, its
//!   arguments declared from the `command-package-fetch-protocol` types.
//!
//! # Why they run in different places
//!
//! - `search` calls a search vendor with the user's account, whose token never
//!   leaves the backend.
//! - `fetch` reads a page as `curl` on the Host would, through the Host's
//!   network, proxy and VPN, reaching what the Host reaches, `localhost` and
//!   the intranet included. It never goes through the backend, which would
//!   otherwise fetch addresses for anyone who asks.
//!
//! One group holds both, bound leaf by leaf, so the model finds them as one
//! capability. A deployment without the `demi.fetch` package leaves the whole
//! group out.
//!
//! Each is a feature the user turns on and off apart: search off until the
//! user turns it on, since a subscription's model searches with its vendor's
//! own search, and fetch on, since every model reads pages with it.
//!
//! # Permissions and reading
//!
//! Neither asks the user for permission: a search spends the user's own search
//! account or the anonymous allowance, and a fetch does what the agent's `curl`
//! already may.
//!
//! No small model reads a page first: the agent reads what it fetched and
//! decides what matters, so nothing is lost on the way.

use demi_command_declarations::NativeOperation;
use demi_command_package_fetch_protocol::{
    DEFAULT_LENGTH, DEFAULT_TIMEOUT_MS, FetchArgs, Fetched, GET, MAX_TIMEOUT_MS,
    PACKAGE,
};
use demi_host_interface::{GroupBuilder, LeafBuilder};
use demi_plugin_interface::{CommandPlugin, Feature, Placement, PortHandled};

use crate::search::{DEFAULT_COUNT, SearchArgs, SearchedPage};

/// The group's entry in the model's capability index.
const ENTRY: &str = "Searches the web; fetches a page as Markdown, or a file.";

/// The plugin's commands: the `demi web` group.
pub(crate) fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![web_group()])
        .expect("the web group is a valid declaration")
}

/// The plugin's features: `search` and `fetch`, each saying its part of the
/// group's summary and index entry.
pub(crate) fn features() -> Vec<Feature> {
    let feature = |id: &str, on_by_default, summary: &str, index_entry: &str| Feature {
        id: id.into(),
        on_by_default,
        group: vec!["web".into()],
        leaves: vec![id.into()],
        summary: summary.into(),
        index_entry: index_entry.into(),
    };
    vec![
        feature("search", false, "Search the web.", "Searches the web."),
        feature("fetch", true, "Read pages from this Host.", "Fetches a page as Markdown, or a file."),
    ]
}

fn web_group() -> GroupBuilder {
    GroupBuilder::new("web", "Search the web and read pages from this Host.")
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc(
                "search",
                "Search the web for one to four queries, run at once, and list the pages found, merged in turn: each page's title, domain, date, URL and up to three lines that matter. Cite the pages you use as Markdown links. Searches with the user's Exa account or API key from Settings › Search, or anonymously within a shared limit.",
            )
            .input::<SearchArgs>()
            .positionals(["query"])
            .describe(
                "count",
                format!("How many pages to list in all, at least 1; default {DEFAULT_COUNT}"),
            )
            .json_output::<Vec<SearchedPage>>()
            .success_output(
                "a line saying that what follows is untrusted web content, then the pages, numbered; or \"No pages found. Try other words or fewer filters.\"",
            )
            .failure_output(
                "an option the search cannot honour without an Exa API key, the anonymous limit, the account's rate limit or credit, an expired sign-in, or Exa not answering within 30 s, each named on stderr with exit status 1",
            )
            .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::native(
                "fetch",
                "Fetch a URL from this Host and print it, a page as Markdown, one part at a time; save a PDF, an image or any other file and print its path. It runs no JavaScript and signs in nowhere.",
                NativeOperation {
                    package: PACKAGE.into(),
                    operation: GET.into(),
                },
            )
            .input::<FetchArgs>()
            .positionals(["url"])
            .describe(
                "offset",
                "The character the part starts at; the line after a part names the next call",
            )
            .describe(
                "length",
                format!("How many characters the part holds; default {DEFAULT_LENGTH}"),
            )
            .describe(
                "timeout",
                format!("The whole fetch's deadline in milliseconds; default {DEFAULT_TIMEOUT_MS}, maximum {MAX_TIMEOUT_MS}"),
            )
            .json_output::<Fetched>()
            .success_output(
                "a line naming the URL, where its redirects led, its HTTP status, type and length; a line saying that what follows is untrusted web content; the part; and, when more remains, a line naming how many characters and the call that reads on. A page read is kept on this Host for 15 minutes, so reading on fetches it only once. A file that is no text is saved under the temporary directory's demi-fetch/, and its path, type and size are printed with the call that shows it",
            )
            .failure_output(
                "an HTTP status of 400 or more prints the page and exits 1; a Cloudflare challenge, no answer within the deadline, more than 10 redirects, more than 10 MiB of text or 64 MiB of a file write the reason to stderr and exit 1",
            ),
        )
}
