//! The `demi web` group (`web.md`): `search` runs in the plugin, against
//! Exa; `fetch` runs `fetch.get` of the `demi.fetch` package on the Host,
//! its arguments declared from the `command-package-fetch-protocol` types.

use demi_command_declarations::NativeOperation;
use demi_command_package_fetch_protocol::{
    DEFAULT_LENGTH, DEFAULT_TIMEOUT_MS, FetchArgs, Fetched, GET, MAX_LENGTH, MAX_TIMEOUT_MS,
    PACKAGE,
};
use demi_host_interface::{GroupBuilder, LeafBuilder};
use demi_plugin_interface::{CommandPlugin, Placement, PortHandled};

use crate::search::{DEFAULT_COUNT, MAX_COUNT, SearchArgs, SearchResults};

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Searches the web and reads pages: `search` lists pages for one to four queries, with their links and the lines that matter, and `fetch` reads a URL from the Host as Markdown, a part at a time, and saves a PDF, an image or any other file for `demi file view`. Use them for documentation, release notes, news or an API's JSON. Not for a page that needs JavaScript, a sign-in or clicks, where `demi browser` works.";

/// The plugin's commands: the `demi web` group.
pub(crate) fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![web_group()])
        .expect("the web group is a valid declaration")
}

fn web_group() -> GroupBuilder {
    GroupBuilder::new("web", "Search the web and read pages from this Host.")
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc(
                "search",
                "Search the web for one to four queries, run at once, and list the pages found, merged in turn: each page's title, domain, date, URL and up to three lines that matter. Searches with the user's Exa account or API key from Settings › Web Search, or anonymously within a shared limit.",
            )
            .input::<SearchArgs>()
            .positionals(["query"])
            .describe(
                "count",
                format!("How many pages to list in all, 1 to {MAX_COUNT}; default {DEFAULT_COUNT}"),
            )
            .json_output::<SearchResults>()
            .success_output(
                "a line saying that what follows is untrusted web content, then the pages, numbered, and a line asking to cite the pages used as Markdown links; or \"No pages found. Try other words or fewer filters.\"",
            )
            .failure_output(
                "an option the search cannot honour without an Exa API key, the anonymous limit, the account's rate limit or credit, an expired sign-in, or Exa not answering within 30 s, each named on stderr with exit status 1",
            )
            .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::native(
                "fetch",
                "Fetch a URL from this Host and print it, a page as Markdown, one part at a time; save a PDF, an image or any other file and print its path. It runs no JavaScript and signs in nowhere: for a page that needs either, or clicks, use demi browser.",
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
                format!("How many characters the part holds; default {DEFAULT_LENGTH}, maximum {MAX_LENGTH}"),
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
