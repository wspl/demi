# Live browser view

The work panel shows the tabs of the conversation's [browser](browser.md) as
the Host renders them, and the user operates them there: clicks, types,
scrolls, pastes, picks a date or a file, answers a dialog, navigates, and opens
and closes tabs. The agent keeps using `demi browser` commands on the same tabs
at the same time. This document owns what the view shows, how its pictures and
input travel, how a tab's viewport and pixel ratio are chosen, and what ends a
view. The browser environment, its tab registry and the agent's commands
belong to [Conversation browser](browser.md). The generic mechanisms the view
uses belong to [Native runtime](native-runtime.md#user-streams),
[Runner](runner.md#service-streams),
[Sessions and targets](sessions-and-targets.md#host-operations) and
[Conversation idle](resource-lifecycle.md#activity); the routes belong to
[Web API](web-api.md#user-streams). The gallery shows what the view looks like.

## What the user sees

For example, the agent starts the user's application at
`http://localhost:3000` on Cloud and opens its login page with
`demi browser open`. The user opens the work panel:

- The login page appears as the Host renders it, sharp on a Retina display,
  because the page renders at the user's device pixel ratio.
- The user clicks the email field and types. The page receives real key
  events, `keydown`, `keypress` and `input`, and Enter submits the form.
- Meanwhile the agent runs `demi browser inspect` and `click`. The user's
  input and the agent's reach the page in the order they arrive.
- The user picks Mobile in the menu beside the address bar. The page lays out
  at 390 × 844 with a phone user agent and touch, and the user's clicks become
  taps.
- The agent runs `demi browser viewport set tab-1 --width 1440 --height 900 --scale 2`.
  The menu gains Custom 1440 × 900 @2 and selects it, and the view scales the
  page to fit the panel.
- The user picks Web. The Custom entry disappears and the page takes the
  panel's size again.
- The user closes the last tab. The browser ends as if the agent had closed it,
  and the agent's next `open` starts a fresh one.

## Parts

```text
User's browser (Demi web)      Backend                     Host (Cloud or paired device)
+----------------------+ WSS  +----------------------+     +----------------------------------+
| web-ui live view     |<====>| user stream route    |pipes| runner --/v1/invoke--> demi-     |
| video, input,        |      | cookie, owner,       |<===>|                        commands  |
| native controls,     |      | Origin; bytes only   |     |   browser environment            |
| viewport menu        |      +----------------------+     |     tab registry                 |
+----------------------+                                   |     live view module             |
                                                           |     Chrome: capture extension,   |
                                                           |     pages and their observers    |
                                                           +----------------------------------+
```

- `demi-commands` owns a live view module inside each conversation's browser
  environment: viewers, capture, delivery, input, and each tab's viewport and
  pixel ratio.
- Chrome runs a capture extension that the environment loads. It encodes each
  watched tab as H.264 and hands the frames to the module over a loopback
  socket on the Host.
- The runner and the backend carry bytes between the module and the page
  through a [user stream](native-runtime.md#user-streams). Neither knows that a
  browser is involved.
- `web-ui` shows the video, sends input, and places the page's native form
  controls over the video.

## A browser tab in the panel

The live view is the content of the work panel's `browser`
[tab kind](web-application.md#work-panel). The panel's tabs are the user's
saved state and know nothing of this document; everything below is the kind's
own business. A `browser` tab's `data` is `{ url, tab? }`: the address it
shows, and the id of the conversation browser's tab it is bound to, once it
has one.

```text
work panel tab state        browser kind                         backend / Host
  add / update / remove  <-- tab source: list, open, close  -->  browser tab routes
                         <-- content: one view, while shown <==> browser user stream
```

- **Opening.** The strip's control adds a tab with `url: 'about:blank'` and no
  `tab`. Its content, once shown, asks the backend to
  [open a tab](web-api.md#conversation-browser-tabs) on that URL and writes the
  answer's id into `data`. That request is ordinary demand: it wakes a stopped
  Cloud and starts the environment as the agent's first `open` does. Meanwhile
  the content shows that the browser is starting; a refusal shows its message
  with Retry. None of this is written to the tab.
- **Showing.** A bound tab's content opens the view (below) and watches its
  `tab`. The address bar follows the page and saves the URL into `data` as it
  changes.
- **Closing.** The user's close removes the panel tab at once and then asks
  the backend to close the browser's tab; closing the last one ends the
  environment, as the agent's `close` does.
- **The agent's tabs.** The kind reads the browser's
  [tab list](web-api.md#conversation-browser-tabs) when the panel opens, when
  the page becomes visible, when a tool call of the conversation finishes, and
  from every `state` message of an open view. A browser tab that no panel tab
  is bound to is added as a panel tab, after the others and without taking the
  selection.
- **A tab the Host no longer has.** The agent closed it, the browser ended, the
  Cloud stopped, or the Host restarted. The panel tab stays: the kind never
  removes a tab. Its content says that the page was closed on the device and
  offers two ways on: Close tab, and Reload, which opens a new browser tab on
  the saved URL and binds to it. A reopened page shares storage only while the
  same browser environment remains alive. Retirement removes its temporary
  profile; a new environment starts with fresh storage
  ([Lifetime](browser.md#lifetime)).
- **A Host that cannot be reached.** A stopped Cloud, an offline device, or a
  lost connection changes no tab. The content of the shown tab says what is
  wrong, reconnects by itself where that can help, and offers Retry where it
  cannot.

A page has at most one view per conversation, open only while a `browser` tab
is the panel's selection. Selecting another `browser` tab keeps the view and
sends `watch`; the module then releases what the viewer held on the old tab,
starts a new stream generation and sends the new tab's dialog, controls and
cursor, and the page discards frames of older generations. A view carries one
watched tab at a time: nothing of one tab can reach another. Selecting Change,
File or another kind closes the view, so the Host captures nothing.

## The stream

### Opening a view

1. The page opens `WS /api/conversations/:id/streams/browser` on the product
   origin. The backend checks the session cookie, that the conversation belongs
   to the user, and the `Origin` header.
2. The backend admits the stream through the conversation's host access,
   without waking a stopped Cloud: a stopped Cloud holds no browser, so the
   shown tab's content says that the Cloud is stopped
   ([Host operations](sessions-and-targets.md#host-operations)).
3. The backend mints two pipes and asks the main Host's runner to open the
   `browser` user stream on them ([Service streams](runner.md#service-streams)).
4. The runner invokes the declared operation on the resident `demi-commands`
   service. Its [command context](native-runtime.md#command-context) names the
   conversation and a `user` caller, and its input and output are the two
   pipes.
5. The live view module answers with the tab list and the watched tab's first
   key frame.

One view is one invocation. The page shows one Host: the conversation's main Host. A browser on an
attached Host is not shown.

The view carries only what a request cannot
([Backend communication](web-application.md#backend-communication)): pictures,
the user's input and its answers to dialogs and native controls, which must
reach the page in order, and what belongs to this viewer's view, the watched
tab, the panel's size and ratio, the viewport mode that follows them, and
frame acknowledgements. Listing, opening, closing and navigating tabs, and
Back, Forward and Reload, are
[requests](web-api.md#conversation-browser-tabs): each has an answer the
content can show, and each works while no view is open.

The module never fails silently toward the page. What it cannot do for a
viewer, such as reading the tab list, it tells that viewer in a `notice` and
writes to the [Host's log](runner.md#host-log).

A view ends once. The module's `ended` message and the socket's close are one
end, and the page opens at most one view after it. An answer that cannot
change, such as 404 for a conversation that does not exist, is not retried.

### Framing and versions

Pipes carry bytes without message boundaries, so the protocol frames each
message with a length prefix. The backend forwards bytes as they arrive; the
page and the module split frames. Control messages are JSON validated against
`browser-protocol` schemas. Video frames are binary: a header naming the tab,
the stream generation, a sequence number, whether the frame is a key frame, its
timestamp and its size, followed by H.264 Annex B data. The page and the
module ship in the same release, as the page and the backend do, so the
protocol carries no version.

### Backpressure

Each hop forwards only what the next hop accepts:

```text
page reads slowly
  -> the backend's WebSocket send buffer fills, and the backend stops pulling
  -> the runner's upload of the output pipe waits
  -> the invocation's output queue fills (four records)
  -> the live view module cannot write, and drops frames until the next key frame
```

The backend must honor its WebSocket's send backpressure. A relay that ignores
it lets pictures pile up in backend memory instead of dropping stale ones on the
Host.

### Ending a view

| Event | Result |
| --- | --- |
| The page closes, hides the view, or loses its connection | The invocation is cancelled; the keys and buttons this viewer holds are released; capture of a tab nobody watches stops |
| The browser environment ends: last tab closed, conversation release, Chrome crash, Cloud stop | The module tells the page that the browser ended, and the stream ends |
| Archive, target or directory change, detach | The backend ends the stream, as it ends file transfers |
| The Host becomes unreachable | The stream ends; the page reconnects when the conversation's host access admits it again |

## Pictures

### Capture

Chrome captures tabs through a Chrome extension that the environment always
loads ([Native driver](browser.md#native-driver)). The extension works in an
offscreen document, which Chrome lists as a background page, not as a tab, so
it never enters the tab registry. It captures a tab with `chrome.tabCapture`,
encodes H.264 High 4:2:0 with WebCodecs in software, and sends each encoded
frame to the live view module over a WebSocket on `127.0.0.1` that accepts only
the environment's token. The frames do not travel over CDP: base64 and JSON
would inflate them, they would share the connection that carries the agent's
commands, and CDP messages have a size limit.

A tab is captured only while someone watches it. Viewers of the same tab share
one capture and one encoding.
Stopping a capture also invalidates its queued restart. A delayed recovery from
an old capture must not reacquire the tab after its viewer has left or switched.
The Host owns capture retries and their backoff. The extension reports a failed
or persistently silent capture and releases it; it never starts an independent
replacement that can outlive the Host's capture owner.
If Chrome aborts stream creation after granting capture, it can retain a request
that rejects every later capture even though the extension received no track to
stop. Recreate the capture extension to release those Chrome-owned requests.
This interrupts pictures in that browser environment; its Host resumes only the
currently watched streams. Existing tabs, documents and form state remain alive,
and no page action is repeated. A new extension connection retires the previous
connection's captures before it admits replacement work. The environment's
owned Chrome process permits its unpacked capture extension to reload;
Chrome distinguishes the initial command-line load from a later unpacked reload.
This permission belongs to the process configuration, not a developer-mode
profile preference that Chrome's preference protection can reset.

The pinned Chrome labels the BT.601 samples of Linux software capture as
BT.709. The extension corrects the label for exactly that case, and every
Chrome upgrade re-verifies the colors against the page's own pixels.

On Linux arm64, a CPU that reports SME without SVE cannot capture: Apple M4
and later have no SVE, and a Linux VM that passes their SME through, such as
OrbStack from 2.2.3, runs Chrome's SME code into an SVE instruction before it
enters streaming mode. The GPU process then dies on every capture until Chrome
gives up and exits, taking the agent's tabs with it. The module checks the
CPU's capabilities before capturing: on such a Host the view keeps its tabs,
input and dialogs, shows no picture and says why, and the agent's browser
keeps running. Booting the VM's kernel with `arm64.nosme` restores capture.
Lima's `vz` VMs do not pass SME through.

### Delivery

- The module sends a key frame when a viewer connects, when the watched tab or
  its viewport changes, and when the page asks after a decode error.
- Shortly after a picture stops changing, it is encoded once more at higher
  quality; a still page then sends nothing further.
- The module sends a heartbeat whenever it has sent nothing else for 250 ms.
  The page tells a still page from a stalled connection by it: after one second without any byte, the page
  shows the connection as stalled and stops sending input, discarding it rather
  than queueing it. For example, a Cloud pauses while it copies its disks for a
  checkpoint, with the browser still running
  ([Managed hosts](managed-hosts.md#lifecycle-and-capacity)); input typed
  during the pause must not reach the page all at once afterward.
- The page acknowledges each frame it shows. The module adapts from
  end-to-end acknowledgement delay: queueing delay is the main signal. Under
  congestion it lowers the bit rate, then the frame rate, then the resolution.
  A hidden page, a blocked page and a stall are pauses, not congestion. There
  is no fixed bit rate ceiling and no Cloud-specific limit.

## Viewport and pixel ratio

### Modes

Each tab has one viewport mode. The menu beside the address bar shows the mode
of the tab being watched.

| Mode | Viewport | Pixel ratio | Phone emulation |
| --- | --- | --- | --- |
| Web (default) | The panel's size in whole CSS pixels | The viewer's `devicePixelRatio` | No |
| Mobile | 390 × 844 CSS pixels | The viewer's `devicePixelRatio` | `mobile`, touch, an Android Chrome user agent with matching client hints |
| Custom | What the agent set | What the agent set | No |

- Choosing Mobile, or leaving it, loads the page again. A phone's user agent,
  client hints and touch reach only what loads after them: the page already in
  the tab was served to the other kind of device and would keep that layout,
  a desktop page shrunk into a phone's width.
- Custom appears only after the agent runs `viewport set`
  ([Evaluation, console, and viewport](browser.md#evaluation-console-and-viewport)).
  The menu lists it as Custom W × H @S and selects it. Choosing Web or Mobile
  discards the agent's setting and removes the entry. The agent's
  `viewport reset` returns the tab to Web.
- The Mobile client hints keep the browser's own brands and set `mobile` and
  the Android platform, so the user agent and the hints agree.
- When the viewport and the panel differ, as in Mobile and Custom, the view
  scales the picture to fit and centers it. Input maps back to page
  coordinates.
  During resize, the retained frame keeps its own aspect ratio until a frame
  painted at the new viewport size replaces it; viewport metadata must not
  stretch the old picture.
- A Web tab nobody watches keeps its last size. A tab never watched is
  1280 × 720.
- With several viewers, the viewer that operated most recently decides the Web
  size and the pixel ratio. The others see the same picture scaled.

In Web mode, what the agent observes follows the user's panel: a screenshot
taken while the panel is 700 × 800 is a 700 × 800 page. An agent that needs a
fixed size sets one. `open`, `info`, `screenshot`, `probe` and `viewport`
report the tab's mode and viewport
([Structured result fields](browser.md#structured-result-fields)).

### Pixel ratio

Chrome paints a window at its screen's pixel ratio. The environment therefore
sets the pixel ratio of its virtual screen with `Emulation.updateScreen`, with
the viewer's screen size, and pins each tab's viewport with
`Emulation.setDeviceMetricsOverride` at the same ratio. It sizes each window to
the viewport plus the browser's own chrome before applying the page's metrics,
so a page never sees an outer size smaller than its inner size. The picture
must fill those metrics from its first frame; a window resize must not leave
black bars that move the visible page away from its input coordinates.
After setting the window and page metrics, wait for a surface repaint before
publishing the viewport to capture. Chrome acknowledging the metrics does not
mean its compositor has applied them; starting a new capture earlier can scale
and letterbox the old surface into the new dimensions.

- The screen is shared by the environment's windows. The ratio follows the
  viewer that operated most recently.
- macOS accepts only whole screen ratios. A fractional viewer ratio, such as
  1.5, rounds up, and the page scales the picture down.
- The ratio stays as it is when viewers leave; only the next viewer's
  operation changes it.
- The agent's screenshots stay in CSS pixels whatever the ratio
  ([Screenshot and probe](browser.md#screenshot-and-probe)).

## Input

The user's input and the agent's are not exclusive. Both reach the page in the
order they arrive; the per-tab operation lock orders agent commands among
themselves only. Interleaving is expected behavior:

- The agent's `fill` focuses a field and types. If the user clicks another
  field in between, the text goes to the field the user clicked, and the
  command still reports success.
- The agent's `click --wait-url` waits for a navigation. A navigation the
  user's click caused satisfies it.

Keys and buttons held down are tracked per source, each viewer and each agent
command. A source's end releases only what it holds.

| Input | Delivery |
| --- | --- |
| Keys | One key event per key, carrying the character it types; Enter carries `\r`, so newlines, form submission and button activation work. The mapping is `keyboard.rs`, shared with agent commands. |
| Input methods and dead keys | Composed in a hidden text field of the Demi page, then sent as composition updates and committed text. |
| A Mac viewer on a Linux or Windows Host | Command becomes Control; Command and Option editing keys become their Home, End and Control equivalents. |
| A macOS Host | Editing shortcuts also carry Blink editing commands, which CDP key events cannot reach through menus. |
| Pointer | CSS coordinates of the tab. Command-click becomes Control-click on a Linux or Windows Host. |
| Wheel | Deltas in CSS pixels, dispatched as they are: with the screen at the viewer's ratio, Chrome scrolls a CDP wheel delta by that many CSS pixels at every ratio, including a tab whose ratio differs from the screen's (measured on macOS and Linux at 1, 1.5, 2 and 3). |
| Mobile mode | The viewer's pointer press, move and release become touch events. Chrome's mouse-to-touch conversion is not used: with it, CDP mouse calls never return. |
| Copy on the Host | Text the watched tab copies within 5 seconds of the viewer's input reaches the local clipboard: what a copy or cut takes from the page, and, where the Host's browser clipboard is isolated from the Host user's ([Upload, download, and clipboard](browser.md#upload-download-and-clipboard)), what the page writes with the Clipboard API. Password fields are never read. |
| Paste | The text and HTML go to the Host browser's isolated clipboard, then the Host's paste shortcut, so the page receives a real `paste` event. A Host without a browser clipboard receives inserted text. |
| Native form controls | See below. |
| Dialogs | Shown in a panel that does not block the page; the first answer wins, whether the user's or the agent's. |

Select, date, time, color, suggestion and file inputs open the viewer's own
native pickers. An observer script in each page reports those controls, their
values and positions, with an identity and a revision. The view places local
native controls over the video. A choice applies only to the revision the
viewer saw, or to one its own earlier choice produced, so fast typing into a
suggestion field is not rejected. The page receives the value with synthetic
`input` and `change` events. Chosen files travel through the stream itself,
which is a pipe, into a directory of the environment, and then attach to the
input. Observers run
in an isolated world, so pages can neither see nor call them.

A user operation, meaning a button press, a key, a wheel turn, a paste or a
control choice but not pointer movement, is conversation activity. The page
reports it at most every 30 seconds
([Activity](resource-lifecycle.md#activity)). Watching is not activity.

## Security

- The stream operates a browser signed in to the user's sites. Its route
  requires the session cookie, ownership of the conversation, and an `Origin`
  of the product.
- CDP stays on the Host. The page speaks only the live protocol.
- The capture socket binds loopback and accepts only its environment's token.
- Page observers run in an isolated world.

## Acceptance

Run real Chrome for Testing against local fixture pages, with scripted
providers and never real models, on a macOS paired device, a Linux paired
device and Cloud. The fixture server records what each page received, and the
checks read it there.

1. Pictures: a moving page shows continuously, a still page keeps its last
   picture, and a new viewer, a tab switch and a viewport change start from a
   key frame.
2. Pixel ratio: at viewer ratios 1, 2 and 3, and 1.5 on Linux, a pattern of
   half-CSS-pixel stripes arrives with real detail at 2 and 3 and as flat gray
   at 1. Mobile and Custom keep their sizes, and the Custom entry appears and
   disappears as [Modes](#modes) says.
3. Input reaches the page as a local Chrome delivers it: Enter in a textarea, a
   form field and a focused button; `preventDefault` in `keydown` stopping a
   character; `keypress` for printable keys; Mac editing shortcuts on a Linux
   Host and on a macOS Host; a real `paste` event; copied text on the local
   clipboard; a wheel turn scrolling the same CSS distance at ratios 1 and 2;
   every native control kind, including fast typing into a suggestion field and
   a file choice; taps in Mobile mode.
4. Agent commands and user input on one tab both take effect. Ending a viewer
   or cancelling an agent command releases only the keys and buttons it holds.
5. A check page reports, on every platform: `navigator.webdriver` false; no
   headless token in the user agent of the page, a worker or a request; hover
   and a fine pointer on Linux; a screen and window consistent with the
   viewport; scrollbars; the user's time zone and languages.
6. At ratio 2, `screenshot` returns an image in CSS pixels and reports ratio 2.
7. After the Host pauses, the page shows the stall, discards the input typed
   meanwhile, and resumes from a key frame. Archive, target change, detach,
   Cloud stop, closing the last tab and a Chrome crash each end the view in the
   state [Ending a view](#ending-a-view) gives.
8. A page that stops reading does not grow backend memory; the Host drops the
   stale frames.
9. Retirement leaves no Chrome process, helper, profile or capture socket
   behind.

## Responsibilities

| Where | Responsibility |
| --- | --- |
| `packages/browser-protocol` | The live protocol: message schemas and the frame header; generated native bindings. |
| `crates/demi-commands` | The live view module: viewers, capture control, delivery and congestion, heartbeat, input, viewport modes and screen ratio; the capture extension and page observers as embedded resources; launch configuration. |
| `crates/command-service`, `crates/runner`, `packages/runner-protocol`, `packages/host-remote` | [User streams](native-runtime.md#user-streams) and [service streams](runner.md#service-streams), with no browser knowledge. |
| `packages/coding-agent` | Declaring the `browser` user stream and `viewport set --scale`. |
| `packages/backend` | The user stream route, its admission and end, backpressure, and activity reports; the [browser tab routes](web-api.md#conversation-browser-tabs), which call the browser's own operations and hold no browser logic. |
| `packages/web-ui` | The `browser` tab kind: its tab source, which lists, opens and closes tabs through an interface the consumer supplies, and its content, the live view: video, input, native control overlays, clipboard, dialogs, the viewport menu, and what it shows while a tab is opening, gone, or out of reach. It depends on `browser-protocol` for the protocol, as it depends on `agent` for agent frames. |
| `packages/web`, `packages/web-gallery` | The product's stream source, tab routes and activity reports; a gallery source that encodes its own picture, keeps its own tab list and speaks the protocol, so the kind shows without a Host. |
| `packages/guest-image` | Fonts for Chinese, Japanese and Korean text. |

## Rationale

- Chrome's own screencast sends every frame as a separate image over CDP,
  with no compression between frames, on the connection that carries the
  agent's commands. Tab capture with H.264 encodes only what changes between
  frames and leaves CDP to the commands
  ([Browser tab H.264 experiment](../browser-tab-h264-experiment.md)).
- WebRTC would bring its own congestion control, but there is no direct path
  between a page and a Host: every byte goes through the backend, which WebRTC
  would need a TURN relay for.
- A launch flag fixes a Chrome process's pixel ratio for its lifetime, and on
  Linux neither emulating a ratio nor requesting a larger capture adds real
  detail; both upscale a 1× picture. The virtual screen's ratio, changed at
  run time, gives real detail at the viewer's ratio without restarting the
  browser.

## Implementation status

Implemented: the protocol, the [user stream](native-runtime.md#user-streams)
that carries it, the Host's live view module with its capture extension and
page observers, the `browser` tab kind in `web-ui` with its tab source and
content, the [browser tab routes](web-api.md#conversation-browser-tabs), the
product's stream source, tab requests and activity reports, the gallery's own
browser, and the guest image's fonts for Chinese, Japanese and Korean.

Verified against real Chrome on macOS arm64 and Linux arm64/amd64: watching a tab beside
the agent, the viewport modes, the viewer's ratio in the picture it receives
(half-CSS-pixel stripes, decoded from the capture), dialogs, native controls,
chosen files, the clipboard, held input released with its viewer, a tab the
viewer opens while it watches another, and two viewers of one tab. An
end-to-end test drives a page's stream through the backend and the runner to a
paired device's Chrome (`DEMI_BROWSER_LIVE_E2E=1`), including a page that stops
showing frames and a Host that pauses in the middle of a view. The same pages
and viewer run against a real Cloud guest
(`DEMI_BROWSER_LIVE_CLOUD_E2E=1`): its pictures at the viewer's size and ratio,
the viewer's input, the agent's commands on the same tab, the view ending with
the browser, and the guest image drawing Chinese, Japanese and Korean text.

Concurrent viewing and agent commands have also been exercised on paired Hosts
and gVisor/systrap Clouds, including DPR 1/2/3, checkpoint during capture,
reconnection, and repeated browser retirement. A small Host shares its CPU
between Chrome, software compositing, H.264 encoding and commands. Capture can
therefore increase command latency even when delivery is prompt; the current
controller measures delivery pressure rather than an encoder or Host CPU budget.
These functional checks do not establish capacity for every page or workload.

Not verified yet: a Windows Host. The prototype and its measurements live in
the separate Tab Lab repository (`browser-remote-lab`).
