# Conversation browser

Status: design only; not implemented. This document defines intended behavior,
command contracts, and acceptance requirements. Example output is illustrative,
not evidence that these commands exist. Implementation prerequisites are recorded
at the end of this document.

This is the authoritative browser design. Command dispatch, Host admission,
native service ownership, and package boundaries retain their existing owners.
Browser automation does not introduce another shell or agent loop. Shared
startup, idle scheduling, and retirement follow
[Resource lifecycle coordination](resource-lifecycle.md).

## Reading map

- [Purpose](#purpose): agent verification and an interactive streamed workpanel.
- [Ownership](#ownership): conversation binding, tabs, concurrency, and lifetime.
- [Idle reclamation](#idle-reclamation): browser activity and the ten-minute policy.
- [Control](#control-and-interruption): Managed and Free modes, pause, and resume.
- [Execution](#execution): browser distribution, command dispatch, and retained resources.
- [Command contract](#command-contract): inputs, text, JSON, media, and failure.
- [Observation](#observation): page trees, references, locators, and coordinates.
- [Command reference](#command-reference): commands, example output, and result fields.
- [Bash workflows](#bash-workflows): consecutive actions and conditional scripts.
- [Workpanel stream](#workpanel-stream): canonical tabs, frames, input, and reconnects.
- [Acceptance](#acceptance): implementation responsibilities, checks, and prerequisites.

## Purpose

An agent starts an application at `http://localhost:3000` on Cloud and opens
its login page. The new tab immediately appears in that conversation's
workpanel. Chrome for Testing runs on Cloud; the workpanel displays its frames
and sends user input back to that same tab. The agent's `tabs` command lists
exactly the browser tabs available in the workpanel.

```text
Conversation on Demi
  +-- Agent: demi browser commands ----+
  |                                   |
  +-- Workpanel browser tabs           v
        | input                  Host browser ---> Application :3000
        +---------------------------> |
        <--------- frame stream ------+
```

There is one executing page per tab. Agent actions and user actions affect the
same DOM, navigation history, scroll position, storage, and application state.
The design covers this streamed Host browser only. Local page execution and
application proxying are outside its scope.

The user chooses how input is shared:

| Mode | Agent | User |
| --- | --- | --- |
| Managed | Operates the browser exclusively | Watches; can switch to Free mode |
| Free | Can operate until user input pauses its browser actions | Can interact at any time |

Managed mode is the initial state of a newly acquired browser controller. In Free mode,
the first accepted user input pauses agent browser actions before the input is
delivered. The user explicitly resumes agent browser actions after finishing.
Switching modes and resuming are product controls; the agent cannot grant itself
exclusive access or dismiss a user pause. The complete control contract is in
[Control and interruption](#control-and-interruption).

Closing the Demi page ends its stream without aborting the agent. Streaming
latency affects what the user sees and the round trip of user input. Consecutive
agent actions execute on the Host without a client round trip.

This design uses Codex's observable, programmable browser tools and readable
feedback as a capability reference. It does not copy proprietary implementations,
protocols, or prompts, or embed another browser-use agent. Dependencies must not
require AGPL licensing.

## Ownership

### Conversation owns the browser

A conversation owns one Demi-managed browser environment on its current main
Host. The root agent and its children operate tabs in that environment. The
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

Commands obtain the conversation, invoking agent node, and execution Host from
the authenticated execution context. They have no `--conversation`, `--session`,
`--profile`, `--cdp-url`, or browser-process argument. Caller-writable environment
variables cannot establish or replace ownership.

`open` starts the environment when necessary. `tabs` returns an empty list when
the browser has not started; listing alone does not launch it. This does not
change the Host acquisition already performed for the containing shell job.
A command with an expired tab ID fails rather than opening a replacement page.

The environment belongs to the main Host. Browser commands invoked from a shell
on an attached Host return `wrong_host`; they do not silently dispatch elsewhere.
The user changes the browser's execution location through the existing target
selection. Concurrent browser environments on multiple Hosts of one conversation
are outside this design.

Different conversations have isolated cookies, local storage, caches,
clipboards, and tabs. Tabs within a conversation share browser storage, so a
login can apply to more than one tab. This is product isolation, not an
additional security boundary against someone with shell access to the Host.
Cloud conversations of the same user already share files and application services.

### One tab registry

A tab ID is an opaque handle. Examples use `tab-1`; actual IDs are never reused
across browser generations. An old ID must never refer to a newly created tab.
A display index is not an authorization token.

The Host browser driver owns one canonical live tab registry. The backend
publishes its snapshot and ordered changes to the conversation. The workpanel
projects this registry; `demi browser tabs` reads the same registry. All top-level
pages appear, including tabs opened by the user, agents, sites, and temporary
content operations. There are no hidden automation-only tabs. File and change
workpanel tabs are other resource types and do not appear in `browser tabs`.

The tab ID is also the browser workpanel item's resource ID. The web application
may retain local selection and panel visibility; it must not maintain another
browser tab inventory or use local workpanel IDs as browser handles. Reloading
Demi obtains the current registry snapshot. Selecting a tab affects the viewer's
subscription only; agent commands always name a tab explicitly.

Root and child agents share the registry. Creation metadata is a discriminated
value: `agent` with node ID, `user`, `page` with opener tab ID, or `temporary`
with invoking node ID. This is diagnostic metadata, not authorization. An iframe
remains part of its top-level tab. A site-created top-level tab is registered
before it can be operated; the triggering action reports observed `openedTabs`.
Later popups appear through registry updates and subsequent `tabs` calls.

Only one agent command executes against a tab at a time. Its operation lock
covers targeting, input, associated waits, and result collection. Conflicting
agent calls return `tab_busy`, without queuing a click for an unknown later page.
Different tabs may progress in parallel. Frame capture does not take this lock.
User input in Free mode follows the preemption rule below, not `tab_busy`.

A shell script is not a transaction. Another agent can act between commands.
Agents needing independent sequences should use separate tabs; shared browser
storage still means those tabs can affect the same login.

### Control and interruption

Control applies to the entire conversation browser, including root and child
agents and every browser tab. A user typing into tab A must not leave an agent
submitting a related form in tab B. The Host browser controller serializes mode
changes, input admission, and action dispatch. It stores one state plus a
monotonic control revision; displayed mode and paused status derive from it.

| State | Agent page-changing commands | User page input |
| --- | --- | --- |
| `managed` | Allowed | Rejected with `managed_mode` |
| `free` | Allowed | Atomically changes state to `free_paused`, then delivers input |
| `free_paused` | Rejected with `browser_paused` | Allowed |

The user can select Free from Managed, select Managed from either Free state,
or resume from `free_paused` to `free`. Each transition advances the revision.
Selecting Managed is an explicit user decision to return exclusive operation
to agents. Merely waiting, disconnecting, reconnecting, or receiving another
agent call never resumes browser actions. State lasts for the live controller;
a replacement resource starts Managed and is shown as a new environment.
The user can change mode before opening the first tab; lazy browser startup
preserves that choice.

Page-changing operations include open/close, navigation, pointer and keyboard
input, form actions, dialog replies, upload/download triggers, clipboard writes,
viewport changes, content fetch, WebMCP calls, and CDP send. Classify CDP send
conservatively even for a nominally observational method. Inspect/find/read,
screenshots/probe, history/info/tabs, logs/events, read-only eval, waits,
capabilities, CDP target discovery, WebMCP discovery, dialog inspection, clipboard
reads, content read/export and asset extraction may observe while paused. Export adapters must
be observational to qualify. Observation can race user changes and cannot claim
a frozen page. Writing an observation to a Host file does not operate the page.

When input arrives in Free mode:

1. Validate the viewer, resource generation, relevant tab/viewport, and input.
   Selecting a workpanel tab or subscribing is not page input. Pointer movement,
   wheel, keys, composition, and browser toolbar actions are input.
2. Advance the control revision and enter `free_paused` before dispatching input.
   Stop new agent page changes on every tab; cancel current agent browser calls
   and their waits. Release held agent keys/buttons and blocking debug sessions.
3. Fence all previous agent dispatches and await their bounded cancellation
   acknowledgement. Then deliver the user's input. If stopping fails, report
   `browser_unavailable` and retire the faulty environment; never run both input
   owners concurrently. A frame may show effects already completed by an agent.
4. Notify all workpanel viewers and affected agent nodes. The interrupted command
   fails with `user_interrupted` and known action progress. The notification
   tells the agent to wait for explicit resume before new page changes.

A shell invocation receives its browser control revision from trusted execution
context, even when its first browser command is later in the script. Every
page-changing command verifies that revision at admission and immediately before
input delivery. Old shell jobs cannot acquire a new revision by retrying. After
resume, an old script still fails `stale_control`; the agent must start a new
shell call and observe the current page before continuing. State-change notices
use existing per-node execution-context updates and steering/yield wakeup machinery. Resume informs affected nodes without
replaying a tool call or creating a second agent scheduler. Cancellation or a
mode transition invalidates outstanding dispatch tokens. It does not undo a
submitted form, undo page JavaScript, or stop arbitrary non-browser shell work.

Mode transitions also cancel/fence in-flight browser calls before acknowledgement,
so switching to Managed cannot leave a user key held down. Cancelling an agent
turn releases input and blocking debug facilities owned by that turn. Browser
state and nonblocking observations such as buffered logs remain available.
Commands rejected before dispatch use `details.action: not_started`; completed
or uncertain effects keep their actual progress. No automatic replay occurs.

Control is a Demi input arbitration rule. Browser page scripts continue running,
and someone with unrestricted shell access could operate a different browser.
It is not an operating-system security boundary against the conversation owner.

### Lifetime

The conversation's retained native resource owns the browser controller. A
browser-capable job binds that resource and obtains its control revision before
shell execution; binding alone does not launch the browser. User opening uses
the same resource. The browser process and its storage form the lazily created
environment.

The environment outlives individual commands, shell jobs, agent turns, and
viewing connections. Its internal lifecycle uses one state:
`absent | starting | ready | closing`. It does not also store contradictory
`running` and `finished` flags.

| Event | Required behavior |
| --- | --- |
| First `open` | Join concurrent environment startup; create a distinct tab for each successful caller |
| Command, shell job, or agent turn completes | Release invocation resources; retain the environment and tabs |
| User closes the workpanel or Demi page | Remove that viewer and release its held input; stop capture after the last viewer leaves; retain control state and continue permitted agent work |
| Command or agent turn is cancelled | Stop current invocation work and temporary waits; retain completed page effects and other tabs |
| `close <tab>` | Close that tab and release its commands, references, and debugging state |
| Last tab closes | Retire the browser resource after the closing invocation completes; stop Chrome for Testing, remove its profile and release its grant; the next `open` starts fresh |
| Browser idle deadline expires | Retire through the shared coordinator under the policy below |
| Main Host or main directory changes | Release the old environment through the target transition; do not copy tabs, cookies, or debugging connections |
| Conversation is archived | End the environment and all viewing/input access; restoring the conversation starts fresh on demand |
| Conversation is forked | Do not inherit the live browser or handles; IDs in copied history are historical text |
| Chrome for Testing crashes, runner connection ends, backend restarts, or Cloud stops/resets | Invalidate the environment and fail affected calls; never replay page actions |

The native controller reports last-tab closure and fences further page actions.
The backend coordinator releases its retained grant after the closing invocation
has delivered its result and released its operation lease; cleanup must not wait
for the invocation that requested it while that invocation waits for cleanup.
Closing the last tab invalidates old action revisions, including later commands
in the same shell call. Opening again requires a new call and a new controller.

A controller bound before the first open can have no tabs or browser process.
It follows the same idle policy, preserving an explicit user mode selection
before the first tab opens. First creation from an empty controller does not
change the revision captured when its job bound the resource.

Browser profiles and live page state are not restored across process lifetime
boundaries. Explicit output files survive according to their Host location.
Screenshots already persisted into the transcript follow the existing media
storage contract.

Target changes, archiving, and observer cancellation follow
[Host operations](sessions-and-targets.md#host-operations). Resource retirement
uses the shared coordinator; cleanup must not create another path to the Host.

### Idle reclamation

The browser idle duration is **10 minutes**, independently configured from Cloud's
idle duration. This policy applies on both paired devices and Cloud. Start the
interval only while a live browser resource has all of these properties:

- Its entire owning conversation's agent tree is idle, including children and
  work already admitted by tree lifecycle coordination.
- No browser command, user input, upload/download operation, or browser startup
  is in progress. Observational commands also count as use.
- No viewer has an active browser frame subscription.

New activity cancels the interval. When all conditions hold again, begin a fresh
full interval. Tabs, cookies, retained grants, and backend metadata subscriptions
are retained state, not activity. Merely showing a conversation in the sidebar
or keeping its chat connected does not prevent browser retirement. A waiting
agent turn does prevent it; a future scheduled turn that has not been admitted
does not. The same policy releases a controller that has remained empty since
binding; explicit last-tab closure still retires immediately as specified above.

At expiry, the shared coordinator reserves admission and rechecks eligibility
before releasing the native browser resource. The grant, process, temporary
profile, tab/reference handles, input state, and capture resources end together.
The backend retains the conversation and its historical outputs, not a live
browser controller. Notify product observers that the browser was reclaimed for
inactivity; it is not an unexplained stream failure. The next user/agent open
creates a fresh controller in Managed mode with new tabs and browser storage.

[Resource lifecycle coordination](resource-lifecycle.md) owns timer cancellation,
new-demand races, shared startup, failure handling, and parent cleanup ordering.
Browser activity participates in Host admission through the existing operation
or viewing lease. Retained idle browser state does not keep Cloud awake.
Cloud's own retirement can end it earlier under the
[Cloud policy](managed-hosts.md#lifecycle-and-capacity). Browser retirement does
not power down a paired device or independently decide to stop Cloud.

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
Testing supplies the executable; the separately selected native driver uses CDP.
Choosing this distribution does not require ChromeDriver or select a driver
library.

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

### Retained resource ownership

A native service ordinarily survives only while something owns it. Browser
continuity between shell jobs therefore requires a conversation-owned retained
resource, not a sleeping fake command or an unmanaged detached child process.
The generic extension is defined in
[Native runtime](native-runtime.md#retained-resources).

For a browser resource:

1. The backend obtains an opaque resource grant through conversation Host access.
   It binds the current main Host, runner connection generation, and exact native
   artifact selected by the runtime catalog.
2. Runner maps authenticated jobs to that grant and supplies trusted resource
   scope to native invocations. CLI arguments and forwarded environment values
   cannot choose the grant.
3. Later calls in the conversation reuse the environment. A shared native
   service still separates environments by grant.
4. Releasing the owner closes the browser, removes its temporary profile, and
   releases the retained service reference. Cancelling one invocation does not
   release that owner.

A retained grant is resource ownership, not an active Host operation. It does
not permanently hold the conversation file gate or Cloud activity lease.

Backend operations for viewing, input, and browser resources enter through
`ConversationTargets.withHost`. Its scope covers actual Host IO. Keeping an
old Host object is not permission to bypass this entry on later requests.
Cloud and paired devices use the same contract.

## Command contract

### Inputs

The root is `demi browser`. There are no agent commands to manage browser
sessions, processes, stream subscriptions, or control-mode changes.

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
an `--expression` option. Help is resolved before any stdin read.

Validate CLI input, JSON bodies, Host files, CDP replies and events, and
page-provided tool schemas/results at entry. Types derive from schemas; casts
and silent repairs do not validate data. Browser arguments, results and event payloads have one schema authority in the
planned `browser-protocol` package. Command declarations consume those schemas;
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
| Ordinary action/read timeout | 10 seconds |
| Navigation, wait, download, and export timeout | 30 seconds |
| Explicit `--timeout` | Milliseconds, 1–120000; no indefinite waits |
| Inline inspect/find/content text | 64 KiB, with explicit truncation |
| List `--limit` | Default 100, maximum 1000 |
| One `content fetch` | At most 10 URLs |
| CDP event buffer per tab | At most 10000 events and 8 MiB, whichever comes first |
| Console buffer per tab | At most 1000 entries and 1 MiB |
| Finite text/JSON stdin | At most 1 MiB; binary clipboard input is not text |

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

Name and text matching is case-sensitive substring matching by default;
`--exact` selects whole-string matching. Explicit `--name-pattern` and
`--text-pattern` accept regular expressions instead of their literal fields.
Do not interpret slash-delimited text as a regex automatically. `--text-match`
locates an element; `--text` is action payload for fill/type/select-text. These
fields must never share a flag or be inferred from the presence of another flag.

`--within <ref>` restricts a container. Repeated `--frame <ref>` enters frames
from outermost to innermost. `--nth <n>` selects a zero-based match. All scope
references must belong to the target tab.

`find` can return multiple matches. `read --all` explicitly reads multiple
matches. A mutation requires exactly one element unless the command inherently
sets several values on one element. Ambiguity fails; there is no implicit first
match.

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

Before clicking, wait until the element is visible, enabled, and can receive
input without obstruction. Before filling, also require editability. An absent
or not-yet-ready semantic target waits until the deadline; an explicitly stale
reference or ambiguous match fails immediately. There is no implicit force or
silent fallback from a failed element action to a coordinate action.

Coordinates are CSS pixels in the current viewport, with origin at its top left.
Screenshots report both image pixels and viewport CSS dimensions so callers can
account for device scale. `probe` uses the same CSS coordinates. Off-viewport
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
tab-2   Admin     user         http://localhost:3000/admin

$ demi browser info tab-1
Tab: tab-1 · Sign in
URL: http://localhost:3000/login
Viewport: 1280 × 720 CSS px
Dialog: none
Control: Managed (revision 4)

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
when no history entry exists. History is local to the named tab.

If `open` creates a tab but navigation times out, the error retains `details.tab`
so the agent can inspect it. Navigation failures report the current URL and
known action progress rather than implying nothing happened.

`info` includes conversation browser control state and revision.
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
`--property text|text-content|value|visible|enabled|checked` or
`--attribute <name>`. `text` is rendered text; `text-content` is raw text content.
A missing attribute returns null. Reading a protected input's value fails with
`protected_value`. Multiple matches require `--all` or an explicit `--nth`.

### Screenshot and probe

```text
$ demi browser screenshot tab-1
<PNG bytes; the shell media adapter displays an image>

$ demi browser screenshot tab-1 --output /tmp/login.png
Screenshot saved: /tmp/login.png
Image: 1280 × 720 px
Viewport: 1280 × 720 CSS px

$ demi browser screenshot tab-1 --full-page --output /tmp/page.png
Screenshot saved: /tmp/page.png
Image: 1280 × 2400 px
Viewport: 1280 × 720 CSS px

$ demi browser screenshot tab-1 --clip 100,200,600,400 --output /tmp/region.png
Screenshot saved: /tmp/region.png
Image: 600 × 400 px

$ demi browser probe tab-1 --xy 420,300
[ref=e3] button "Sign in"
Bounds: x=360 y=280 width=120 height=40 CSS px

$ demi browser probe tab-1 --xy 420,300 --output /tmp/annotated.png
[ref=e3] button "Sign in"
Annotated screenshot saved: /tmp/annotated.png
```

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

Scroll accepts `--dx` and `--dy`, with at least one nonzero value. An element or
coordinate can select the scroll origin. Delivered input does not prove the
page moved: it may already be at a boundary. Observe again when that matters.

### Input and forms

```text
$ demi browser fill tab-1 --ref e1 --text test@example.com
Filled textbox "Email" [ref=e1].

$ demi browser type tab-1 --text hello
Typed into the focused element.

$ demi browser type tab-1 --ref e1 --text '.test'
Typed into textbox "Email" [ref=e1].

$ demi browser key tab-1 Enter
Pressed Enter.

$ demi browser key tab-1 --ref e1 ControlOrMeta+A
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

$ demi browser ax-action tab-1 --ref e8 --action expand
Performed accessibility action "expand" on [ref=e8].
```

Fill replaces the value. Type inserts at the cursor without clearing and emits
sequential character input. Key combinations are validated; an unknown key name
is not treated as literal text. Check sets the requested state; an already
matching state succeeds without toggling it.

Select accepts one of repeated `--value`, `--option-label`, or `--option-index`,
not a mixture. `--label` still locates the select element itself.
Multiple values for a single-select control are invalid. Select-text defaults
to selecting the match; `--cursor before|after` positions a cursor instead.
`--prefix` and `--suffix` disambiguate repeated text; remaining ambiguity fails.

AX actions must be advertised by the observed element. They are not arbitrary
JavaScript. An unsupported action returns `unsupported_capability`.

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
`--state attached|detached|visible|hidden`, `--url`, or `--load`.
URL patterns define `*` as any string without `/`, `**` as any string, and all
other characters literally. They are not implicit regular expressions.

Hidden includes removal within the same document; detached waits for removal.
If navigation replaces a referenced document, return `stale_ref` rather than
treating disappearance of the old page as a condition on the new page.

An action's `--wait-url` installs observation before delivering input and waits
for a matching navigation after that action starts. An already matching URL
before the action does not satisfy it. Key supports the same option for Enter
submission. There is no generic `--page-changed`: an animation or clock update
cannot establish that pagination finished.

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
Viewport: 390 × 844 CSS px.

$ demi browser viewport reset tab-1
Viewport reset to default: 1280 × 720 CSS px.
```

Eval is read-only page inspection. The stdin expression can access document;
a unique target additionally exposes element, or explicit `--all` exposes
elements. Results must be JSON-representable. Functions, DOM objects, cycles,
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

Viewport overrides apply to the named tab only. Initial defaults are 1280 × 720
CSS pixels and device scale 1. Reset removes the override. This does not emulate
complete mobile hardware, touch, user agent, or network conditions. Screenshots
must not silently change viewport settings.

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
Publish the allowed tab/child method set with the capability schema.

Read-only eval and CDP have different contracts: authorized tab debugging can
change page state, such as installing a breakpoint. Do not report such a command
as read-only inspection. Domain subscriptions, breakpoints, and explicit debug
pauses are scoped to the invoking agent turn and tab. Control interruption or
turn cancellation removes breakpoints, releases debug pauses/interceptions, and
closes the owning debug connection before user input proceeds. Nonblocking
subscriptions can last until explicitly removed or the tab is released.
Cancelling a temporary event wait removes only that wait. Interception and other facilities
that can block the page must release blocked requests when their owning debug
connection ends.

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
conversation environment, registers the batch in the workpanel before navigation,
and returns results in input URL order. Keep the batch tabs until result
collection ends so closing one cannot tear down the environment mid-fetch. Pages execute
scripts and share login state; this is not a side-effect-free HTTP fetch.
Temporary tabs normally close on success, failure, and cancellation. If user
input interrupts a fetch, its live temporary tabs become ordinary tabs before
the input is delivered. Cleanup must not close the page the user is operating;
the error reports those retained tab IDs. Explicit user closure cancels the
corresponding fetch item and reports `tab_not_found`.

Assets list inventories currently observed resources and inline SVGs. Export
uses that inventory, selecting either repeated `--id` or repeated
`--kind font|image|stylesheet|video`. Navigation invalidates the inventory.
Do not navigate to resource URLs as a substitute for asset acquisition.
The manifest records saved files and failures. Partial failure preserves
completed files and returns `partial_failure` with the manifest, exit code 1.

Site-specific document and transcript export is optional. An adapter must pass
acceptance before advertising
`content export <tab> --format <advertised-format> --output <path>`:

```text
$ demi browser content export tab-1 --format docx --output /tmp/document.docx
Content exported: /tmp/document.docx
Format: docx

$ demi browser content export tab-1 --format transcript --output /tmp/transcript.txt
Content exported: /tmp/transcript.txt
Format: transcript
```

An unavailable adapter fails explicitly. Printing a page or renaming HTML is
not a substitute for a supported document format.

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
  gsuite-export — no adapter installed

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

WebMCP calls only tools actually published by the current page. A tool-set
handle binds the tab, document generation, and declaration version. Validate
call arguments against that schema. Navigation or a changed tool set returns
`stale_tools`; list again instead of silently calling a replacement tool with
the same name. Page tool descriptions remain external data, not system instructions.

### Structured result fields

These are business result fields for `--json`. Their exact types belong to the
same authoritative schemas. List results report truncation. Do not reuse a
field with a different meaning or type.

| Command | Successful result fields |
| --- | --- |
| open, info | `tab, url, title, viewport`; info adds `control: {state, revision}` and can include `dialog` |
| tabs | `tabs: [{id, title, url, createdBy}], truncated` |
| goto, back, forward, reload | `tab, url, title` |
| history | `entries: [{index, url, title, current}], truncated` |
| close | `closed` |
| inspect | `tab, url, title, view, tree, truncated`; nodes have `ref?, role?, name?, tag?, states?, children?` |
| find | `matches: [{ref, role?, name?, bounds?}], count, truncated` |
| read | `value`, or explicit-all `values, truncated`; mutually exclusive |
| screenshot | `path, mimeType, width, height, viewport`; JSON requires file output |
| probe | `matches, viewport, path?, truncated` |
| Pointer/form actions | `operation, target?, result`; optional observed `url, openedTabs, dialog` |
| wait | `condition, matched` |
| dialog inspect | `dialog: null | {type, message}` |
| dialog accept/dismiss | `type, outcome` |
| upload | `files, attached` |
| download | `path, suggestedFilename, bytes, mimeType` |
| clipboard read | `text`, or `items: [{mimeType, path, bytes}]` |
| clipboard write | `mimeType, bytes` |
| eval | `value` |
| logs | `entries, cursor, hasMore, truncated` |
| viewport set/reset | `width, height` |
| cdp targets | `targets: [{id, kind, url}], truncated` |
| cdp send | `method, result` |
| cdp events | `events, cursor, hasMore, truncated` |
| content read | `url, title, format, content, truncated`, or `url, title, format, path` |
| content fetch | `pages: [{requestedUrl, url, title, content, error?}], truncated` |
| content export | `path, format, mimeType` |
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
Current URL: http://localhost:3000/login
Action: completed.
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

After user input interrupts a Free-mode workflow, the browser remains available
for observation, while a new page-changing call reports the pause:

```text
$ demi browser info tab-1
Tab: tab-1 · Sign in
URL: http://localhost:3000/login
Viewport: 1280 × 720 CSS px
Dialog: none
Control: Free — agent actions paused by user input (revision 5)

$ demi browser click tab-1 --ref e3
Error: browser_paused
The user is operating this conversation's browser. Wait for the user to resume agent actions.
Action: not_started.
```

An already-running call instead returns `user_interrupted`. If both conditions
apply to a subsequent old-script command, report `browser_paused` while paused
and `stale_control` after resume. Neither result authorizes an automatic retry.

Messages explain the situation; scripts inspect code and typed details:

| Code | Meaning |
| --- | --- |
| `invalid_input` | Arguments or finite input violate the contract |
| `wrong_host` | Invocation is not on the current main Host |
| `tab_not_found` | Missing, closed, or inaccessible tab |
| `tab_busy` | Another agent command owns the tab operation lock |
| `browser_paused` | User input paused agent page changes; explicit user resume is required |
| `user_interrupted` | User input cancelled this browser call; action progress is retained |
| `stale_control` | This shell call belongs to an earlier control revision; never replay its actions |
| `stale_ref`, `stale_cursor`, `stale_inventory`, `stale_tools` | Expired object or generation |
| `target_not_found`, `ambiguous_target`, `not_actionable` | Missing, non-unique, or unready target |
| `timeout` | Deadline expired; details report known action progress |
| `dialog_blocked`, `dialog_not_found`, `invalid_dialog_action` | Blocking or inapplicable dialog state |
| `history_boundary` | No previous or next navigation entry |
| `protected_value`, `side_effect_rejected`, `unsupported_result` | Disallowed inspection, side effect, or unrepresentable value |
| `unsupported_capability`, `cdp_method_denied` | Unsupported or unavailable operation in this scope |
| `output_exists`, `io_error`, `result_too_large` | File or result-size failure |
| `partial_failure` | Some batch items failed; details retain item results or manifest |
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

## Workpanel stream

### Product and Host boundary

The workpanel contains a browser resource type alongside files and changes.
Browser items project the canonical tab registry described in [Ownership](#one-tab-registry).
User open, close, back, forward, reload, and address navigation invoke the same
Host browser operations and use the same IDs as agent commands. In Managed
mode those user operations are disabled and rejected server-side. Switching mode,
selecting a tab, hiding the panel, and disconnecting remain available.

Demi authenticates the user and conversation on every subscription and control
request. Backend adapters enter through `ConversationTargets.withHost`; runner
carries authenticated resource scope to the native controller. The client gets
neither a raw CDP socket nor credentials to another conversation. The browser
renders and executes page scripts on the Host. The Demi page decodes frames and
captures input; it does not execute the remote page's HTML or JavaScript.

The Host controller is authoritative for registry and live control state.
Grant-scoped registry/control notifications are emitted over the existing runner
connection. The backend maintains their reconstructible product projection;
watching that projection does not open a long-lived Host operation or hold a use
lease. Snapshot repair is a short `withHost` operation. After resource loss or
retirement, publish the ended generation without waking a replacement browser.
Frame subscriptions are distinct from these metadata notifications.
Snapshots include browser generation and event revision. Registry/control
changes form an ordered stream; a gap requires a fresh snapshot. Stale updates
cannot resurrect a closed tab or overwrite a newer mode. The backend forwards
validated state and correlates requests without maintaining an independent tab
or control-state authority. Native control transitions must be acknowledged
before the product reports success.

### Frames and flow control

A selected tab has a frame subscription. Each frame carries browser generation,
tab ID, increasing frame sequence, capture time, viewport revision, CSS viewport
dimensions, encoded image dimensions, and encoding. Readiness, tab closure,
environment loss, and errors are explicit messages. Initial compressed image
frames are sufficient; the transport choice must preserve this contract.

Capture once per watched tab and fan out. Each viewer has at most one pending
frame, replaced by a newer frame when it falls behind. Viewing must not block
agent commands or input. Stop capture and release buffers when the last viewer
leaves. Reconnection displays current state, not a frame backlog. The last image
may remain only with a disconnected indication; it cannot accept input.

Hiding or unmounting the browser surface unsubscribes from its frame stream.
Selecting another tab unsubscribes from the old frame stream and releases that
viewer's held input before subscribing to the new one. Tab closure releases its
capture and input resources and removes it from all registry projections.
Frontend unmount, WebSocket loss, authentication loss, and subscription failure
share cleanup. Browser generation loss invalidates subscriptions instead of
silently redirecting them to a replacement page. Streaming is not recorded to
disk by default; explicit screenshots have their own output contract.

### Input protocol and arbitration

A viewer connection has an authenticated identity and monotonically increasing
input sequence. Page-input events supply generation, tab, viewport revision,
observed frame sequence, current control revision, and their typed payload.
Browser toolbar requests supply resource generation and control revision; only
operations on an existing tab carry a tab ID. In particular, opening the first
tab needs no old tab or frame. Mode requests address the controller directly.
Duplicate or out-of-order sequences are rejected, not replayed. An acknowledgement reports
the input sequence, resulting control revision, and `delivered | rejected`;
connection loss before acknowledgement leaves delivery unknown.

Pointer coordinates map the displayed image rectangle to viewport CSS pixels,
excluding letterboxing. Bounds, finite coordinates, button/key enums, modifier
sets, wheel units, and payload lengths are schema-validated. A stale viewport
revision returns `stale_viewport`; wait for a matching frame instead of guessing
a new coordinate. Old frames within the same viewport can still show stale
content, so delivery acknowledgement is not proof of an application outcome.

The input protocol covers pointer move/down/up, wheel, key down/up, composition
start/cancel, and committed IME text. Composition stays local until commitment;
deliver its text once and suppress duplicate key/text events. Tab, Escape, navigation shortcuts, and
clipboard operations must have explicit routing in the shared browser component.
Do not capture global user shortcuts outside the focused browser surface. The
browser clipboard belongs to the Host environment; transferring local clipboard
text requires an explicit user copy/paste action and the local browser's APIs.
Native file pickers on the Host cannot appear in the user's local browser:
workpanel file selection uploads through conversation Host access, then supplies
those Host paths to the existing chooser operation. User downloads stream the
completed Host file through the same authorized file access. JavaScript dialogs
are exposed as structured state and use the same dialog operations as commands.
All these page-affecting actions obey Managed/Free arbitration.

In Free mode, input first executes the interruption transition described above.
After the pause is acknowledged, queued events from that viewer carry the new
revision. Keep at most 256 events and 256 KiB per viewer pending acknowledgement;
coalesce unsent pointer moves and wheel deltas only when no intervening button,
key, or composition event changes their meaning. On overflow, stop input, release
held keys/buttons, and report `input_overflow`; never silently drop a key-up.

Multiple windows can watch concurrently. Within Free mode only one viewer can
hold a pointer gesture or key/composition sequence at a time. The first accepted
down/composition-start acquires this short input lease; competing viewers receive
`input_busy`. Release after all held input ends, focus loss, mode change,
disconnect, or a 5-second interval without an owner event. Renew using owner
input/heartbeat only. Timeout sends release events and drops that viewer's queued
input. Stateless toolbar and wheel actions are serialized by the same controller.
Any viewer can request a mode change; its expected control revision must match
or it receives a conflict and current state. A disconnect releases input, but
leaves `free_paused` paused. There is no unattended automatic resume.

The workpanel exposes current mode, paused state, connection status, and failed
input without implying that a click succeeded in the application. Visual design,
keyboard affordances, and interaction specimens live in the gallery. Implement
shared behavior once in `web-ui`, with product adapters and gallery fixtures in
the same checkpoint, following [Web architecture](web-application.md).

Observer cancellation and target/archive transitions follow
[Host operations](sessions-and-targets.md#host-operations). A stream holds activity
only for its admitted lifetime; an idle retained browser resource holds no
permanent Cloud activity lease. Idle eligibility follows
[Idle reclamation](#idle-reclamation).

## Acceptance

### Implementation ownership

[Package boundaries](../package-boundaries.md) remains authoritative:

- `browser-protocol`: shared browser schemas and derived types.
- `coding-agent`: declared commands using those schemas and generated help;
  assemble available commands through injected capabilities.
- `backend/conversation`: browser owner, main-Host binding, admission, and product
  adapters.
- `runner` and `host-remote`: generic retained resources, trusted invocation
  scope, cancellation, and transport; no webpage algorithms.
- `demi-commands`: driver, page observation, actions, output rendering, assets,
  and CDP handling.
- `command-service`: generic invocation/resource protocol, not page or cookie
  semantics.
- `web-ui`: shared browser tab, stream, and input behavior; `web` and
  `web-gallery` provide real adapters and fixtures respectively.

A library adopted for locating or acting must also cover the adjacent waiting,
introspection, and error handling it provides. Reuse one observation/targeting
implementation across commands rather than parallel AX, DOM, and coordinate
business state. Command naming need not mirror every method in a library API.

### Required checks

Use real Chrome for Testing against local page fixtures, scripted providers,
and isolated storage. Never run tests that call real models.

1. Run `open → inspect → fill → click → inspect` on paired devices and Cloud;
   verify localhost, files, and screenshots belong to the correct Host.
2. Retain tabs and login state across shell jobs, agent turns, and user Web
   disconnect while the environment is live. Stop capture without viewers.
3. Reject cross-conversation tab/ref/grant use. Isolate browser storage. Return
   wrong_host for browser calls from an attached Host.
4. Verify target changes, archive, Fork, sleep/reset, disconnect, and crashes.
   Old handles never identify replacement pages.
5. Concurrent calls on one tab report busy; separate tabs progress independently.
   Temporary fetch tabs are visible and cleaned up; user-interrupted pages are retained.
6. Cover multilingual names, containers, frames, local DOM replacement, navigation,
   duplicate matches, obstruction, disabled controls, and Canvas coordinates.
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
    child targets, cursors, method admission, and cleanup.
13. Slow viewers neither block actions nor build queues. Passive streams can be
    cancelled for transitions. Verify shared components in product and gallery.
14. Verify Managed rejects user input; Free user input pauses all agent tabs,
    interrupts waits, releases held input/debug blockers, and fences old scripts
    through resume. Cover IME, clipboard, chooser/download, scaling, slow viewers,
    multi-window input, stale revisions, disconnect, and input overflow. Verify
    user-, agent-, site-, and fetch-created tabs match the workpanel registry.
15. Verify the browser idle policy and cross-resource races in
    [Lifecycle acceptance](resource-lifecycle.md#integration-and-acceptance):
    viewers and active children prevent idle cleanup; metadata-only watchers do
    not; cleanup on paired and Cloud Hosts releases the native grant and profile.
16. Deliver native changes to every required build target, paired device, and
    Cloud guest. Verify Chrome for Testing provisioning on every platform
    offering the feature; success on the development Mac is insufficient.

### Deferred decisions and implementation status

This checkpoint delivers design only. Browser commands, browser management,
streaming, input arbitration, idle reclamation, and retained-resource support
are not implemented.
The checks above are acceptance requirements, not completed results.

Resolve these prerequisites before implementing their dependent behavior:

| Decision | Required outcome | Blocks |
| --- | --- | --- |
| Driver | Select a non-AGPL implementation for CDP, semantic targeting, actionability, and enforced read-only evaluation; establish platform support | Native browser commands |
| Browser delivery | Chrome for Testing is selected; choose the first version pin and implement installation, verification, updates, and reclamation under the distribution contract above | Runtime delivery |
| Lifecycle integration | Implement the shared coordinator and its Cloud/browser adapters under the lifecycle contract | Idle and dependent-resource reclamation |
| Retained-resource wire | Add trusted grant acquisition, job association, release, and cleanup acknowledgement to the native/runner schemas | Continuity across shell jobs |
| Stream transport | Select frame encoding/carriage and specify the validated registry, control, frame, input and acknowledgement wire records with the limits above | Interactive workpanel |
| Optional adapters | Select and validate any page tools or site-specific exports included in the release | Corresponding capabilities |

These are implementation prerequisites, not runtime choices for the agent.
Features may land in separate checkpoints, but unavailable capabilities must
not be advertised. Using Codex as a capability reference is not a guarantee
of parity with every Codex product feature, such as personal browser management
or locally executing browser tabs.
