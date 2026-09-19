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
| viewport menu        |      +----------------------+     |     tab registry (existing)      |
+----------------------+                                   |     live view module (new)       |
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

## The stream

### Opening a view

1. The page opens `WS /api/conversations/:id/streams/browser` on the product
   origin. The backend checks the session cookie, that the conversation belongs
   to the user, and the `Origin` header.
2. The backend admits the stream through the conversation's host access,
   without waking a stopped Cloud: a stopped Cloud holds no browser, so the page
   shows that the browser is not running
   ([Host operations](sessions-and-targets.md#host-operations)).
3. The backend mints two pipes and asks the main Host's runner to open the
   `browser` user stream on them ([Service streams](runner.md#service-streams)).
4. The runner invokes the declared operation on the resident `demi-commands`
   service. The invocation names the conversation as its trusted
   `conversation` and the user as its `caller`, and its input and output are
   the two pipes.
5. The live view module answers with the protocol version, the tab list and the
   watched tab's first key frame.

One view is one invocation. The page shows one Host: the conversation's main
Host. A browser on an attached Host is not shown.

When no browser runs, the page says so and offers a new tab. A new tab is
ordinary demand: it wakes a stopped Cloud, starts the environment as the
agent's first `open` does, and then opens the view.

### Framing and versions

Pipes carry bytes without message boundaries, so the protocol frames each
message with a length prefix. The backend forwards bytes as they arrive; the
page and the module split frames. Control messages are JSON validated against
`browser-protocol` schemas. Video frames are binary: a header naming the tab,
the stream generation, a sequence number, whether the frame is a key frame, its
timestamp and its size, followed by H.264 Annex B data.

The first message carries the protocol version. A page and a Host with
different versions do not talk: the page says whether to refresh the page or
update the device. There is no compatibility mode.

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

The pinned Chrome labels the BT.601 samples of Linux software capture as
BT.709. The extension corrects the label for exactly that case, and every
Chrome upgrade re-verifies the colors against the page's own pixels.

### Delivery

- The module sends a key frame when a viewer connects, when the watched tab or
  its viewport changes, and when the page asks after a decode error.
- Shortly after a picture stops changing, it is encoded once more at higher
  quality; a still page then sends nothing further.
- The module sends a heartbeat every 250 ms. The page tells a still page from
  a stalled connection by it: after one second without any byte, the page
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
the viewport plus the browser's own chrome, so a page never sees an outer size
smaller than its inner size.

- The screen is shared by the environment's windows. The ratio follows the
  viewer that operated most recently.
- macOS accepts only whole screen ratios. A fractional viewer ratio, such as
  1.5, rounds up, and the page scales the picture down.
- A minute after the last viewer leaves, the screen returns to ratio 1 and
  every tab's ratio follows. Pages see `devicePixelRatio` change, as they do
  when a user drags a window to another display.
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
| Wheel | Deltas in CSS pixels, scaled by the tab's pixel ratio before dispatch. |
| Mobile mode | The viewer's pointer press, move and release become touch events. Chrome's mouse-to-touch conversion is not used: with it, CDP mouse calls never return. |
| Copy on the Host | The copied text reaches the local clipboard when it comes from the watched tab within 5 seconds of the viewer's input; password fields are never read. |
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
input; one choice carries at most 25 MiB, the attachment limit. Observers run
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
| `packages/browser-protocol` | The live protocol: message schemas, the frame header and the protocol version; generated native bindings. |
| `crates/demi-commands` | The live view module: viewers, capture control, delivery and congestion, heartbeat, input, viewport modes and screen ratio; the capture extension and page observers as embedded resources; launch configuration. |
| `crates/command-service`, `crates/runner`, `packages/runner-protocol`, `packages/host-remote` | [User streams](native-runtime.md#user-streams) and [service streams](runner.md#service-streams), with no browser knowledge. |
| `packages/coding-agent` | Declaring the `browser` user stream and `viewport set --scale`. |
| `packages/backend` | The user stream route, its admission and end, backpressure, and activity reports. It has no browser module. |
| `packages/web-ui` | The live view: video, input, native control overlays, clipboard, dialogs, the viewport menu, stall display. It depends on `browser-protocol` for the protocol, as it depends on `agent` for agent frames. |
| `packages/web`, `packages/web-gallery` | The product's stream source and activity reports; a gallery source replaying recorded frames and messages. |
| `packages/guest-image` | Fonts for Chinese, Japanese and Korean text. |

## Open decisions

- **Local frame tabs.** The work panel's browser tabs today show pages in the
  user's own browser, in a frame, and exposes open there
  ([Web architecture](web-application.md#package-responsibilities)). Host
  tabs either replace them, or the panel keeps both kinds side by side.
- **Time zone and languages on the Host.** The browser applies the user's
  last time zone and languages ([Native driver](browser.md#native-driver)).
  They reach the Host either as job context variables, such as
  `DEMI_TIME_ZONE` and `DEMI_LANGUAGES`, or as a trusted invocation field. The
  standard `TZ` and `LANG` are not used: they would change the behavior of the
  user's own tools and tests.
- **Stream budget.** A service admits at most 32 concurrent invocation streams,
  and a user stream holds one for as long as the view is open. User streams
  need their own count, so that open views never block agent commands.

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

Designed, not implemented. The prototype and its measurements live in the
separate Tab Lab repository (`browser-remote-lab`).
