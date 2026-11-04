---
status: proposed
---

# A plugin serves the libraries its pages load

Supersedes: 0153 (A reply shows a widget), its loading of Tailwind's browser
build from jsDelivr; 0261 (A widget is shared, or copied as an image), its
loading of html-to-image from jsDelivr.

## Problem

Every widget's document loads Tailwind from jsDelivr
(`@tailwindcss/browser@4`) before the agent's page. The theme's names, such
as `text-fg-muted`, are Tailwind's, so a widget draws in Demi's look only
once that script arrives. Three things follow:

- A user whose network reaches the server but not jsDelivr, as on a company
  network or a server in a region that blocks it, sees every widget
  unstyled.
- When jsDelivr is slow, each widget waits on it. A copy as an image that
  loaded html-to-image the same way took 8.5 seconds the first time.
- `@4` follows whatever 4.x jsDelivr serves that day, so a Tailwind release
  can change how every past widget draws, with no Demi release.

## Decision

A library Demi's own code runs is Demi's: it ships with the release and the
backend serves it. A library only the agent's page chooses, such as Chart.js
or an icon set, stays the page's, loaded from wherever the page says, and the
guide still says so.

For the widget plugin, Demi's libraries are two:

| Library               | Version | Size     | Loaded                  |
| --------------------- | ------- | -------- | ----------------------- |
| `@tailwindcss/browser` | 4.3.4   | 287 KB   | before the page's head  |
| `html-to-image`        | 1.11.13 | 20 KB    | at the first copy       |

### The plugin's files

A plugin factory names its files and gives their bytes, which it holds in its
binary:

```rust
pub trait PluginFactory: Send + Sync {
    fn manifest(&self) -> &Manifest;
    fn instance(&self) -> Rc<dyn Plugin>;
    /// A file its pages load, by name, such as
    /// `tailwindcss-browser-4.3.4.js`.
    fn file(&self, name: &str) -> Option<PluginFile> {
        None
    }
}

pub struct PluginFile {
    pub content_type: &'static str,
    pub bytes: &'static [u8],
    /// The same bytes compressed with gzip.
    pub gzip: Option<&'static [u8]>,
}
```

The widget plugin's build compresses each library with gzip
(`crates/plugin-widget/plugin/build.rs`).

The factory, not a user's instance, answers, because a file is the same for
every user and a share's reader has no user.

### The route

`GET /api/plugins/{plugin}/files/{name}` serves a file with no session, as
`/api/html/` and the shares' routes do: a widget's frame is an opaque origin
and sends no cookie, and a share's reader is not signed in. The answer
carries:

- `Cache-Control: public, max-age=31536000, immutable`, since the name holds
  the version and a new version is a new name.
- `Content-Encoding: gzip` and `Vary: Accept-Encoding`, with the gzip copy,
  when the request accepts gzip, as every browser's does.
- `Access-Control-Allow-Origin: *`, since a frame's opaque origin fetches it.
- `X-Content-Type-Options: nosniff`.

The cache header does not save the fetches that matter: Chrome keeps nothing
in its HTTP cache for a frame whose origin is opaque, so every widget frame
that shows fetches Tailwind again. It did so from jsDelivr before, compressed
by jsDelivr's edge. The gzip copy keeps that cost near what it was: 75 KB
for Tailwind in place of 287 KB, where jsDelivr's Brotli sent 61 KB. A proxy
in front of the server may compress further.

A plugin the server does not have, or a name its plugin does not give,
answers 404, with `Access-Control-Allow-Origin: *` too, so a frame's fetch
reads the refusal. A plugin a user turned off still serves its files: they are
public code, the same in every release, and a share stays readable.

### The widget's document

The bootstrap loads Tailwind from the route by a root-relative address,
`<script src="/api/plugins/widget/files/tailwindcss-browser-4.3.4.js">`, so
a product page, a share's page and the web app's development server, which
passes `/api` on, all reach the backend that served the document. The
gallery has no backend: its development server serves the widgets'
documents and the same files at the same paths, in place of the blob URLs it
frames them by today, whose documents cannot resolve a root-relative
address.

The document a Host's browser draws for `demi widget create`'s screenshots
holds Tailwind's code in its own script element instead: that browser loads
the document as text and reaches no backend.

At the first copy, the bootstrap fetches html-to-image from the same route
and runs it with `define`, `exports` and `module` shadowed, so an AMD loader
or a CommonJS shim the agent's page set up does not take the library's
exports. When `window.htmlToImage` is still missing, the copy fails with
"The image library did not load." A failed fetch is not kept, so the next
copy tries again.

### The files in the repository

The two files sit in `crates/plugin-widget/plugin/vendor/` as published on
npm, each beside its `LICENSE`, and are embedded with `include_bytes!`.
`LIBRARIES` in `crates/xtask/src/notices.rs` lists both, so a release's
notices carry them. Upgrading one is replacing the file and its name.

### What an older page meets

A widget document is made at each request, so every widget, old ones
included, loads the served Tailwind after the upgrade. A frame an older page
still shows keeps the jsDelivr copy it loaded until it reloads.

## Spike

* **Whether a widget frame keeps the file.** A sandboxed frame without
  `allow-same-origin`, as a widget's is, loaded the file with a script
  element and with `fetch`, three frames in a row, under
  `Cache-Control: public, max-age=31536000, immutable`: all six requests
  reached the server. Frames that were not sandboxed sent one. In the
  product, three reloads of a conversation with one widget made three
  requests for Tailwind. Safari and Firefox were not tried.

## Prior art

- VS Code's webviews load an extension's own scripts from the extension's
  directory through `asWebviewUri`, never from a CDN the extension did not
  choose ([Loading local content](https://code.visualstudio.com/api/extension-guides/webview#loading-local-content)).
- JupyterLab's server serves each prebuilt extension's static files from the
  extension's package, under `/lab/extensions/{name}/static/`
  ([Prebuilt extensions](https://jupyterlab.readthedocs.io/en/stable/extension/extension_dev.html#prebuilt-extensions)).

Both serve the files from the server that runs the extension. Demi departs
in serving them without a session, because its frames are opaque origins
and its shares have readers without an account.

## Alternatives

- **Bundle them into the web app and hand them to the frame:** the parent page
  posts the library's code into the frame. Tailwind must run before the
  agent's page draws, before any message can arrive, so it cannot come this
  way; and two ways for the same purpose is one too many.
- **Serve them as the web app's static files:** the web app's build would own
  files only the widget plugin's documents use, and a server without the web
  app's files, as in development, would lack them.
- **Hold Tailwind in every document**, as the screenshots' document does:
  each frame then carries the same 287 KB in its document, which the
  backend serves uncompressed, and a page of ten widgets carries it ten
  times.
- **Keep jsDelivr and pin the version:** the version stops drifting, but a
  network that cannot reach jsDelivr still sees unstyled widgets.

## Open questions

None.
