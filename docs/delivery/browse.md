# bun browse

A product check uses the running product, or the gallery, the way a person
would, and shows the result in screenshots ([AGENTS.md](../../AGENTS.md)). This
document owns the tool agents do it with: `bun browse`, which runs a
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

## Why a tool of our own, and why JavaScript

Before it, every check started from nothing. In 39 checks, agents wrote at least
14 browser drivers and about 200 one-off scripts, and lost the most time to
getting the product into the state a check needs, fixed sleeps, elements they
could not find, the app's shared browser pane being hidden or shared, and
servers they could not stop cleanly; one agent's `pkill` stopped the user's own
backend. A first version gave each step its own command, and a batch of fixes
then cost hundreds of model turns, one shell call per click, wait and look.

So a check is a short script, as the leading harnesses do it: Codex gives the
model one JavaScript REPL against a browser tab, and OpenAI's computer-use
guidance prefers such code to lists of actions. The model already knows
Playwright's API from its training, so the tool invents no language of its
own: finding, acting and waiting are Playwright's, and the tool adds only what
this project needs and Playwright does not have, on one object, `demi`.

## A call

`bun browse` reads a script from standard input, or from a file it names, and
runs it as the body of an async function in which these are defined:

| Name | What it is |
| --- | --- |
| `page` | The slot's browser page, a Playwright `Page`, open between calls; `page.goto` takes an address relative to the slot's web app, and actions wait at most 10 seconds for what they need, navigations 30, so a wrong locator fails quickly |
| `context` | Its `BrowserContext` |
| `expect` | Playwright's `expect`, whose assertions wait until they hold |
| `cdp` | A DevTools protocol session on the page |
| `demi` | The project's helpers, below |
| `keep` | An object that keeps its properties from one call to the next, for values a later call needs |

A call ends by printing what the script printed with `console.log` and the
screenshots it wrote, then the page's state, so the agent rarely needs another call only to see where the page
stands: its address and title, a dialog or layer that covers it, the focused
element, and the console errors and failed requests since the call began;
requests the page itself abandoned are left out. A call has a time limit, ten minutes unless
`--limit` says otherwise.

**Failures.** A script that throws stops there. The call prints the line of the
script it stopped at, what it was doing, what Playwright looked for and found
(its strict mode lists every element a locator matched, and an action on an
element something covers names what covers it), and a screenshot, then the
page's state. A step's failure is never silent and never a raw stack of the
tool's own frames.

**Waiting.** Playwright's actions and `expect` wait for what they need, so a
script never sleeps for a fixed time; `demi.turn()` waits for the open
conversation's turn to end.

## What `demi` adds

- **The slot's servers.** `demi.up(...servers)` starts `backend`, `web` and
  `gallery`, the first two when none is named, on the slot's ports (slot *n*:
  backend 33*n*0, web 33*n*1, gallery 33*n*2), waits until each answers, and
  signs the browser in with the development account, keeping the page it
  shows when the backend still knows its session. `demi.down(...servers)`
  stops exactly the process groups it recorded, never anything found by name,
  so the user's servers and other slots' are never touched;
  `demi.down({ wipe: true })` also removes the backend's data and the paired
  runner. The backend's data lives in the slot's `.cache/browse/`, through
  `bun xtask dev --data`, so a check can restart the backend as an upgrade
  does and see the page come back to the same account and conversations.
  `up` copies the user's `.env` into the slot for the backend and `down`
  deletes it.
- **Seeing.** `demi.shot(name, { element, region, zoom })` writes a PNG at the
  page's real size and pixel ratio, once the page's finite animations end, up
  to two seconds, into `.cache/browse/shots/`, and returns its path, which the
  agent lists in its report for the lead to send. `demi.timeline(action,
  [0, 100, 1500], { region })` runs the action and keeps the browser's frames
  painted nearest to those milliseconds after it, for the moments between a
  click and its result; a full screenshot takes about 100 ms and could not
  show the 100th. `demi.pixel(x, y)` reads a colour.
- **The network.** The browser loads the web app through the tool's forwarder
  on 33*n*3, since the browser's own emulation can neither slow a WebSocket
  nor drop one without a close. `demi.net.latency(ms)`, `.bandwidth(kbps)`,
  `.offline()`, `.unreachable()` (the page's traffic goes nowhere while the
  browser still reports a network), `.cut(pattern)` (sockets close without a
  close frame, as a lost connection does) and `.reset()`. They apply to the
  app's requests and sockets; the development server's own files stay fast.
  The web app's live-update connection bypasses the forwarder, so a restart of
  the tool never reloads the page.
- **Input Playwright cannot give.** `demi.ime(text, { into })` composes text
  through an input method and commits it, as a Chinese or Japanese input
  method does.
- **Conditions.** `demi.emulate({ viewport, scale, theme, device, locale,
  timeZone })` applies to the page as it is, without reloading it; the theme
  reaches the gallery's own setting too. `demi.grant(permission)` gives a
  permission such as the clipboard or local network access.
- **The gallery.** `demi.gallery('/session?view=blocks')` opens a specimen
  page of the slot's gallery.
- **The product's other parts.** `demi.message(text)` replaces any draft,
  sends the text in the open conversation and waits for its turn to end, not
  for background jobs. `demi.runner()` starts the slot's built runner with its
  own installation folder under `.cache/browse/` and pairs it through Add
  Device, or reuses the one it paired; `demi.runner.stop()` and `.start()`
  stop and start that same runner, as a device that goes away and comes back,
  and `demi.runner({ fresh: true })` pairs a new one.
- **Watching.** `demi.log.console()`, `.network()` and `.sockets()` return
  what the page logged, requested, and sent or received since the browser
  started or since `demi.log.mark(name)`, including what goes over a
  [direct channel](../execution/direct-channel.md), which no HTTP request
  shows: each operation as `direct <kind> <status> <path>`, the file watch's
  messages among the sockets'.

## One browser per slot

The first call of a slot starts a browser that lives between calls:
Playwright's Chromium, headless unless `--headed`, with its own profile in the
slot's `.cache/browse/`, so cookies, storage and sign-ins never mix between
slots or with the user's browser. The browser runs as a process of its own that
the tool's server reaches over the DevTools protocol, so when the tool's own
code changes, only the server restarts, with the agent's current code, and the
browser, its pages, what was typed in them, `keep`, the network settings and
the logs stay as they were. A call starts the browser again when it crashed.
The slot comes from the working directory: slot *n* for `../demi-slots/n`, slot
0 for the user's own checkout.

For a person at a shell, `bun browse up`, `bun browse down`, `bun browse stop`
(the browser) and `bun browse help` do what the helpers of the same names do.
Nothing else is a command: a check is a script.

## Whose tool it is

`bun browse` is ordinary code of this repository, and every agent changes it as
part of its own work: a check that needs what the tool lacks adds it to `demi`,
and a helper that misleads or breaks is fixed then, so the next agent has it.
What Playwright already does is never wrapped: a helper exists only for what
this project needs and Playwright lacks. The code stays plain so that this stays
cheap: one file per helper under `packages/browse/src/demi/`, sharing the
browser, the slot and the failure reporting.

## What it is not

It is not the product's tests. Automated tests run without a browser of this
kind and never call a real model ([Testing](testing.md)); `bun browse` is for
the checks a person would do by looking, and for reproducing a bug before it
has a test. It is not the app's shared browser pane either, which the lead
keeps for showing the user something.
