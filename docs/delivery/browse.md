# bun browse

A product check uses the running product, or the gallery, the way a person
would, and shows the result in screenshots ([AGENTS.md](../../AGENTS.md)). This
document owns the tool agents do it with: `bun browse`, a command-line tool over
[Playwright](https://playwright.dev) in the private package `packages/browse`.

For example, an agent in slot 2 checks that Reload shows the loading state at
once on a far backend:

```text
$ bun browse up web
Backend   http://127.0.0.1:3320  ready in 71 s
Web       http://127.0.0.1:3323  ready in 4 s, signed in as developer@example.test
$ bun browse net latency 400
$ bun browse open /chat/c-1
$ bun browse timeline 'click role=button[name="Reload"]' --at 0,100,1500
/…/demi-slots/2/.cache/browse/shots/timeline-0ms.png     painted at -30 ms
/…/demi-slots/2/.cache/browse/shots/timeline-100ms.png   painted at 85 ms
/…/demi-slots/2/.cache/browse/shots/timeline-1500ms.png  painted at 1484 ms
$ bun browse down
```

## Why a tool of our own

Before it, every check started from nothing. In 39 checks, agents wrote at least
14 browser drivers and about 200 one-off scripts, and lost the most time to
getting the product into the state a check needs, fixed sleeps, elements they
could not find, the app's shared browser pane being hidden or shared, and
servers they could not stop cleanly; one agent's `pkill` stopped the user's own
backend. `bun browse` does each of those once, in one place, the same way every
time.

## What it does

**One browser per slot.** The first command of a slot starts a browser that
lives between commands: Playwright's Chromium, headless unless `--headed`,
with its own profile in the slot's `.cache/browse/`, so cookies, storage and
sign-ins never mix between slots or with the user's browser. Commands reach it
through a socket in the same folder; it ends on `bun browse stop`, on `bun
browse down` or after 30 minutes without a command. The browser runs as a
process of its own that the tool's server reaches over the DevTools protocol,
so when the tool's own code changes, only the server restarts, with the
agent's current code, and the browser, its pages and what was typed in them
stay as they were; the new server keeps the browser's `net` settings, which
the slot's state holds until the browser stops, and goes on from the page's
logs and the log mark the last one left. A command starts the browser again
when it crashed. The slot comes from the working directory: slot *n*
for `../demi-slots/n`, slot 0 for the user's own checkout.

**The slot's servers.** `bun browse up [backend] [web] [gallery]` starts what
it names on the slot's ports (slot *n*: backend 33*n*0, web 33*n*1, gallery
33*n*2), waits until each answers, puts the tool's own forwarder on 33*n*3 in
front of the web app, which the browser loads it through, since the browser's
own network emulation can neither slow a WebSocket nor drop one without a
close, signs the browser in with the development
account once the web app is up, and records each process group. `bun browse
down` stops exactly those groups, never anything found by name, so the user's
servers and other slots' are never touched; `down <server>…` stops only those.
`up` copies the user's `.env` into the slot for the backend and `down` deletes
it. The backend's data lives in the slot's `.cache/browse/`, through `bun xtask
dev --data`, and survives `down` and `up`, so a check can restart the backend
as an upgrade does and see a page come back to the same account and
conversations; the paired runner stays too. `down --wipe` removes both. `up web` keeps the page the
browser shows when the backend still knows its session; otherwise it signs in
and loads the same page again, so a sign-in page goes on to the page it was
opened for. Only a page outside the web app opens `/`. The
web app's development server keeps its live-update connection on its own
port rather than through the forwarder, so a restart of the tool never
reloads the page.

**Finding things.** Every command that acts takes a Playwright locator:
`role=button[name="Reload"]`, `text=Allow for This Conversation`,
`label=Email`, `testid=…`, or CSS. `text=` matches the whole visible text of
an element, as a person reads a label; `text~=` matches a part of it. A
textbox locator also finds a field the page marks as a combobox, as a person
sees one, and also finds an editable element with that label, such as the
composer. A role's quoted name, `role=button[name="Add Device"]`, matches the
whole accessible name ignoring a trailing ellipsis, as a person reads "Add
Device…"; `[name~="device"]` matches a part of it in any case. A locator that
matches more than one element fails at once, as do `wait` and `wait gone`, and one that matches nothing after two seconds, time
for a menu or dialog still opening; either failure says what it looked for
and what it found, with a screenshot. Gallery specimens open by address, `bun browse open
gallery:/session?view=blocks`.

**Acting.** `click`, `fill`, `type`, `press`, `hover`, `select`, `upload`,
`drag`, `scroll`, and `ime <text>`, which composes the text through an input
method and commits it, as a Chinese or Japanese input method does. An action
on an element a dialog covers fails at once and names the dialog, since the
dialog would take the click, the typing or the keys.

**Waiting for what happens, never for time.** `wait` takes a locator to appear
or go, text, a URL, a JavaScript condition, or `turn`, the end of the open
conversation's current turn. Every wait has a timeout and says on failure what
it waited for.

**Seeing.** `shot [name]` writes a PNG at the page's real size and pixel ratio,
once the page's finite animations end (up to two seconds; `--now` at once),
into `.cache/browse/shots/` and prints its path, which the agent lists in its
report for the lead to send. `--element <locator>` clips to an element,
`--region x,y,w,h` to an area, `--zoom 2` magnifies without changing the
page's own pixel ratio. `timeline '<action>' --at 0,100,1500 [--region …]` runs one action and keeps the browser's frames painted nearest to
those milliseconds after it, for the moments between a click and its result;
each line says when its frame was painted, since a full screenshot takes
about 100 ms and could not show the 100th. `pixel x y` reads a colour.

**Conditions.** `emulate` sets the viewport, pixel ratio, light or dark
theme, a phone, locale and time zone, for the page as it is, without
reloading it; the theme reaches the gallery's own setting too. `net` makes the network offline or
online, adds latency or limits bandwidth for the app's requests and sockets
(the development server's own files stay fast, or a page of hundreds of
modules would take half a minute to open), or cuts the sockets matching a
pattern without a close, as a lost connection does. `grant` gives a permission such as the clipboard or local network
access.

**Watching.** `log console`, `log network` and `log sockets` print what the
page logged, requested and sent or received since the browser started or
since a mark, `log mark`.

**The real product's other parts.** `message <text>` replaces any draft and sends the
text in the open conversation and waits for its turn to end; `runner` starts the slot's
built runner with its own installation folder under `.cache/browse/` and pairs
it to the slot's backend through Add Device; `runner stop` and `runner start`
stop and start that same runner, as a device that goes away and comes back,
and `runner --new` pairs a new one. `focus` says which element has the
focus, and `download '<command>'` keeps the file a command downloads.

**Anything else.** `eval <js>` runs JavaScript in the page, `await` included, and prints the
result, `cdp <method> [params]` sends a DevTools protocol command, and
`script <file>` runs a script with the page, its context and a CDP session, for
a step no command covers yet.

## Whose tool it is

`bun browse` is ordinary code of this repository, and every agent changes it as
part of its own work: a check that needs what the tool lacks adds it, as a
command or an option, and a command that misleads or breaks is fixed then, so
the next agent has it. A `script` that proves useful becomes a command. The
code stays plain so that this stays cheap: one file per command under
`packages/browse/src/commands/`, sharing the browser, the slot and the
locator handling.

## What it is not

It is not the product's tests. Automated tests run without a browser of this
kind and never call a real model ([Testing](testing.md)); `bun browse` is for
the checks a person would do by looking, and for reproducing a bug before it
has a test. It is not the app's shared browser pane either, which the lead
keeps for showing the user something.
