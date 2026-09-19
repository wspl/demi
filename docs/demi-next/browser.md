# Conversation browser

Every command in the command reference belongs to the `demi browser` catalog.
The reference is not split into a shipped core and later extensions; a
capability that a platform or page cannot provide is reported unavailable with
its reason rather than omitted from the design. Platform delivery and
acceptance are required before enabling a deployed release.

This is the authoritative browser design. Command dispatch, Host admission,
native service ownership, and package boundaries retain their existing owners.
Browser automation does not introduce another shell or agent loop, and it does
not reach into the agent framework: the browser is a plugin behind the
[conversation-scoped state port](native-runtime.md#conversation-scoped-state).
It keeps its own state per conversation and ends it when the conversation is
released, as [Conversation idle and Host resource release](resource-lifecycle.md)
defines. Nothing in the agent, backend, runner, or protocol exists for the
browser specifically.

## Reading map

- [Purpose](#purpose): browser automation for agents on the conversation Host.
- [Ownership](#ownership): conversation binding, tabs, concurrency, and lifetime.
- [Release](#release): the conversation release ends the browser; the browser has no timer.
- [Cancellation](#cancellation): stop invocation work and release held input.
- [Execution](#execution): browser distribution, command dispatch, and conversation-scoped state.
- [Command contract](#command-contract): inputs, text, JSON, media, and failure.
- [Observation](#observation): page trees, references, locators, and coordinates.
- [Command reference](#command-reference): commands, example output, and result fields.
- [Bash workflows](#bash-workflows): consecutive actions and conditional scripts.
- [Acceptance](#acceptance): implementation responsibilities, checks, and prerequisites.

[Live browser view](browser-live-view.md) owns how the user watches and
operates these tabs in the work panel.

## Purpose

An agent starts an application at `http://localhost:3000` on Cloud, opens its
login page with `demi browser open`, fills the form, and checks the result using
page observations or an explicit screenshot. Chrome for Testing runs on that
conversation's Host; the commands operate the same pages and browser storage
across shell jobs and agent turns.

```text
Agent -> demi browser commands -> Host browser -> Application :3000
      <- text, JSON, screenshots <-
```

This document covers browser automation for agents. The work panel shows the
same tabs live and lets the user operate them at the same time;
[Live browser view](browser-live-view.md) owns that view, its capture and its
transport. There is no Managed/Free mode. Explicit screenshots continue
through the existing command media contract.

The design uses Codex's observable, programmable browser tools and readable
feedback as the primary reference when correcting browser API behavior. Where
the current design or Playwright behavior differs, follow the corresponding
Codex API's verified normal behavior. Reproduce and document obvious reference
defects separately; do not adopt them as the target behavior. Use the API's
contract and Playwright's normal implementation to resolve those cases.
Playwright implementation and tests provide supplementary references for the
affected APIs. Match the operation's purpose:
an accessibility value setter and a locator click are different operations.
It does not copy proprietary implementations,
protocols, or prompts, or embed another browser-use agent. Dependencies must not
require AGPL licensing.

## Ownership

### Conversation owns the browser

A conversation owns one Demi-managed browser environment on each Host where
its shell runs; the browser is wherever the command runs, and a command never
reaches a browser on another Host. The root agent and its children operate
tabs in that environment. The
environment contains an isolated browser storage context, tabs, a clipboard,
and debugging connections. It is not an agent-selectable session.

```text
Conversation
  +-- Current main Host
  |     +-- Managed browser environment
  |           +-- tab-a
  |           +-- tab-b
  +-- Agent tree
        +-- root
        +-- child
```

Commands obtain the conversation and invoking agent node from the invocation's
[command context](native-runtime.md#command-context), which the runner fills
from the job the backend started. A script cannot change it. Commands have no
`--conversation`, `--session`, `--profile`, `--cdp-url`, or browser-process
argument.

`open` starts the environment when necessary. `tabs` returns an empty list when
the browser has not started; listing alone does not launch it. A command with
an expired tab ID fails rather than opening a replacement page.

A shell on the main Host and a shell on an attached Host therefore use two
environments with separate tabs, cookies, and handles; a handle from one is
`tab_not_found` on the other. The browser does not know which role its Host
plays in the conversation and does not need to: the conversation release
reaches every device the conversation used.

Different conversations have isolated cookies, local storage, caches,
clipboards, and tabs. Tabs within a conversation share browser storage, so a
login can apply to more than one tab. This is product isolation, not an
additional security boundary against someone with shell access to the Host.
Cloud conversations of the same user already share files and application services.

### One tab registry

A tab ID is an opaque handle. An actual tab ID is `t_` followed by 22
base64url characters encoding a fresh 128-bit random value; a node reference is
`e_` with the same encoding. Examples abbreviate them to `tab-1` and `e3` for
readability. IDs are never reused across browser generations. An old ID must
never refer to a newly created tab. A display index is not an authorization token.

The Host browser driver owns one canonical live tab registry; `demi browser tabs`
reads it. All top-level pages appear, including pages opened by agents, sites,
and temporary content operations. Root and child agents share this registry.
Commands always name a tab explicitly; there is no globally selected tab.

Creation metadata is a discriminated value: `agent` with node ID, `page` with
opener tab ID, `temporary` with invoking node ID, or `user` for a tab the user
opens in the [live view](browser-live-view.md). This is diagnostic metadata,
not authorization. An iframe remains part of its top-level tab. A site-created
top-level tab is registered before it can be operated; the triggering action
reports observed `openedTabs`. Later popups appear in subsequent `tabs` calls.
The driver retains the public ID of each top-level page for the browser
generation, including closed tabs, so a popup can still report its opener after
it closes. Only top-level pages receive tab IDs; iframe and worker targets are
reached through `cdp targets`. The registry follows target events through a
bounded buffer. When that buffer overflows, the driver reconciles the registry
from the browser's current target list before serving the next command; a lost
event costs one reconciliation, never a failed `tabs` call.

Only one agent command executes against a tab at a time. Its operation lock
covers targeting, input, associated waits, and result collection. Conflicting
agent calls return `tab_busy`, without queuing a click for an unknown later page.
Different tabs may progress in parallel.

A shell script is not a transaction. Another agent can act between commands.
Agents needing independent sequences should use separate tabs; shared browser
storage still means those tabs can affect the same login.

### Cancellation

Cancelling a command stops its further browser dispatches, waits, and
blocking debug facilities, and releases held keys/buttons; cancelling an agent
turn reaches the browser only as the cancellation of its running command. Await bounded
cleanup before reporting cancellation complete. Browser state and nonblocking
observations such as buffered logs remain available. Cancellation does not undo
submitted forms or page JavaScript and does not close unrelated tabs.

Commands rejected before dispatch use `details.action: not_started`; completed
or uncertain effects keep their actual progress. No automatic replay occurs.

### Lifetime

The native browser service owns one environment per conversation, keyed by
the conversation identity of the invocation. The browser process and its
storage form the lazily created environment; nothing outside the native
service holds a handle to it.

The environment outlives individual commands, shell jobs, and agent turns.
Its internal lifecycle uses one state:
`absent | starting | ready | closing`. It does not also store contradictory
`running` and `finished` flags.

| Event | Required behavior |
| --- | --- |
| First `open` | Join concurrent environment startup; create a distinct tab for each successful caller |
| Command, shell job, or agent turn completes | Release invocation resources; retain the environment and tabs |
| User closes the Demi page | Retain the environment; closing the web client does not cancel agent work |
| User opens a tab in the live view | Same as `open`: start the environment when necessary and register the tab as `user` |
| User closes a tab in the live view | Same as `close <tab>`; closing the last tab retires the environment without asking |
| Command or agent turn is cancelled | Stop current invocation work and temporary waits; retain completed page effects and other tabs |
| `close <tab>` | Close that tab and release its commands, references, and debugging state |
| Last tab closes | Retire the environment after the closing invocation completes; stop Chrome for Testing and remove its profile; the next `open` starts fresh |
| Conversation release arrives | Retire the environment the same way; the backend sends it when the conversation has been idle for the idle window, moves to another Host, or is archived |
| Main Host or main directory changes | The old device receives the conversation release; nothing is copied to the new Host |
| Conversation is archived | The device receives the conversation release; restoring the conversation starts fresh on demand |
| Conversation is forked | Do not inherit the live browser or handles; IDs in copied history are historical text |
| Chrome for Testing crashes, runner connection ends, backend restarts, or Cloud stops/resets | Invalidate the environment and fail affected calls; never replay page actions |

A command still running when another command closes the last tab fails with
`browser_lost`; a command still running when the conversation release arrives
fails as `cancelled`, like any invocation cancellation. The next `open`, in the
same shell call or a later one, starts a fresh environment. Browser profiles and live page state are not restored across
process lifetime boundaries. Explicit output files survive according to their Host
location. Screenshots already persisted into the transcript follow the existing
media storage contract.

### Release

The browser has no idle timer and no notion of agent activity. It ends a
conversation's environment on exactly two signals it sees itself: the last tab
closing, and the conversation release the runner forwards. The backend decides
when to send that release from the one conversation idle rule, one hour of no
agent activity, or from a target change or archive; an open tab, a page that is
still loading, or a running download never postpones it. On Cloud the release
is not sent: an idle machine stops, and the browser ends with it.

Retirement stops Chrome for Testing, terminates its process tree, removes the
temporary profile, and invalidates every tab, reference, and debugging handle
together. A release that arrives while a command is running on that
conversation cancels the command like any invocation cancellation, then retires.
Repeating a release is harmless. The next `open` creates a fresh environment
with new tabs and browser storage.

## Execution

### Browser distribution

Demi uses **Chrome for Testing**, the Chromium-based Chrome distribution for
browser automation. The selected download is the `chrome` browser artifact.
The distribution provides versioned downloads without automatic updates, which
lets Demi verify and reproduce a browser release across Hosts. See the
[official distribution description](https://developer.chrome.com/docs/automation-and-testing/chrome-for-testing).

Demi manages this browser installation and its isolated conversation profiles.
It does not discover an arbitrary Chrome executable on PATH, attach to a user's
personal browser, or silently substitute a system Chromium installation. Browser
selection is product configuration, not an agent command option. Chrome for
Testing supplies the executable; the native driver uses chromiumoxide over CDP.
ChromeDriver is not required.

A browser release pins a complete version and a per-platform artifact record:
platform, download location, byte size and SHA-256 established by the Demi release
pipeline. Resolve official version/download metadata during release preparation;
Host startup installs the pinned artifact rather than resolving a moving channel
such as `latest`. Validate downloaded bytes before publishing the installation.
Paired-device installation and Cloud image preparation consume the same release
record. Browser and driver/CDP compatibility must pass acceptance together.

Platform availability is checked for the exact selected version against the
[official artifact matrix](https://github.com/GoogleChromeLabs/chrome-for-testing#supported-platforms).
A runner build target does not by itself establish browser support. Offer the
browser capability only on platforms with an available, verified artifact and
passing native/browser acceptance. A missing artifact reports an unavailable
capability; it does not trigger an alternative browser or architecture fallback.

Updates are explicit Demi release changes. Existing environments retain their
browser executable until released; new environments use the selected release.
Keep an old installation while any environment still uses it, then reclaim it
through the installer lifecycle. Fixed versions require a maintained update
process; they are not a promise to keep an old browser indefinitely. The first
version pin and installer/image implementation remain delivery prerequisites.

The first release pins Chrome for Testing `153.0.8010.36`. Its release record
contains official archive URLs, exact sizes and SHA-256 digests for supported
platforms. The native browser owner installs and verifies that release lazily
before its first open. Installation is shared across conversation profiles;
the guest image preinstalls the same verified archive and its Linux system libraries.
On Unix, the owner first checks the pinned installation under `/opt/demi/browsers`;
otherwise it installs under the Host user's `.demi/browsers`. Both locations use
the same receipt and executable integrity checks; an invalid installation fails
without falling back to another location. An unsupported
platform fails explicitly rather than using a different browser.

### Native driver

Chrome runs headlessly. Its browser toolbar, address-bar WebUI, and their preload
and process-overhead experiments are disabled: these internal interfaces are not
agent pages and must not consume renderers while an agent opens its application.
Page rendering and Chrome's sandbox remain enabled. The environment also loads
the live view's capture extension, with a fixed key so that its ID can be
allowlisted for tab capture ([Capture](browser-live-view.md#capture)).

Pages see an ordinary Chrome with no automation markers, because the agent and
the user test how real sites and applications behave for real visitors:

- Launch omits `--enable-automation` and `--hide-scrollbars` and disables the
  `AutomationControlled` Blink feature, so `navigator.webdriver` is false and
  scrollbars take their usual width.
- The user agent is the standard Chrome user agent of the Host's platform,
  without the headless token, for pages, workers and requests alike. Client
  hints keep the browser's own brands.
- On Linux, Blink settings declare a fine pointer with hover. A headless Linux
  browser otherwise reports no hover, and pages take their touch styles.
- The virtual screen and every window follow the
  [live view's pixel ratio](browser-live-view.md#pixel-ratio), and a window's
  outer size is never smaller than its viewport.
- Time zone and languages are the ones the user's browser last reported,
  which arrive in the starting invocation's
  [command context](native-runtime.md#command-context),
  applied through CDP when the environment starts; the Host's own settings
  differ between devices and do not count. They do not change while the
  environment lives.
- The Cloud guest ships fonts for Chinese, Japanese and Korean.

What cannot change without a GPU remains: on Cloud, WebGL reports its software
renderer.

The Rust browser module in `demi-commands` uses chromiumoxide for Chrome process
integration, typed CDP calls, page handles, and event decoding. Demi owns semantic
targeting, actionability checks, input ownership, and resource retirement. A
library helper is used only when its behavior matches the command contract;
a successful low-level input dispatch is not proof that the requested control
was enabled or unobstructed.

The dependency is maintained under `vendor/chromiumoxide` following the package
boundary rules. Its event subscriptions use bounded buffers and explicitly report
lost events. A slow listener must not stop control requests from progressing.
Loss of registry events invalidates the affected observation and must be
reconciled before further operations; log consumers report truncation. CDP
messages also have a finite transport size limit. An oversized message fails the connection
and enters normal browser-loss cleanup.

Read-only eval uses Chrome's enforced side-effect checking. The driver rejects
unsupported results and never falls back to unrestricted evaluation. Cancellation
stops further driver steps and releases held input, without claiming to undo
side effects that Chrome has already executed.

The browser environment owns its Chrome process tree, event task, and temporary
profile. On Unix, Chrome launches in a separate process group and inherits a
private environment marker identifying its profile owner. Helpers such as
Crashpad can detach into another session; retirement tracks these by that marker
as well as the group. The marker is read only from processes whose executable
lies inside the Chrome for Testing installation, so retirement never reads the
environment of every process on the Host. Retirement closes Chrome, reaps its
main process, terminates remaining group members and marked helpers, waits for
them to disappear within the control timeout, and joins the event task before
removing the profile.
Failed launch, owner cancellation, and failed graceful closure use the same
termination and reaping path. Profile removal retries only
“directory not empty” errors for at most 300 ms after process retirement; any
remaining cleanup failure retains the profile path and reports the failure.
Invocation cancellation does not retire an unrelated tab or owner.

### One command path

Browser commands are ordinary declared `demi` commands:

```text
Agent shell_exec
       |
       v
Brush on Host -> Declared-command dispatcher
                         |
                         v
                 Demi native command service
                         |
                         v
                Chrome for Testing / CDP
                         |
                         v
               stdout / stderr / exit status
                         |
                         v
               Existing shell result and media handling
```

Declaration, argument conversion, help, and `--json` follow
[Commands](commands.md) and [Command help](../command-help.md). Brush builtins call
the dispatcher directly; external programs use the same root alias. A browser
action does not launch another browser process or a new model-tool loop.

Browser algorithms and the driver belong to the native `demi.builtin`
implementation. Runner owns authenticated scope, processes, and transport.
Chrome for Testing and its driver run on the selected Host. The debugging
connection is not exposed directly to the public network or the Demi web app.

### Conversation-scoped state

The browser is [conversation-scoped state](native-runtime.md#conversation-scoped-state)
of the native service. The runner keeps the service resident while it holds any
conversation's browser, forwards the conversation release to it, and writes the
trusted conversation and caller identity into every invocation. The backend
starts jobs and sends releases; it holds no browser handle, grant, or tab
inventory. Cloud and paired devices use the same contract.

## Command contract

### Inputs

The root is `demi browser`. There are no agent commands to manage browser
sessions or processes.

Every tab operation takes an explicit `<tab>`. Only `open`, `tabs`,
`content fetch`, and generated help operate without an existing tab. Actions
do not depend on a mutable globally selected tab.

Ordinary arguments use the existing command system's scalar and repeated-array
fields. Multiple files use repeated `--file`, selected values use repeated
`--value`, and drag paths use repeated `--point`. Do not introduce another
command-line parser.

Finite stdin carries evaluation expressions, CDP parameter objects, WebMCP
arguments, complex `find --query` trees, and clipboard writes. Each input has
one source: for example, `eval` does not also accept a positional expression and
an `--expression` option. Text and JSON bodies use the command declaration's
`stdinField`; `find` reads that field only as a query when `--query` is selected.
Clipboard write uses the existing raw byte stdin channel so PNG bytes remain
unchanged; it has no argv body field. Help is resolved before any stdin read.

Validate CLI input, JSON bodies, Host files, CDP replies and events, and
page-provided tool schemas/results at entry. Types derive from schemas; casts
and silent repairs do not validate data. Browser arguments, results and event payloads have one schema authority in the
`browser-protocol` package. Command declarations consume those schemas;
native builds generate Rust contracts from them. Backend and UI consume the same
platform-neutral schemas without importing the coding harness, following
[Package boundaries](../package-boundaries.md).

Navigation accepts `http:`, `https:`, `file:`, and `about:blank`. File URLs refer
to the selected Host. `javascript:`, browser settings, extension pages, personal
browser profiles, and caller-provided CDP endpoints are not navigation inputs.

Paths refer to the invoking Host, with relative paths resolved against the
invocation cwd. Output files fail if they exist unless `--overwrite` is explicit.
Write to a temporary file and publish only when complete. Failure and
cancellation remove partial files. File effects use
[Edit tracking](edit-tracking.md), not another recorder.

### Default text

Default UTF-8 output is concise and readable, with page hierarchy preserved:

```text
Tab: tab-1 · Sign in
URL: http://localhost:3000/login

- main:
  - heading "Welcome back" [level=1]
  - textbox "Email" [ref=e1]
  - textbox "Password" [ref=e2] [protected]
  - button "Sign in" [ref=e3]
```

A simple action produces a short acknowledgement. Relevant navigation, dialogs,
or opened tabs appear on additional lines. It does not automatically return
the entire page tree or a screenshot after every action.

Page titles, text, attributes, and errors are quoted data. Escape control
characters, newlines inside values, and terminal sequences so page content
cannot forge a result header or command status. Observation of a password field
shows `[protected]`; filling it does not echo its value.

Text column widths and line numbers are not a parsing contract. Scripts use
structured results. Default output labels are English regardless of the page's
language; page content remains in its original language.

### JSON and exit status

Each structured leaf declares `output.json`. The existing `--json` path returns
one validated JSON value. Text and JSON are renderings of the same typed result;
neither is obtained by parsing the other.

```text
$ demi browser find tab-1 --role button --name 'Sign in' --json
{"matches":[{"ref":"e3","role":"button","name":"Sign in"}],"count":1,"truncated":false}
```

| Outcome | stdout | stderr | Exit code |
| --- | --- | --- | --- |
| Success, default | Readable result or pure media bytes | Necessary diagnostics | 0 |
| Success, `--json` | One schema-validated JSON value | Necessary diagnostics | 0 |
| Normal empty result | Empty-result text or valid empty collection | None | 0 |
| Normal false value | The false value | None | 0 |
| Operation failure | No success result | Text error, or error JSON under `--json` | 1 |
| Invalid arguments or input | Empty | Usage diagnostic | 2 |
| Cancellation | No success result | Cancellation or existing shell termination information | 130 when reportable |

Browser handlers own the error object `{error: {code, message, details?}}`.
This is not a claim that all existing Demi commands already use that error
format. Dispatcher-level usage errors retain the common command diagnostic.
Successful JSON stdout validation reuses the existing dispatcher contract.

An action failure records one progress value in `details.action`:
`not_started | completed | unknown`. It must not maintain several overlapping
booleans such as `clicked`, `submitted`, and `actionCompleted`.

### Images and large outputs

`screenshot <tab>` writes pure PNG bytes to stdout. The existing shell media
adapter detects the binary format and applies the model's media support and
size limits. Do not put status text before or after PNG bytes. If the model
cannot accept the image, preserve the existing explicit diagnostic; do not
claim that the agent saw it.

```bash
demi browser screenshot tab-1
```

With `--output`, save the file and return file information instead. To show the
saved image, use the existing file command:

```bash
demi browser screenshot tab-1 --output /tmp/login.png
demi file read /tmp/login.png
```

Never discover artifacts by parsing `Screenshot saved:` or arbitrary paths from
stdout. A webpage must not be able to cause file reads by forging output text.
`screenshot --json` requires `--output`; it returns metadata, not base64 image
content. `probe --output` likewise returns an annotated image path plus matches.

One shell stdout is one byte stream. Concatenating PNGs, or mixing log text with
PNG bytes, does not produce multiple media results. Save multiple images and
read them separately. Downloads, large page content, and bundles should also
be files rather than enormous inline results.

### Limits, timeouts, and cancellation

Define these initial protocol defaults once in the browser contract:

| Setting | Default or limit |
| --- | --- |
| Open, including browser installation/startup and first navigation | 5 minutes |
| Page actions, reads, navigation, waits, downloads, and exports | 30 seconds |
| Explicit `--timeout` | Milliseconds, 1–300000; no indefinite waits |
| Chrome process startup ceiling | 60 seconds; the invoking command can time out earlier |
| Inline inspect/find/content text | 64 KiB, with explicit truncation |
| List `--limit` | Default 100, maximum 1000 |
| One `content fetch` | At most 10 URLs |
| CDP event buffer per tab | At most 10000 events and 8 MiB, whichever comes first |
| Console buffer per tab | At most 1000 entries and 1 MiB |
| Finite text/JSON stdin | At most 1 MiB; binary clipboard input is not text |

A command has one deadline across startup, registry lookup, page work and result
collection. It does not restart the budget between those steps. On expiry, cancel
further work and await bounded input and file cleanup before returning the timeout.
The longer `open` default also covers a cold Host. All later commands use the
same 30-second default, including clicks that wait for navigation. An explicit
`--timeout` replaces the default for the whole call.

Text truncation is explicit. Truncated JSON results remain complete valid JSON
and contain `truncated: true`. If an atomic result cannot be represented within
the limit, return `result_too_large` with a suggestion to save or narrow it;
never output a JSON prefix as a successful result.

Event waits, driver calls, stdin, file IO, and output backpressure respond to
cancellation. Invocation listeners, temporary tabs, wait handles, and temporary
files share cleanup paths for success, failure, timeout, and cancellation.
Late events cannot complete a cancelled invocation.

Failure does not prove that an action never occurred. Connection loss after
input delivery can produce `outcome_unknown`. Do not automatically replay the
action. Cancellation cannot undo a submitted form or restore prior page state.

## Observation

### Page trees and references

`inspect` defaults to accessibility semantics: role, accessible name, hierarchy,
state, and actionable references. `--view dom` returns a filtered DOM structure,
not complete HTML source. `content read --format html` explicitly requests HTML.

Each node reports its role, its accessible name, and, when the accessibility
tree carries one, its current `value` as a separate field; name and value are
never merged. Values are strings or numbers; other AX value types, including
booleans, are omitted without failing the observation. Boolean states such as
`checked`, `disabled`, and `expanded` are
reported with their actual value, including `false`; a state the tree does not
carry is omitted rather than reported as false. A password input is marked
`protected` and never carries a value; the same rule makes `read --property
value` return `protected_value`. Default text output shows the same states and
value as the JSON result.

Structural text can lack a reference. Actionable nodes, scope containers, and
iframes must be referenceable. Inaccessible or unsupported frames are marked,
not presented as empty pages. Report actual Shadow DOM, cross-origin iframe,
and Canvas observation support through capabilities.

References bind the browser generation, tab, frame document generation, and
actual node. They are not short names for selectors:

- Full navigation, reload, frame replacement, and environment restart invalidate
  affected references.
- An ordinary same-document update does not invalidate every reference. A
  surviving node keeps its identity.
- Removing or replacing a node invalidates its reference. Never resolve the old
  reference by finding another node with the same text.
- Inspecting again does not reassign a previous reference to a different node.
- Returned references describe the observed page; they do not freeze the page
  until the next command.

The initial `inspect` returns a full bounded snapshot each time. It has no
shared mutable “last view” diff cursor that one agent can advance for another.

### Shared target grammar

Commands requiring an element use the same target grammar. Choose one primary
locator per call:

| Arguments | Meaning |
| --- | --- |
| `--ref e3` | Previously returned node |
| `--role button --name 'Sign in'` | Accessible role and name |
| `--label Email` | Associated label |
| `--placeholder 'Search products'` | Placeholder |
| `--text-match Phone` | Visible text |
| `--test-id checkout` | `data-testid` |
| `--css 'main .product'` | Standard CSS selector |
| `--xy 420,300` | Viewport point; only for supported pointer/media operations |

The three text locators are three different computations. `--role` with
`--name` matches the accessibility role and the computed accessible name.
`--label` matches the text of a form control's associated label: a `<label for>`,
a wrapping `<label>`, or `aria-labelledby`; a labelled control is found through
its label whatever its role, including native date and file inputs.
`--text-match` matches the element's visible rendered text; `aria-label`
contributes to the accessible name, not to visible text.

Name and text matching is case-sensitive substring matching by default;
`--exact` selects whole-string matching. Explicit `--name-pattern` and
`--text-pattern` accept ECMAScript regular-expression source (without flags)
instead of their literal fields. Malformed patterns fail with `invalid_input`
before matching.
Do not interpret slash-delimited text as a regex automatically. `--text-match`
locates an element; `--text` is action payload for fill/type/select-text. These
fields must never share a flag or be inferred from the presence of another flag.

`--within <ref>` restricts a container. Repeated `--frame <ref>` enters frames
from outermost to innermost. `--nth <n>` selects a zero-based match. All scope
references must belong to the target tab.

`find` can return multiple matches. `read --all` explicitly reads multiple
matches. A mutation requires one element unless the command inherently sets
several values on one element. When a locator matches several elements and
exactly one of them is visible, the action uses that visible element. Every
other multiple match, whether all hidden or several visible, fails with
`ambiguous_target`; there is no implicit first match. `--nth` selects
explicitly among all matches, hidden ones included. A locator with no match
fails with `target_not_found`, never with ambiguity.

Hidden matches exist only where the locator's own computation can see them.
`--css`, `--label`, `--text-match`, `--placeholder`, and `--test-id` match
DOM elements whatever their rendering. `--role` with `--name` matches
accessibility-tree nodes; an element the browser excludes from that tree, such
as one under `display: none`, has no role or name and is not a role/name match
at all, so it neither counts toward ambiguity nor toward `--nth`.

For composed locators, `find <tab> --query` reads a declarative tree from stdin:

```bash
demi browser find tab-1 --query <<'JSON'
{
  "within": {"match": {"role": "row"}, "hasText": "Order A"},
  "match": {"role": "button", "name": "Delete", "exact": true},
  "visible": true
}
JSON
```

The query schema defines:

- `match`: one primary element locator from the table, excluding coordinates.
- `within` and `frame`: another query resolving to one container/frame.
- `and` or `or`: query arrays for intersection or a deduplicated union.
- `has` and `hasNot`: descendant query filters.
- `hasText`, `hasNotText`, `visible`, and `nth`: result filters.

A query has one base: `match`, `and`, or `or`; scope and filters modify that base.
Nested scopes express nested frames. Results use DOM order. Unknown keys and
invalid combinations fail strict validation. This is data, not JavaScript.
Query mode cannot also supply ordinary target flags. References returned by a
query use the same node identity and invalidation rules as inspect.

### Actionability and coordinates

Each action checks the conditions it needs and then acts; it does not apply
another action's conditions:

| Action | Required element conditions |
| --- | --- |
| `click`, `check` | Visible, enabled, a stable bounding box after scrolling into view, and the click point hits the element or one of its descendants |
| `fill`, `type` | Visible, enabled, editable; no pointer hit test |
| `key` | Visible, enabled; the element is focused before input |
| `select` | Enabled; options are matched inside the control |
| `select-text` | Visible; the element renders the requested text |
| `upload` | Enabled; the element is a file input or a control that opens a file chooser |
| `download` | The conditions of `click` |
| `move`, `scroll` with an element | A bounding box after scrolling into view; no enabled or editable requirement |
| `type`, `key` without a target | None; input goes to the tab's current focus |
| `--xy`, `drag --point` | Every point lies inside the current viewport |

Visible means a rendered box with area and a visible computed `visibility`.
Enabled follows the control and its ancestors, including a disabled `fieldset`
and `aria-disabled`. Editable means an input or textarea that is not read-only,
or a contenteditable element. Stable means the bounding box is unchanged across
two consecutive animation frames within a bounded number of frames. The hit test
follows the element's shadow roots and frames, so a control inside a shadow root
is not treated as obstructed by its host; an element that another element
covers, or whose computed `pointer-events` is `none`, fails with `not_actionable`
naming the intercepting element once the deadline expires. Conditions are
rechecked after scrolling and before dispatch.

An absent or not-yet-ready semantic target waits until the deadline. An
explicitly stale reference, an ambiguous match, or a permanently inapplicable
element type, such as filling a `<div>`, fails immediately. There is no implicit
force or silent fallback from a failed element action to a coordinate action.

Coordinates are CSS pixels in the current viewport, with origin at its top left.
A screenshot has one image pixel per CSS pixel whatever the device pixel ratio,
so a point in a viewport screenshot is the same point in viewport coordinates.
`probe` uses the same CSS coordinates. Off-viewport
coordinates in a full-page screenshot are not current viewport coordinates.

## Command reference

The `$` below is a prompt, not part of command input. Omitted long content is
illustrative; actual truncation follows the explicit contract. Commands share
the target, timeout, output-file, and JSON rules above.

### Tabs and navigation

```text
$ demi browser open http://localhost:3000/login
Tab: tab-1
URL: http://localhost:3000/login
Title: Sign in

$ demi browser tabs
Tab     Title     Created by   URL
tab-1   Sign in   agent root   http://localhost:3000/login
tab-2   Admin     agent child  http://localhost:3000/admin

$ demi browser info tab-1
Tab: tab-1 · Sign in
URL: http://localhost:3000/login
Viewport: 1280 × 720 CSS px
Dialog: none

$ demi browser goto tab-1 http://localhost:3000/products
Navigated to http://localhost:3000/products.
Title: Products

$ demi browser back tab-1
Navigated to http://localhost:3000/login.

$ demi browser forward tab-1
Navigated to http://localhost:3000/products.

$ demi browser reload tab-1
Reloaded http://localhost:3000/products.

$ demi browser history tab-1
  0  Sign in   http://localhost:3000/login
* 1  Products  http://localhost:3000/products

$ demi browser close tab-2
Closed tab-2.
```

`open` returns the tab it created, never another concurrent caller's latest tab.
Navigation defaults to `domcontentloaded`; `--load commit|domcontentloaded|load`
changes the completion condition. `back` and `forward` return `history_boundary`
before issuing anything when no history entry exists. History is local to the
named tab.

`open`, `goto`, `reload`, `back`, and `forward` install main-document
observation before issuing the navigation and report that navigation only. A
new document completes at the selected load state; redirects remain part of the
same navigation. A same-document navigation, such as a fragment change,
completes when its event arrives without waiting for a load state that will
never fire. A main-document network failure reported by the browser, such as a
refused connection or a dropped response, fails with `navigation_failed` and
the browser's reason. An HTTP error status is a received document, not a
failure; the agent observes the page to judge it.

If `open` creates a tab but navigation times out, the error retains `details.tab`
so the agent can inspect it. Navigation failures report the current URL and
known action progress rather than implying nothing happened.

`tabs` and `history` accept `--offset` and `--limit`. Their listings are not
frozen across calls. Closing reports success only after tab resources are
released. An invalid or already closed handle returns `tab_not_found`.

### Inspect, find, and read

```text
$ demi browser inspect tab-1
Tab: tab-1 · Sign in
URL: http://localhost:3000/login

- main:
  - heading "Welcome back" [level=1]
  - textbox "Email" [ref=e1]
  - textbox "Password" [ref=e2] [protected]
  - button "Sign in" [ref=e3]

$ demi browser inspect tab-1 --view dom --within e10
- form [ref=e10]:
  - input type=email label="Email" [ref=e11]
  - button "Submit" [ref=e12]

$ demi browser find tab-1 --role button --name 'Sign in' --exact
1 match:
  [ref=e3] button "Sign in"

$ demi browser find tab-1 --text-match 'Missing product'
No matches.

$ demi browser find tab-1 --frame e30 --label Email
1 match:
  [ref=e31] textbox "Email"

$ demi browser read tab-1 --ref e21 --property text
Phone A

$ demi browser read tab-1 --ref e21 --attribute href
/products/a

$ demi browser read tab-1 --ref e3 --property enabled
true

$ demi browser read tab-1 --css '.product' --property text --all
[0] Phone A
[1] Phone B
```

Inspect's `--within` and `--frame` narrow observation scope. Find accepts
`--offset` and `--limit`. Read requires either
`--property text|text-content|html|value|visible|enabled|checked` or
`--attribute <name>`. `text` is rendered text; `text-content` is raw text
content; `html` is the element's outer HTML.
A missing attribute returns null. Reading a protected input's value fails with
`protected_value`. Multiple matches require `--all` or an explicit `--nth`.

### Screenshot and probe

```text
$ demi browser screenshot tab-1
<PNG bytes; the shell media adapter displays an image>

$ demi browser screenshot tab-1 --output /tmp/login.png
Screenshot saved: /tmp/login.png
Image: 1280 × 720 px, one pixel per CSS pixel
Viewport: 1280 × 720 CSS px, Web mode, rendered at device pixel ratio 2

$ demi browser screenshot tab-1 --full-page --output /tmp/page.png
Screenshot saved: /tmp/page.png
Image: 1280 × 2400 px, one pixel per CSS pixel
Viewport: 1280 × 720 CSS px, Web mode, rendered at device pixel ratio 2

$ demi browser screenshot tab-1 --clip 100,200,600,400 --output /tmp/region.png
Screenshot saved: /tmp/region.png
Image: 600 × 400 px, one pixel per CSS pixel

$ demi browser probe tab-1 --xy 420,300
[ref=e3] button "Sign in"
Bounds: x=360 y=280 width=120 height=40 CSS px

$ demi browser probe tab-1 --xy 420,300 --output /tmp/annotated.png
[ref=e3] button "Sign in"
Annotated screenshot saved: /tmp/annotated.png
```

Every screenshot, printed or saved, is scaled to CSS pixels: its size is the
rendered size divided by the device pixel ratio. The ratio follows whoever
watches the tab in the live view, or the agent's `viewport set --scale`
([Pixel ratio](browser-live-view.md#pixel-ratio)); scaling keeps the image the
model receives the same size whoever is watching. The readable output and the
result's `viewport.devicePixelRatio` tell the model that the page rendered at a
higher ratio and that the image was scaled.

`--full-page` and `--clip x,y,width,height` are mutually exclusive. Capture does
not silently resize the page or scroll through it to trigger more content.
Full-page capture does not promise to load unseen lazy content. Probe returns
candidate elements, roles, names, bounds, and available locator information.
`--include-non-interactable` includes ordinary nodes.

### Pointer actions

```text
$ demi browser click tab-1 --ref e3
Clicked button "Sign in" [ref=e3].

$ demi browser click tab-1 --role button --name 'Sign in'
Clicked button "Sign in" [ref=e3].

$ demi browser click tab-1 --xy 420,300
Clicked at 420,300.

$ demi browser click tab-1 --ref e21 --count 2
Double-clicked [ref=e21].

$ demi browser click tab-1 --ref e21 --button right
Right-clicked [ref=e21].

$ demi browser move tab-1 --xy 420,300
Pointer moved to 420,300.

$ demi browser drag tab-1 --point 100,200 --point 150,220 --point 300,250
Dragged from 100,200 to 300,250.

$ demi browser scroll tab-1 --dy 600
Scroll input delivered: dy=600.

$ demi browser scroll tab-1 --ref e40 --dy 300
Scroll input delivered to [ref=e40]: dy=300.
```

Click count is 1 or 2; button is left, middle, or right. Pointer commands accept
repeated `--modifier Alt|Control|ControlOrMeta|Meta|Shift`. A drag requires at
least two ordered points. Release pressed buttons and modifiers on every exit
path, including cancellation.

A click dispatches at the point computed after the stability check. Without
`--wait-url`, it installs a bounded main-document load observation before
input; when a load starts, it waits for that load within the call deadline.
Not observing a load is not a failure: a click that navigates nowhere succeeds.
Whether the page did what the agent intended is a separate observation.

Scroll accepts `--dx` and `--dy`, with at least one nonzero value. An element or
coordinate can select the scroll origin. Delivered input does not prove the
page moved: it may already be at a boundary. Observe again when that matters.

### Input and forms

```text
$ demi browser fill tab-1 --ref e1 --text test@example.com
Filled textbox "Email" [ref=e1].

$ demi browser type tab-1 --text hello
Typed into textbox "Email" [ref=e1].

$ demi browser key tab-1 --key Escape
Pressed Escape in the focused element.

$ demi browser type tab-1 --ref e1 --text '.test'
Typed into textbox "Email" [ref=e1].

$ demi browser key tab-1 --ref e1 --key Enter
Pressed Enter in [ref=e1].

$ demi browser key tab-1 --ref e1 --key ControlOrMeta+A
Pressed ControlOrMeta+A in [ref=e1].

$ demi browser check tab-1 --ref e5 --value true
Checkbox [ref=e5]: checked.

$ demi browser check tab-1 --ref e5 --value false
Checkbox [ref=e5]: unchecked.

$ demi browser select tab-1 --ref e6 --option-label Singapore
Selected: Singapore (SG).

$ demi browser select tab-1 --ref e6 --value SG --value JP
Selected: SG, JP.

$ demi browser select-text tab-1 --ref e7 --text 'Replace this'
Selected text in [ref=e7].

$ demi browser select-text tab-1 --ref e7 --text 'Replace this' --cursor before
Cursor placed before the matching text in [ref=e7].
```

Without a target, `type` and `key` deliver input to the tab's current focus,
following focus into frames, without changing it and without checking element
conditions: this is the keyboard, not an element action. The result names the
focused element when the accessibility tree has a node for it and otherwise
reports the document. Untargeted `key` still supports `--wait-url`.

Fill handles the control's native type. Text-like inputs, textareas, and
contenteditable elements are focused, their contents selected, and the text
inserted as native text input; an empty text deletes the selection instead.
Date, time, datetime-local, month, week, color, and range inputs receive the
value through the control's native value setter and fail with `invalid_input`
when the browser
rejects it, for example a malformed date. Number inputs require numeric text.
Any other element fails immediately as not fillable. Fill does not click the
control and does not assert that the final value equals the input; page scripts
may transform or refuse it, and the agent observes the result when it matters.

Type focuses the target without clicking, keeps an existing selection, and emits
key events for each character in order. If the target is replaced, loses focus,
or its frame changes while typing, typing stops with `not_actionable` and the
error reports how many characters were delivered.

Key focuses the target, then presses one key or a `+`-joined combination such
as `ControlOrMeta+A`. Modifiers are `Alt`, `Control`, `Meta`, `Shift`, and
`ControlOrMeta`, which maps to Meta on macOS Hosts and Control elsewhere. Key
names are validated before any input; an unknown name is `invalid_input`, not
literal text. Every pressed key and modifier is released on every exit path.

Check reads the current state first; an already matching state succeeds without
a click. Otherwise it clicks the control and reads the state again, failing
with `not_actionable` when the page prevented the change. Unchecking a radio
button is `invalid_input`. Native checkboxes and radios and elements with the
ARIA `checkbox`, `radio`, or `switch` role are supported.

Select accepts one of repeated `--value`, `--option-label`, or `--option-index`,
not a mixture. `--label` still locates the select element itself. Options are
matched in document order: a single-select control takes the first matching
candidate. A multi-select control consumes each supplied candidate at its first
matching option in document order; a later option with the same value is not
selected by that candidate again. Every supplied candidate must have a match
for a multi-select. A candidate that has not appeared yet is waited for until
the deadline; a matching disabled option fails with `not_actionable`; a candidate
that never appears fails with `target_not_found`. The control receives `input`
and `change` events, and the result lists the values actually selected.
Select-text defaults
to selecting the match; `--cursor before|after` positions a cursor instead.
`--prefix` and `--suffix` disambiguate repeated text; remaining ambiguity fails.

There is no accessibility-action command. The Chrome DevTools Protocol
Accessibility domain observes nodes but exposes neither an advertised action
list nor an action dispatch, and inferring actions from roles or substituting
DOM clicks would misreport what happened. Expanding, collapsing, or activating
a control is done with the pointer and keyboard commands.

### Waiting

```text
$ demi browser wait tab-1 --role heading --name 'Welcome back' --state visible --timeout 10000
Matched heading "Welcome back" [ref=e50].
State: visible.

$ demi browser wait tab-1 --ref e51 --state hidden --timeout 5000
Element [ref=e51] is hidden.

$ demi browser wait tab-1 --url '**/dashboard' --timeout 10000
URL matched: http://localhost:3000/dashboard.

$ demi browser wait tab-1 --load domcontentloaded
Load state reached: domcontentloaded.

$ demi browser click tab-1 --ref e3 --wait-url '**/dashboard' --timeout 10000
Clicked button "Sign in" [ref=e3].
Navigated to http://localhost:3000/dashboard.
```

Wait selects exactly one condition: an element with
`--state attached|detached|visible|hidden|enabled`, `--url`, or `--load`.
`--load commit|domcontentloaded|load` waits until the tab's current document
reaches that state; a document already past it satisfies the wait immediately.
Replacing that document before the state is reached fails with `navigation_failed`.
It does not wait for a future navigation. URL patterns define `*`
as any string without `/`, `**` as any string, and all other characters
literally, including `?`, brackets, and braces. They are not implicit regular
expressions.

Element states use the same computations as the actionability conditions.
`visible` requires a rendered box and visible `visibility`; `hidden` is
satisfied when no element matches or none is visible, which includes removal
within the same document; `attached` and `detached` follow node connection;
`enabled` follows the control and its ancestors. If navigation replaces a
referenced document, return `stale_ref` rather than treating disappearance of
the old page as a condition on the new page.

`wait --url` succeeds immediately when the current URL already matches. An
action's `--wait-url` installs URL observation before delivering input, then
delivers the input whether or not the URL already matches, and succeeds once
the URL observed after delivery matches; an already matching URL therefore
satisfies it. URL changes are observed through navigation events, so a matching
document that is replaced again quickly is not missed. Key supports the same
option for Enter submission. There is no generic `--page-changed`: an animation
or clock update cannot establish that pagination finished.

Element and URL waits work on pages with long-lived WebSockets or polling.
The initial design does not use network-idle as a business completion signal.
Fixed delays use the existing shell `sleep` command.

### JavaScript dialogs

```text
$ demi browser dialog inspect tab-1
Dialog: confirm
Message: "Delete this record?"

$ demi browser dialog accept tab-1
Accepted confirm dialog.

$ demi browser dialog dismiss tab-1
Dismissed confirm dialog.

$ demi browser dialog accept tab-1 --text 'New filename'
Accepted prompt dialog.

$ demi browser dialog inspect tab-1
No dialog.
```

Support alert, confirm, prompt, and beforeunload. Text is accepted only for a
prompt. An alert closes through dismiss; accept returns `invalid_dialog_action`.
Inspecting when no dialog exists succeeds with null; accept/dismiss fail with
`dialog_not_found`.

A command blocked by a dialog reports it and releases the operation lock so a
subsequent dialog command can run. It must not wait indefinitely while holding
the lock required to dismiss the dialog. Closing a tab explicitly releases the
whole resource and does not wait for a beforeunload confirmation.

### Upload, download, and clipboard

```text
$ demi browser upload tab-1 --ref e60 --file /tmp/avatar.png
Attached 1 file through [ref=e60].
  /tmp/avatar.png

$ demi browser upload tab-1 --ref e60 --file /tmp/a.pdf --file /tmp/b.pdf
Attached 2 files through [ref=e60].

$ demi browser download tab-1 --ref e61 --output /tmp/report.pdf
Downloaded: /tmp/report.pdf
Suggested filename: report.pdf
Bytes: 48320

$ demi browser download tab-1 --xy 500,300 --output /tmp/image.png
Downloaded: /tmp/image.png
Bytes: 20480

$ demi browser clipboard write tab-1 <<'TEXT'
Hello
TEXT
Clipboard written: text/plain, 6 bytes.

$ demi browser clipboard read tab-1 --format text
Hello

$ demi browser clipboard write tab-1 --mime image/png < /tmp/image.png
Clipboard written: image/png, 20480 bytes.

$ demi browser clipboard read tab-1 --output-dir /tmp/clipboard
Clipboard exported:
  text/plain: /tmp/clipboard/item-0.txt
  image/png: /tmp/clipboard/item-1.png
```

Upload targets a file input or a control opening a chooser. Validate files first,
install chooser observation, and then trigger the control. Multiple files on
a non-multiple input fail. Completion means attachment to the page, not receipt
by the application server.

Download installs observation before the trigger and publishes the output only
after completion. Supported media targets can download through the current page.
Cancellation and failure remove partial files. If output is omitted, generate
and return a temporary Host path. Suggested filenames are data, never permission
for path traversal.

An ordinary click may report an observed download, but that is not a completed
file result. Workflows requiring a saved file use download. Never click again
automatically to recover an event missed by an earlier command.

The clipboard belongs to the managed environment, not the user's system
clipboard. Write reads finite raw stdin, defaulting to UTF-8 text/plain;
trailing newlines are preserved. Read chooses either `--format text` or
`--output-dir` for supported MIME entries. Binary input must be validated against
its declared supported media type and bounded before replacing clipboard data.
Supported MIME types are `text/plain`, `text/html`, and `image/png`. Text and
HTML are limited to 1 MiB; PNG input is limited to 16 MiB encoded and 16 million
decoded pixels, checked before decoding or replacing clipboard data.

Clipboard commands use the tab's Clipboard API with read and write permission
granted to the environment's browser context, so page copy and paste see the
same data. Whether a platform's headless Chrome keeps its own clipboard or
routes it to the Host user's system clipboard is verified per platform in
acceptance; on a platform where it reaches the system clipboard, the
`clipboard` capability is unavailable with that reason and the commands fail
with `unsupported_capability`.

### Evaluation, console, and viewport

```text
$ demi browser eval tab-1 <<'JS'
document.querySelectorAll('.product').length
JS
12

$ demi browser eval tab-1 --ref e21 <<'JS'
element.textContent
JS
"Phone A"

$ demi browser logs tab-1 --level error --limit 20
[error] Failed to load orders
  http://localhost:3000/orders

$ demi browser viewport set tab-1 --width 390 --height 844
Viewport: 390 × 844 CSS px, device pixel ratio 1, custom.

$ demi browser viewport set tab-1 --width 1440 --height 900 --scale 2
Viewport: 1440 × 900 CSS px, device pixel ratio 2, custom.

$ demi browser viewport reset tab-1
Viewport: 1280 × 720 CSS px, device pixel ratio 1, web.
```

Eval is read-only page inspection. The stdin expression can access document;
a unique target additionally exposes element, or explicit `--all` exposes
elements. All bound elements must belong to one frame document; a set spanning
frame documents fails with `unsupported_capability` and must be narrowed with
`--frame` or `--within`. Chrome cannot bind remote objects from separate execution
contexts into one read-only expression. Results must be JSON-representable. Functions, DOM objects, cycles,
and other unsupported values fail rather than silently losing information.
The default is a compact JSON representation of the value; `--json` returns
`{value: ...}`.

Read-only behavior must be enforced by the execution mechanism, not merely by
instructions to the agent. A driver unable to prevent DOM, storage, or network
side effects must not advertise this capability. It must not substitute
unrestricted Runtime.evaluate. Getters requiring side effects must fail instead
of weakening the restriction.

Logs accept repeated `--level`, substring `--filter`, `--after` cursor, and
`--limit`. Without after, read the most recent limited entries. Cursor reads
return entries after that sequence. Eviction sets truncated. Reading must not
clear shared logs and make another agent lose its observations.

A tab's viewport has a mode ([Modes](browser-live-view.md#modes)). In Web
mode, the default, it follows the live view's panel and the viewer's pixel
ratio; a tab nobody has watched is 1280 × 720 CSS pixels at ratio 1. `set`
puts the named tab in Custom mode with the given size and `--scale`, 1 by
default, until the user picks another mode in the live view. `reset` returns
the tab to Web mode. Custom mode does not emulate mobile hardware, touch, user
agent, or network conditions; the user's Mobile mode emulates a phone.
Screenshots must not silently change viewport settings.

### CDP commands and events

```text
$ demi browser cdp targets tab-1
Target    Kind     URL
main      page     http://localhost:3000/orders
frame-1   iframe   https://example.com/widget
worker-1  worker   http://localhost:3000/worker.js

$ demi browser cdp send tab-1 Network.enable <<'JSON'
{}
JSON
CDP Network.enable completed.
Result: {}

$ demi browser cdp events tab-1 --method Network.responseReceived
Cursor: 120
No events. Use --after 120 to read subsequent events.

$ demi browser reload tab-1
Reloaded http://localhost:3000/orders.

$ demi browser cdp events tab-1 --method Network.responseReceived --after 120 --timeout 5000
[124] Network.responseReceived
  requestId: r1
  url: http://localhost:3000/api/orders
  status: 500
Cursor: 124
Has more: false
Truncated: false

$ demi browser cdp send tab-1 Network.getResponseBody <<'JSON'
{"requestId":"r1"}
JSON
CDP Network.getResponseBody completed.
Result: {"body":"{\"error\":\"database unavailable\"}","base64Encoded":false}

$ demi browser cdp detach tab-1
Detached debugging from tab-1.
```

CDP exposes tab debugging through the
[Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/),
not another Host access channel. Pin its supported version with the browser
build and validate commands/events against generated protocol schemas. Unknown
methods, malformed parameters, and malformed replies are not trusted objects.

Send takes a positional method and a finite stdin parameter object. Supply `{}`
for a method with no parameters. Events accepts repeated `--method`, `--after`,
`--limit`, and `--timeout`. Without after, it returns the current cursor rather
than all history. Without timeout, it returns immediately; explicitly waiting
uses the common bounded timeout range.

Event sequences increase monotonically within a tab debugging generation.
Continue using the returned cursor while hasMore is true, keeping filters and
child target unchanged. Evicted events set truncated. An expired generation
returns `stale_cursor`.

`cdp targets` lists only the named tab and its debuggable child targets. It uses
the common offset/limit rules; returned handles expire with their targets.
A returned child target can be selected with `--target` for an iframe or worker
belonging to the tab. Caller-supplied external sessions, browser-wide process
control, profile/file-path reconfiguration, and Target operations that create,
close, or move contexts are unavailable. They cannot bypass tab ownership.

Method admission is by domain and method, checked before the send. The
`Target`, `Browser`, `SystemInfo`, `Tethering`, and `HeadlessExperimental`
domains, and the methods `Page.close`, `Page.crash`, and
`Page.setDownloadBehavior`, return `cdp_method_denied`; every other method of
the pinned protocol version is allowed on the tab or a child target. The
capability schema publishes this rule as the denied list, not as a copy of the
protocol.

Read-only eval and CDP have different contracts: authorized tab debugging can
change page state, such as installing a breakpoint. Do not report such a command
as read-only inspection.

Debug state lives in a debugging connection that the browser owns per tab and
per calling agent node; Chrome discards every domain subscription, breakpoint,
pause, and interception installed through a connection when that connection
closes, so the browser tracks connections, not methods. A connection lasts
until one of three events the browser itself observes: the same caller runs
`cdp detach <tab>`, the tab closes, or the browser retires. Nothing outside the
browser signals it; agent turns and shell jobs are not a scope the browser
knows. Cancelling a `cdp send` or `cdp events` call in flight closes that
caller's connection; cancelling only the bounded wait of `cdp events` does not.
Tab-owned observations such as buffered logs are not debug state and remain
until the tab is released.

A caller that leaves interception or a pause behind blocks the page for every
caller until it detaches. When a command on that tab times out while another
caller's connection is open, the timeout error names that caller and the tab in
`details.tab` and `details.debuggingCallers` (an array of calling node IDs), so the
agent can ask for the detach rather than guess. There is no
automatic detach on behalf of another caller.

### Content and assets

```text
$ demi browser content read tab-1 --format text
Title: Products
URL: http://localhost:3000/products

Welcome to the sample store.

$ demi browser content read tab-1 --format html --output /tmp/page.html
Content saved: /tmp/page.html
Format: html

$ demi browser content fetch --url https://example.com/a --url https://example.com/b --format text
URL: https://example.com/a
Title: Page A
Page A content.

URL: https://example.com/b
Title: Page B
Page B content.

$ demi browser assets list tab-1
Inventory: assets-1
 a1  image       http://localhost:3000/logo.png
 a2  font        http://localhost:3000/font.woff2
 a3  stylesheet  http://localhost:3000/style.css

$ demi browser assets export tab-1 --inventory assets-1 --kind image --output-dir /tmp/assets
Exported 1 asset to /tmp/assets.
Manifest: /tmp/assets/manifest.json
Failures: 0
```

Content read/fetch supports text, html, and dom. DOM uses the same observation
implementation, not another extractor. Fetch uses temporary tabs in the same
conversation environment, registers the batch before navigation, and returns
results in input URL order. Keep the batch tabs until result collection ends so
closing one cannot tear down the environment mid-fetch. Pages execute scripts
and share login state; this is not a side-effect-free HTTP fetch. Temporary tabs
close on success, failure, and cancellation. Explicit closure by another agent
cancels the corresponding fetch item and reports `tab_not_found`.

Assets list inventories currently observed resources and inline SVGs. Export
uses that inventory, selecting either repeated `--id` or repeated
`--kind font|image|stylesheet|video`. Navigation invalidates the inventory.
Do not navigate to resource URLs as a substitute for asset acquisition.
The manifest records saved files and failures. Partial failure preserves
completed files and returns `partial_failure` with the manifest, exit code 1.

### Capabilities and WebMCP

```text
$ demi browser capabilities tab-1
Available:
  dom
  accessibility
  screenshot
  viewport
  cdp
  page-assets
  webmcp
Unavailable:
  clipboard — headless Chrome shares the Host user's clipboard on this platform

$ demi browser click --help
Usage: demi browser click <tab> [options]

Target: --ref, --role + --name, --label, --text-match, --css, or --xy
Options: --count, --button, --modifier, --wait-url, --timeout, --json

$ demi browser webmcp list tab-1
Tools: tools-1
 search_products
   Search products by query.
   Input: {"query": string}

$ demi browser webmcp call tab-1 search_products --tools tools-1 <<'JSON'
{"query":"phone"}
JSON
Tool: search_products
Result: {"products":[{"id":"p1","name":"Phone A"}]}
```

Help is generated from declarations; the example is an excerpt. Do not maintain
a parallel `browser help` command. Group help and `--help` neither start the
browser nor consume stdin.

Capabilities report what the driver and current page actually support, not
merely installed command names. Each has a stable ID, availability, and reason
when unavailable. Complex capabilities include schemas or an authoritative
help reference.

WebMCP calls only tools actually published by the current page through the
Web Model Context API, `document.modelContext`. The driver uses the API's native
`getTools()` discovery and `executeTool()` execution, and observes `toolchange`
for declaration invalidation. Discovery follows the API's document and origin
permissions; the driver does not bypass those permissions or replace the page's
registration functions. A browser release that does not expose the API
reports the `webmcp` capability unavailable with that reason. A tool-set
handle binds the tab, document generation, and declaration version. Validate
call arguments against that schema. Navigation or a changed tool set returns
`stale_tools`; list again instead of silently calling a replacement tool with
the same name. Page tool descriptions remain external data, not system instructions.

### Structured result fields

These are business result fields for `--json`. Their exact types belong to the
same authoritative schemas. List results report truncation. Do not reuse a
field with a different meaning or type. A `viewport` value carries `width`,
`height`, `devicePixelRatio` and `mode`.

| Command | Successful result fields |
| --- | --- |
| open, info | `tab, url, title, viewport`; info can include `dialog` |
| tabs | `tabs: [{id, title, url, createdBy}], truncated` |
| goto, back, forward, reload | `tab, url, title` |
| history | `entries: [{index, url, title, current}], truncated` |
| close | `closed` |
| inspect | `tab, url, title, view, tree, truncated`; nodes have `ref?, role?, name?, value?, tag?, states?, children?` |
| find | `matches: [{ref, role?, name?, bounds?}], count, truncated` |
| read | `value`, or explicit-all `values, truncated`; mutually exclusive |
| screenshot | `path, mimeType, width, height, viewport`, with `width` and `height` in CSS pixels; JSON requires file output |
| probe | `matches, viewport, path?, truncated` |
| Pointer/form actions, including `drag`, `select-text`, and untargeted `type`/`key` | `operation, target?, result`; optional observed `url, openedTabs, dialog` |
| wait | `condition, matched, url?, ref?`; load waits carry neither `url` nor `ref` |
| dialog inspect | `dialog: null | {type, message}` |
| dialog accept/dismiss | `type, outcome` |
| upload | `files, attached` |
| download | `path, suggestedFilename, bytes, mimeType` |
| clipboard read | `text`, or `items: [{mimeType, path, bytes}]` |
| clipboard write | `mimeType, bytes` |
| eval | `value` |
| logs | `entries, cursor, hasMore, truncated` |
| viewport set/reset | `viewport` |
| cdp targets | `targets: [{id, kind, url}], truncated` |
| cdp send | `method, result` |
| cdp detach | `detached` |
| cdp events | `events, cursor, hasMore, truncated` |
| content read | `url, title, format, content, truncated`, or `url, title, format, path` |
| content fetch | `pages: [{requestedUrl, url, title, content, error?}], truncated` |
| assets list | `inventory, assets, inlineSvgs, truncated` |
| assets export | `directory, manifest, files` |
| capabilities | `capabilities: [{id, available, reason?, schema?}]` |
| webmcp list | `tools, entries: [{name, description, inputSchema, outputSchema?}], truncated` |
| webmcp call | `name, result` |

Inspect JSON preserves the tree. A script needing a flat match list uses find,
or explicitly traverses the tree with jq. Find's count is the total current
match count, even if offset/limit restricts the returned matches.

### Errors

```text
$ demi browser click tab-1 --ref e3
Error: stale_ref
Element [ref=e3] belongs to an expired document. Inspect the page again.
Action: not_started.

$ demi browser click tab-1 --role button --name Delete
Error: ambiguous_target
Found 2 matching buttons:
  [ref=e10] inside row "Order A"
  [ref=e20] inside row "Order B"
Action: not_started.

$ demi browser click tab-1 --ref e3 --wait-url '**/dashboard' --timeout 5000
Error: timeout
The click completed, but the expected navigation was not observed.
Action: completed.
Current URL: http://localhost:3000/login
```

The last failure has this JSON representation on stderr:

```json
{
  "error": {
    "code": "timeout",
    "message": "The click completed, but the expected navigation was not observed.",
    "details": {
      "action": "completed",
      "tab": "tab-1",
      "url": "http://localhost:3000/login"
    }
  }
}
```

Every action failure carries `details.action`: `not_started` when the failure
occurred before input was delivered, such as locating, condition checks, a
blocking dialog, or invalid input; `completed` when input was delivered and a
later wait failed; `unknown` when the connection was lost after delivery began.
A follow-up observation after a completed action, such as reading the current
URL for the result, does not turn the action into a failure: the result reports
the action and omits unavailable metadata. Navigation results always include
the observed URL; title and the open viewport can be omitted. `info` remains
an explicit metadata read and can fail. Each failure keeps its own cause;
a missing tab, a lost browser, zero matches, several matches, a history
boundary, an existing output file, and a driver error are never folded into
one generic code.

Messages explain the situation; scripts inspect code and typed details:

| Code | Meaning |
| --- | --- |
| `invalid_input` | Arguments or finite input violate the contract |
| `tab_not_found` | Missing, closed, or inaccessible tab; other tabs are unaffected |
| `tab_busy` | Another agent command owns the tab operation lock |
| `stale_ref`, `stale_cursor`, `stale_inventory`, `stale_tools` | Expired object or generation |
| `target_not_found`, `ambiguous_target`, `not_actionable` | Zero matches, disallowed multiple matches, or a target that did not meet its action's conditions; details name the missing condition or intercepting element |
| `timeout` | Deadline expired; details report known action progress |
| `dialog_blocked`, `dialog_not_found`, `invalid_dialog_action` | Blocking or inapplicable dialog state |
| `history_boundary` | No previous or next navigation entry |
| `navigation_failed` | The browser reported a main-document network failure for an explicit navigation |
| `protected_value`, `side_effect_rejected`, `unsupported_result` | Disallowed inspection, side effect, or unrepresentable value |
| `unsupported_capability`, `cdp_method_denied` | Unsupported or unavailable operation in this scope |
| `output_exists`, `io_error`, `result_too_large` | File or result-size failure |
| `partial_failure` | Some batch items failed; details retain item results or manifest |
| `driver_error` | The browser driver failed without a more specific cause, including an unusable CDP response or an event-stream gap |
| `browser_unavailable`, `browser_lost` | Environment could not start or an existing one was lost |
| `cancelled`, `outcome_unknown` | Cancellation or unconfirmed outcome |

Host offline, failed Cloud wake, archive, and reset errors retain their Host
meaning; they do not become missing-element errors. Tool success and application
success are different: a delivered click followed by “Incorrect password” is a
successful click, and a subsequent observation establishes the login failure.

## Bash workflows

### Login in one shell call

```bash
set -euo pipefail

tab=$(demi browser open http://localhost:3000/login --json | jq -r '.tab')
demi browser inspect "$tab"
demi browser fill "$tab" --label Email --text test@example.com
demi browser fill "$tab" --label Password --text test-password
demi browser click "$tab" --role button --name 'Sign in' --wait-url '**/dashboard'
demi browser inspect "$tab"
```

One shell call executes this known sequence without another model decision
between commands. Browser commands implement bounded waits; Bash supplies
ordering, conditions, and loops. Return observations to the agent when the next
step needs interpretation rather than predetermined logic.

This fixture assumes a successful login navigates to dashboard. An application
that updates the current page requires a corresponding element condition.
The example does not define how real credentials are obtained. Confirmation
output avoids password echoes, but literals authored into scripts already
belong to the transcript.

### Search across pages

This fixture starts on page 1. Its next-page link is named “Next”, and the
current-page marker has test IDs `current-page-1`, `current-page-2`, and so on:

```bash
set -euo pipefail

tab='tab-1'

for attempt in {1..20}; do
  matches=$(demi browser find "$tab" --role link --name 'Pending order' --exact --json)
  count=$(jq -r '.count' <<< "$matches")

  if [[ "$count" == 1 ]]; then
    ref=$(jq -r '.matches[0].ref' <<< "$matches")
    demi browser click "$tab" --ref "$ref"
    demi browser inspect "$tab"
    exit 0
  fi

  if [[ "$count" -gt 1 ]]; then
    echo 'Multiple pending orders; the agent must choose one.' >&2
    exit 1
  fi

  if [[ "$attempt" == 20 ]]; then
    break
  fi

  next=$(demi browser find "$tab" --role link --name Next --exact --json)
  next_count=$(jq -r '.count' <<< "$next")
  if [[ "$next_count" == 0 ]]; then
    echo 'Reached the last page without finding the order.' >&2
    exit 1
  fi
  if [[ "$next_count" != 1 ]]; then
    echo 'Next-page link is ambiguous.' >&2
    exit 1
  fi

  next_ref=$(jq -r '.matches[0].ref' <<< "$next")
  demi browser click "$tab" --ref "$next_ref"
  expected=$((attempt + 1))
  demi browser wait "$tab" --test-id "current-page-$expected" --state visible
done

echo 'Stopped at the 20-page search limit.' >&2
exit 1
```

The application must actually expose those markers. Otherwise inspect its
pagination behavior and choose a real URL or element condition. This is not a
universal pagination script.

`set -euo pipefail` retains normal shell semantics; it is not a transaction or
universal exception mechanism. Conditions have their usual nonzero-status rules.
Extract needed values directly from complete JSON with jq instead of using
`head` to close a large producer's pipe early.

### Screenshots after a batch

```bash
set -euo pipefail

demi browser viewport set tab-1 --width 390 --height 844
demi browser screenshot tab-1 --output /tmp/mobile.png
demi browser viewport reset tab-1
```

A script needing restoration on every exit should use a shell trap and surface
cleanup failures. Then read `/tmp/mobile.png` separately with `demi file read`
to inspect the image. Browser commands do not add another batch language or a
persistent JavaScript REPL; Bash already composes their operations.

## Acceptance

### Implementation ownership

[Package boundaries](../package-boundaries.md) remains authoritative:

- `browser-protocol`: shared browser schemas and derived types.
- `coding-agent`: declared commands using those schemas and generated help;
  assemble available commands through injected capabilities.
- `backend`: builds the command context of every job and sends the generic
  conversation release; no browser module.
- `runner` and `host-remote`: the command context of every invocation, service residency,
  the release forward, cancellation, and transport; no webpage algorithms.
- `demi-commands`: driver, per-conversation environments, page observation,
  actions, output rendering, assets, and CDP handling.
- `command-service`: generic invocation and conversation protocol, not page or
  cookie semantics.
- `web-ui`: the [live view](browser-live-view.md#responsibilities), with the
  generic user stream in runner, host-remote and backend.

A library adopted for locating or acting must also cover the adjacent waiting,
introspection, and error handling it provides. Reuse one observation/targeting
implementation across commands rather than parallel AX, DOM, and coordinate
business state. Command naming need not mirror every method in a library API.

### Required checks

Use real Chrome for Testing against local page fixtures, scripted providers,
and isolated storage. Never run tests that call real models.

1. Run `open → inspect → fill → click → inspect` on paired devices and Cloud;
   verify localhost, files, and screenshots belong to the correct Host. Check
   fixture reachability from each Host before the run and record an
   environment failure separately from a browser result.
2. Retain tabs and login state across shell jobs, agent turns, and user Web
   disconnect while the environment is live.
3. Reject cross-conversation tab/ref use, including a script that sets
   environment variables naming another conversation. Isolate browser storage. A main-Host shell and an
   attached-Host shell of one conversation use separate environments.
4. Verify target changes, archive, Fork, sleep/reset, disconnect, and crashes.
   Old handles never identify replacement pages.
5. Concurrent calls on one tab report busy; separate tabs progress independently.
   Temporary fetch tabs appear in `tabs` and are cleaned up.
6. Cover multilingual names, containers, frames, local DOM replacement, navigation,
   duplicate matches, obstruction, disabled controls, and Canvas coordinates.
   Fixtures include native date inputs and a confirmation checkbox that must
   be checked again on every submission; the fixture server records the final
   submitted state and acceptance reads it there.
7. Immediate navigation, downloads, choosers, and dialogs cannot lose events,
   repeat side effects, or deadlock on the operation lock.
8. Cancellation releases waits, blocked IO, partial downloads, and input state
   without closing unrelated tabs or claiming rollback of completed effects.
9. Text preserves hierarchy; JSON validates independently. Truncation preserves
   valid JSON. Page text cannot forge terminal controls or tool metadata.
10. PNG stdout follows existing media handling. Mixed streams are not misread as
    images. A printed path never triggers an automatic Host read.
11. Run workflow fixtures through the embedded brush and jq, checking error exits,
    pipefail, empty matches, and business failure separately.
12. Read-only eval rejects DOM/storage/network side effects. Validate CDP scope,
    child targets, cursors, method admission, and cleanup. Verify per platform
    whether the headless clipboard is isolated from the Host user's clipboard
    and that the `clipboard` capability reports the result.
13. Verify the [conversation release](resource-lifecycle.md#acceptance): an
    idle conversation's release on a paired device ends Chrome and its profile
    while the device stays available; a Cloud stop ends it with the machine; a
    running job or waiting child turn defers it; a target change or archive
    releases the old device.
14. Deliver native changes to every required build target, paired device, and
    Cloud guest. Verify Chrome for Testing provisioning on every platform
    offering the feature; success on the development Mac is insufficient.
    Paired-device acceptance runs on both a macOS and a Linux runner. Cloud
    acceptance checks the guest PID 1 runner hash after boot and after wake,
    as [Managed hosts](managed-hosts.md#the-shipped-base) requires.
15. Retirement leaves no Chrome process, helper, or profile directory behind,
    on the development Mac, a Linux paired device, and the Cloud guest.

### Implementation status

The command catalog is the command reference above:

| Family | Commands |
| --- | --- |
| Navigation and ownership | `open`, `tabs`, `info`, `goto`, `back`, `forward`, `reload`, `history`, `close` |
| Observation | `inspect`, `find` including `--query`, `read`, `screenshot`, `probe`, enforced read-only `eval`, `logs` |
| Page input | `click`, `move`, `drag`, `scroll`, `fill`, `type`, `key`, `check`, `select`, `select-text`, `wait` |
| Files and clipboard | `upload`, `download`, `clipboard write/read` |
| Viewport and dialogs | `viewport set/reset`, `dialog inspect/accept/dismiss` |
| Debugging | `cdp targets/send/events/detach` |
| Content and assets | `content read/fetch`, `assets list/export` |
| Capability discovery | `capabilities`, `webmcp list/call` |

The shared browser schema is the argument/result authority for these declarations
and their generated native bindings. Help exposes only declared arguments.

Chrome for Testing delivery is pinned by the release record. An unsupported Host
platform fails explicitly. A six-target runner build does not imply that Chrome
is available on every one of those targets. Platform execution and Cloud-image
acceptance remain release gates, as specified above.

Job conversation identity, service residency, and the conversation release are
part of the command path. A capability the driver, platform, or page
cannot provide is reported unavailable with its reason and never advertised.
The [live browser view](browser-live-view.md) is designed and not implemented.
