//! The `demi web` group (`web.md`): `fetch` runs `fetch.get` of the
//! `demi.fetch` package on the Host, its arguments declared from the
//! `command-package-fetch-protocol` types.

use demi_command_declarations::NativeOperation;
use demi_command_package_fetch_protocol::{
    DEFAULT_LENGTH, DEFAULT_TIMEOUT_MS, FetchArgs, Fetched, GET, MAX_LENGTH, MAX_TIMEOUT_MS,
    PACKAGE,
};
use demi_host_interface::{GroupBuilder, LeafBuilder};

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Reads a web page from the Host: `fetch` prints it as Markdown, a part at a time, and saves a PDF, an image or any other file for `demi file view`. Use it for documentation, articles, release notes or an API's JSON at a URL you have. Not for a page that needs JavaScript, a sign-in or clicks, where `demi browser` works.";

pub(crate) fn web_group() -> GroupBuilder {
    GroupBuilder::new("web", "Read web pages from this Host.")
        .index_entry(ENTRY)
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
