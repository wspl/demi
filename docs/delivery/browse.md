# The `bun browse` tool

A product check uses the running product, or the gallery, the way a person
would, and shows the result in screenshots ([AGENTS.md](../../AGENTS.md)). This
document owns the tool that agents use for it: `bun browse`. It runs a
JavaScript check against a browser that stays open, over
[Playwright](https://playwright.dev), in the private package `packages/browse`.

For example, an agent in slot 2 checks that Reload shows the loading state at
once on a far backend, in one call:

```text
$ bun browse <<'JS'
await demi.up('web')
await demi.net.latency(400)
await page.goto('/chat/c-1')
const reload = page.getByRole('button', { name: 'Reload' })
await demi.timeline(() => reload.click(), [0, 100, 1500])
await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible()
JS
Backend   http://127.0.0.1:3320  ready in 71 s
Web       http://127.0.0.1:3323  signed in as developer@example.test
timeline  /…/demi-slots/2/.cache/browse/shots/timeline-0ms.png     painted at -30 ms
timeline  /…/demi-slots/2/.cache/browse/shots/timeline-100ms.png   painted at 85 ms
timeline  /…/demi-slots/2/.cache/browse/shots/timeline-1500ms.png  painted at 1484 ms
Page      http://127.0.0.1:3323/chat/c-1 · "Reload check — Demi"
Focus     button "Stop"
```

## Why this project has its own tool, and why JavaScript

Before the tool, every check started from nothing. In 39 checks, agents wrote
at least 14 browser drivers and about 200 one-off scripts. They lost the most
time to the following problems:

- Getting the product into the state a check needs
- Fixed sleeps
- Elements they could not find
- The app's shared browser pane being hidden or shared
- Servers they could not stop cleanly

One agent's `pkill` stopped the user's own backend.

A first version of the tool gave each step its own command. A batch of fixes
then cost hundreds of model turns, one shell call per click, wait and look.

So a check is a short script, as the leading harnesses do it: Codex gives the
model one JavaScript REPL against a browser tab, and OpenAI's computer-use
guidance prefers such code to lists of actions.

The model already knows Playwright's API from its training, so the tool invents
no language of its own. Finding, acting and waiting are Playwright's. The tool
adds only what this project needs and Playwright does not have, on one object,
`demi`.

## A call

`bun browse` reads a script from standard input, or from a file it names, and
runs it as the body of an async function in which the following names are
defined:

| Name | What it is |
| --- | --- |
| `page` | The slot's browser page, a Playwright `Page`, open between calls. `page.goto` takes an address relative to the slot's web app. Actions wait at most 10 seconds for what they need, navigations 30, so a wrong locator fails quickly |
| `context` | Its `BrowserContext` |
| `expect` | Playwright's `expect`, whose assertions wait until they hold |
| `cdp` | A DevTools protocol session on the page |
| `demi` | The project's helpers ([What `demi` adds](#what-demi-adds)) |
| `keep` | An object that keeps its properties from one call to the next, for values a later call needs |

A call ends by printing what the script printed with `console.log` and the
screenshots it wrote. Then it prints the page's state, so the agent rarely
needs another call only to see where the page stands:

- Its address and title
- A dialog or layer that covers it
- The focused element
- The console errors and failed requests since the call began, without the
  requests that the page itself abandoned

A call has a time limit, ten minutes unless `--limit` says otherwise.

**Failures.** A script that throws stops there. The call prints the
following, in this order:

- The line of the script it stopped at
- What it was doing
- What Playwright looked for and found (its strict mode lists every element a
  locator matched, and an action on an element that something covers names
  what covers it)
- A screenshot
- The page's state

A step's failure is never silent and never a raw stack of the tool's own
frames.

**Waiting.** Playwright's actions and `expect` wait for what they need, so a
script never sleeps for a fixed time. `demi.turn()` waits until the open
conversation no longer runs, its subagents and background jobs included.

## What `demi` adds

`demi` is the object that holds the project's helpers. The following sections
describe them by purpose.

### The slot's servers

`demi.up(...servers)` starts `backend`, `web` and `gallery`, the first two when
none is named, on the slot's ports (slot *n*: backend 33*n*0, web 33*n*1,
gallery 33*n*2). It waits until each answers, and signs the browser in with the
development account, keeping the page it shows when the backend still knows its
session.

`demi.down(...servers)` stops exactly the process groups it recorded, never
anything found by name, so the user's servers and other slots' are never
touched. `demi.down({ wipe: true })` also removes the backend's data and the
paired runner.

- **The backend's data.** It lives in the slot's `.cache/browse/`, through
  `bun xtask dev --data`, so a check can restart the backend as an upgrade does
  and see the page come back to the same account and conversations.
- **Passkeys.** `demi.up(..., { publicHost: 'localhost' })` gives the backend a
  public URL on `localhost`, the one name besides a real domain for which
  browsers make passkeys, so a check can add and use passkeys with Chrome's
  virtual authenticator through the page's CDP session.
- **A production build.** `demi.up('web', { built: true })` serves a production
  `vite build` of the web app instead of the development server, for a check of
  what only a build has, such as the service worker and opening offline.
- **Offline.** `demi.net.offline()` cuts the network of the Chromium context
  only, not that of a `demi.webkit()` page.
- **The `.env`.** `up` copies the user's `.env` into the slot for the backend,
  and `down` deletes it.
  - The `.env` can name the developer's Codex CLI sign-in
    (`DEMI_DEV_CODEX_AUTH`), which the backend's first run in the slot seals as
    a Codex entry, so a check sends its message to a real model the developer
    already pays for. `down({ wipe: true })` lets the next `up` seal a newer
    sign-in.
  - The `.env` names the development model with the file types it reads
    (`DEMI_DEV_PROVIDER_ACCEPTED_EXTENSIONS`), so a check that shows the model
    an image sees it attached. A model whose types are left unknown is shown no
    file, and a check would then report a refusal that says nothing about the
    change it checks.

### Screenshots, timelines and colors

The following helpers show the page:

- `demi.shot(name, { element, region, zoom, page })` writes a PNG of the slot's
  page, of `page` such as one of `demi.webkit()`, or of the page `element` is
  on (`zoom` only on the slot's page). The PNG has the page's real size and
  pixel ratio. The helper takes it after the page's finite animations end, up
  to two seconds, and writes it into `.cache/browse/shots/`. The call returns
  its path, which the agent lists in its report for the lead to send. A shot
  covers only what the viewport shows: an element or a region that reaches
  past it fails, so shoot the parts the viewport shows or set a larger
  viewport with `demi.emulate({ viewport })`.
- `demi.timeline(action, [0, 100, 1500], { region })` runs the action and keeps
  the browser's frames painted nearest to those milliseconds after it, for the
  moments between a click and its result. A full screenshot takes about 100 ms
  and could not show the 100th.
- `demi.pixel(x, y)` reads a color.

### The network

The browser loads the web app through the tool's forwarder on 33*n*3, because
the browser's own emulation can neither slow a WebSocket nor drop one without a
close. The `demi.net` helpers apply to the app's requests and sockets. The
development server's own files stay fast. The helpers are:

- `demi.net.latency(ms)`
- `.bandwidth(kbps)`
- `.offline()`
- `.unreachable()`: the page's traffic goes nowhere while the browser still
  reports a network.
- `.cut(pattern)`: sockets close without a close frame, as a lost connection
  does.
- `.stall(pattern)`: open connections go silent without closing, as one the
  phone's system cut while the page was in the background, while new ones still
  work.
- `.reset()`

The web app's live-update connection bypasses the forwarder, so a restart of
the tool never reloads the page.

### Other helpers

The following helpers cover the remaining needs of a check:

- **Input Playwright cannot give.**
  - `demi.ime(text, { into })` composes text through an input method and
    commits it, as a Chinese or Japanese input method does. The confirming
    Enter comes in the order the page's engine gives it: in Chromium during the
    composition, in WebKit just after it ends. `{ commit: 'escape' }` cancels
    the composition instead.
  - `demi.webkit()` opens a signed-in WebKit page for the call, with the slot's
    screen, pixel ratio, theme, locale and time zone, for checks that Safari's
    order of events matters to.
- **A browser that stops answering.** A browser that stops answering for 5
  seconds ends the call, whatever the script waits on, with "the browser did
  not answer". The tool terminates the browser, and the next call starts a new
  one.
- **Focus rings.**
  - `demi.focusRings({ root, name, shots })` Tabs through the page, then
    focuses what Tab skips. It reports each focus ring an ancestor clips or an
    element above covers, with the side and pixels cut and a picture of each.
  - `demi.focusRings.of(locator)` reads a ring a state draws.
  - A list that keeps the focus and marks a row by `aria-activedescendant` is
    read at that row.
- **Conditions.** `demi.emulate({ viewport, scale, theme, device, locale,
  timeZone })` applies to the page as it is, without reloading it, and returns
  after the page has drawn a frame under them. The theme reaches the gallery's
  own setting too.
  - The conditions are the slot's, kept for its later calls and every page of
    it.
  - The tool is their one owner. A script's `page.setViewportSize` and
    `page.emulateMedia({ colorScheme })`, on any page of the context, are
    `demi.emulate` under Playwright's names, because a second session setting
    them would undo the tool's at the next reload.
  - `demi.grant(permission)` gives a permission such as the clipboard or local
    network access.
- **A hidden page.** `demi.hide({ page })` puts a blank tab in front of the
  slot's page, or of `page`, which then reads `document.visibilityState` as
  `hidden` and gets `visibilitychange`. `demi.show({ page })` brings it back, as
  the call's end does. A tab the script brings to the front hides nothing,
  because the tool emulates focus on every page.
- **Downloads.** A download lands under its own name in the slot's
  `.cache/browse/downloads/`. Playwright's `download` event never comes,
  because the tool attaches to the browser without Playwright's defaults,
  whose focus emulation only Playwright could turn off.
- **The gallery.** `demi.gallery('/session?view=blocks')` opens a specimen page
  of the slot's gallery.
- **Any language.** A helper finds a control by its role and a `data-*` hook
  the web-ui component carries for it (`data-composer`, `data-add-device`,
  `data-pairing-code` and the like), never by its words, so a check runs the
  same with the page in any language.
- **Frames.** `demi.frames(probe)` runs `probe` in the page at every frame the
  page draws and returns what it read each time, for a check that a list never
  shrinks or a row keeps its place across a transition.
- **Watching.** `demi.log.console()`, `.network()` and `.sockets()` return what
  the page logged, requested, and sent or received since the browser started or
  since `demi.log.mark(name)`. They include what goes over a direct channel
  (`crates/runner-direct/src/lib.rs`), which no HTTP request shows: each
  operation as `direct <kind> <status> <path>`, the file watch's messages among
  the sockets'.

### The product's other parts

The following helpers drive the product itself:

- **Messages.** `demi.message(text)` replaces a draft of text and sends the text
  in the open conversation. It waits for its turn to end, not for the subagents
  and background jobs it started. `demi.turn()` after it waits for those too.
  - `demi.message(text, { files })` attaches the files through the composer and
    waits until each shows as its capsule.
  - A draft that holds files makes it fail before it writes, naming them,
    because writing over the draft would delete them.
  - It fails at once, with the page's words, when the page marks the message
    not delivered or the turn ends with an error.
- **Runners.**
  - `demi.runner()` starts the slot's built runner with its own installation
    folder under `.cache/browse/` and pairs it through Add Device, or reuses
    the one it paired.
  - `demi.runner.stop()` and `.start()` stop and start that same runner, as a
    device that goes away and comes back.
  - `demi.runner({ fresh: true })` pairs a new one.
  - `demi.runner({ backend })` and `.start({ backend })` have the runner reach
    the backend at another address, such as a local proxy that stands in for a
    CDN's edge.

## One browser per slot

The first call of a slot starts a browser that lives between calls:
Playwright's Chromium, headless unless `--headed`, with its own profile in the
slot's `.cache/browse/`, so cookies, storage and sign-ins never mix between
slots or with the user's browser.

The browser runs as a process of its own that the tool's server reaches over
the DevTools protocol. So when the tool's own code changes, only the server
restarts, with the agent's current code. The browser, its pages, what was typed
in them, `keep`, the network settings and the logs stay as they were. A call
starts the browser again when it crashed.

The slot comes from the working directory: slot *n* for `../demi-slots/n`, slot
0 for the user's own checkout.

For a person at a shell, `bun browse up`, `bun browse down`, `bun browse stop`
(the browser) and `bun browse help` do what the helpers of the same names do.
Nothing else is a command: a check is a script.

## Whose tool it is

`bun browse` is ordinary code of this repository, and every agent changes it as
part of its own work:

- A check that needs what the tool lacks adds it to `demi`.
- A helper that misleads or breaks is fixed then, so the next agent has it.

What Playwright already does is never wrapped: a helper exists only for what
this project needs and Playwright lacks. The code stays plain so that this
stays cheap: one file per helper under `packages/browse/src/demi/`, sharing the
browser, the slot and the failure reporting.

## What it is not

It is not the product's tests. Automated tests run without a browser of this
kind and never call a real model ([Testing](testing.md)). `bun browse` is for
the checks a person would do by looking, and for reproducing a bug before it
has a test.

It is not the app's shared browser pane either, which the lead keeps for
showing the user something.
