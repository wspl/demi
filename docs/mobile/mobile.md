# Mobile simulators

The agent runs the user's iOS or Android app in a simulator on a paired
device, operates it as a person would, and the user watches and operates the
same simulator live in the work panel. For example, Alex asks the agent to
check the sign-up flow of his app:

```text
$ xcodebuild -scheme Shop -destination 'platform=iOS Simulator,name=iPhone 17' build
$ demi mobile open "iPhone 17"
m1  iPhone 17 · iOS 26.5 · 1179×2556 · simctl id 5F1C…
$ xcrun simctl install 5F1C… build/Shop.app && xcrun simctl launch 5F1C… com.example.shop
$ demi mobile inspect m1
$ demi mobile tap m1 e7                      # "Sign Up"
$ demi mobile type m1 "alex@example.com" --into e3
$ demi mobile screenshot m1 | demi file view
```

The iPhone appears in the work panel as the agent opens it; Alex taps
through the app himself between the agent's steps.

This is a [plugin](../architecture/plugins.md), `plugin-mobile`, as the
browser is: the `demi mobile` commands, the panel and the Host-side drivers
are its own, and nothing in the agent, the backend or the runner knows a
simulator.

## Scope

- **Booting without a window.** `open` boots a shut-down simulator the way
  `simctl boot` does, without Simulator.app; a Simulator.app the user has
  open shows it as it shows any booted simulator.
- **Simulators only**: the iOS Simulator, on a Mac with Apple silicon,
  macOS 15 or later and Xcode, as the library Demi uses is built only for
  those, and the
  Android Emulator, on a Mac, Linux or Windows device with the Android SDK.
  A physical iPhone or Android phone is not supported: an iPhone takes no
  input without a signed XCTest runner on it, and a phone holds a person's
  real accounts.
- **Paired devices only.** A Cloud has neither Xcode nor the hardware
  virtualization the Android Emulator needs; `demi mobile` there says so.
- **What Demi does and what the native tools do.** Demi does what touches
  the screen, the input and who controls them: opening a device, the live
  view, screenshots, the UI tree, taps, swipes, typing, keys, rotation,
  light or dark appearance and recording: everything the panel offers the
  user, each the one operation behind both the panel's control and its
  command, so the agent can do what the user can and the other way round.
  Building, installing, launching an app, logs, deep links,
  locale, location, text size, permissions and creating or deleting
  simulators are done with the tools the agent already knows, `xcodebuild`,
  `xcrun simctl`, `avdmanager`, `emulator` and `adb`, which name every one
  of these exactly; `demi mobile open` prints the device's `simctl` id or
  `adb` serial for them.
- **Installing what is missing.** Xcode and the Android SDK are the user's,
  installed as any developer tool: Xcode from the App Store, the SDK, its
  emulator and system images by the agent with `sdkmanager` when asked.
  Demi's own parts, the iOS helper and the Android one, ship with the
  runner. A command that needs what the device lacks says what and how:
  `demi mobile open: no Android emulator; install the Android SDK's
  emulator package (sdkmanager "emulator")`.

## Commands

```text
demi mobile list
demi mobile open <device>
demi mobile close <m>
demi mobile screenshot <m> [--output <path>]
demi mobile inspect <m> [--keyboard]
demi mobile tap <m> <x> <y> | <m> <element> [--long]
demi mobile swipe <m> <x1> <y1> <x2> <y2> | <m> <element> up|down|left|right
demi mobile type <m> <text> [--into <element>] [--replace]
demi mobile key <m> home|back|lock|volume-up|volume-down|enter|delete|tab|escape
demi mobile rotate <m> portrait|landscape|left|right
demi mobile appearance <m> light|dark|toggle
demi mobile wait <m> --text <text> | --element <element> | --gone <element> [--timeout <s>]
demi mobile record <m> start | stop
```

The verbs are the browser's where the meaning is the same
([Command reference](../browser/browser.md#command-reference)), so an agent
that uses `demi browser` already knows them.

- **`list`** prints a table of every simulator of the Host it runs on, iOS
  and Android together; on an attached Host it runs inside `demi host
  shell`, as every command does: its id, the `simctl` id or AVD name `open`
  takes, platform, OS version, state (shut down, booted, open in this
  conversation, or open in another one), its handle such as `m1` while open
  here, model and name. A platform the Host cannot run is a warning on
  stderr, *iOS simulators need a Mac*.
- **`open <device>`** takes a name, a `simctl` id or an AVD name; a name
  two devices share is refused, listing both with their ids, so the agent
  names the one it means. It boots the
  device when it is shut down, without a window, attaches it to the
  conversation and opens it in the panel. It answers the handle, `m1`, the
  screen size and the native id, and for an Android emulator its adb serial,
  `adb serial emulator-5554`, which the agent's own `adb -s` takes. Handles are the conversation's, across
  its Hosts, so `m2` may be an Android emulator on a Linux workstation while
  `m1` is an iPhone on the Mac. A conversation has at most four devices
  open on each Host, the most a computer runs well beside its work; a
  device open in another conversation is refused, naming it, so two
  conversations never drive one device. The panel shows the devices open
  on the conversation's own Host; a device the agent opens on an attached
  Host is driven by its commands alone, with no tab, until the panel
  reaches attached Hosts.
- **`close <m>`** detaches it, and shuts it down when Demi booted it.
- **`screenshot <m>`** writes a PNG to stdout, as `demi browser screenshot`
  does ([Images and large outputs](../browser/browser.md#images-and-large-outputs)),
  scaled so its long side is at most 1568 pixels, the most a model reads at
  full detail; every coordinate a command takes or prints is in this
  image's pixels, so the agent taps what it sees.
- **`inspect <m>`** prints the screen's UI tree, every window, system
  alerts and the keyboard included, each element with a reference, `e7`, its
  role, text or label, value, identifier, frame and state: the
  [browser's tree](../browser/browser.md#page-trees-and-references) for a
  screen. The software keyboard is one element, *Keyboard*, with its frame,
  unless `--keyboard` lists its keys: a full keyboard is 184 elements and
  90 KB, which the agent rarely needs, since `type` types.
- **`tap`, `swipe`, `type`, `key`, `rotate`, `appearance`** act and return once the
  device took the input; they do not wait for the screen to change. An
  element is a reference from the last `inspect`, resolved to the centre of
  its frame. `type --replace` sets a field's text at once, as the
  accessibility interface does, where typing would add to it. Text a US
  keyboard cannot type, such as *Grüße 東京*, the user's own typing in the
  view included, an input method's Chinese too, goes in as each platform
  allows: on Android at the cursor through the emulator's clipboard and a
  paste, the clipboard put back as it was; on iOS the focused field's value
  becomes its value with the text added at its end, through the
  accessibility interface `--replace` uses, since a simulator from Xcode
  26.3 on takes no text into its pasteboard from outside (`simctl pbcopy`
  leaves it empty).
- **`wait`** waits until text or an element appears or an element is gone,
  re-reading the tree, 30 s by default, as `demi browser wait` does; it is
  how the agent knows that something it started has finished, such as a
  page that loads.
- **`record <m> start`** starts recording what the device shows;
  **`stop`** ends it and writes the MP4 to stdout. The panel has the same
  button for the user.

**A settled picture.** An input returns before the screen has changed:
measured on both platforms, a screenshot taken at once still shows the
screen before a tap. So `screenshot` and `inspect` first wait until the
picture stops changing, no new frame from the device for 300 ms, at most
1.5 s, and then take it; both drivers send a frame only when the screen
changes, so a still screen answers at once after the 300 ms; a screen still changing then, a video or a spinner, is taken as it
is, and the command says so on stderr: `demi mobile screenshot: the screen
was still changing after 1.5 s`. The live view's frames tell it at no cost.
Neither platform's own idle signal is used: both report idle while an app
draws its own animation, which the iOS and Android probes showed.

## The live view

Each device the agent opens is a tab in the work panel beside the
browser's tabs, streamed live, and the user operates it with the mouse or, on
a phone, the touch screen: a click is a tap, a drag a swipe, the keyboard
types. While the agent uses the device, from the start of its first
`demi mobile` command on it until three seconds after its last one ends,
a `wait` or an `inspect` included, the view shows *The agent is using this
device*, and it marks where the agent taps; the panel's own controls are
the user's, which the call's origin tells, and show neither; the user's input and
the agent's reach the device in the order they arrive.

| The panel | Its command |
| --- | --- |
| A device's tab, which appears when the agent opens the device; the user switches among the open devices by their tabs, and a device's toolbar starts with its name where the whole name fits; on a phone, whose tab already names the device, the toolbar holds only its controls. The panel opens no device of its own: a device is opened by the agent, which knows what the task needs | `open` |
| A device's close button; a device that closes otherwise, its Host's runner restarting or a Cloud stopping, or that is lost, loses its tab the same way, and the agent opens it again when the task needs it | `close` |
| Rotate Left, Rotate Right | `rotate` |
| Toggle Appearance | `appearance` |
| Home, Lock, Volume Up, Volume Down, and on Android Back | `key` |
| Screenshot, Record | `screenshot`, `record` |

The device in the panel answers as the device on the desk does:

- **The toolbar acts at once.** Home, Lock, the volume keys, Back, the
  rotations and Toggle Appearance send their call on the press and wait for
  nothing: the picture is the answer, as on a remote, and the button never
  shows a spinner. A call that fails says why in a toast. On a panel too
  narrow for every control, the controls that do not fit move, from the
  end, into a More menu at the toolbar's end, as a macOS window's toolbar
  does; on a phone the menu opens as a sheet. Lock is the side button: on
  Android the Host sends the system's power key, which the emulator's own
  power button does not reach on recent Android versions. These calls are
  methods bound to the package's operations, so on a paired device they go
  over the direct channel ([Calling its command package](../architecture/plugins.md#calling-its-command-package)).
- **The user's input never waits for the agent.** A tap, a swipe or a key
  from the panel reaches the device as it arrives, between the steps of an
  agent's command on the same device: a command that reads or waits,
  `inspect`, `wait`, `screenshot` or `record`, holds nothing the user's input
  waits for, and the helper takes input apart from its other work. Input
  keeps its own order, and an agent's tap and the user's land in the order
  they arrive.
- **The input method's window stays where the user typed.** The candidate
  window of an input method, Chinese pinyin for example, opens at the point
  of the user's last tap on the picture, where the field is, not at the
  panel's corner.
- **A screen that is off says so.** When the device's screen turns off, by
  Lock, by the agent's `key lock` or by the device's own sleep, the picture
  dims under *Screen Off* with Wake, which turns the screen on and never
  off; any tap or key does the same. An Android emulator goes on sending
  its dark screen, which is what shows under the notice; a simulator sends
  nothing, and its last frame does. The Host tells the page as the screen
  changes: Android logs each change of the screen as an event, which the
  Host reads from the emulator with its state when the view starts, and a
  simulator, which reports none, is off from the Lock the Host pressed
  until the next input. Wake on Android is the wake key, which only turns
  the screen on; on a simulator, whose off state is the Host's own, it is
  the side button.
- **Pictures at the panel's size.** The Host scales the device's screen to
  the size the panel shows it at, in device pixels, before it encodes, and
  starts again at the native size only when the panel grows to it: a
  1206 × 2622 iPhone in a panel 430 pixels wide is encoded at that width,
  a fifth of the bytes.

The picture sits in the device's frame, as Simulator.app shows it. On a
Mac the frame is the one Xcode installs for the device's model: the
device type names it (`chromeIdentifier` in its profile), a bundle in
`/Library/Developer/DeviceKit/Chrome` holding the bezel as PDF pieces and
the places of its buttons, and the screen's rounded corners are the device
type's framebuffer mask. The helper draws the frame once for each device
and size the view asks for, as DeviceKit's own renderer draws it, which
DeviceHub shows: the frame is sized from the screen in points, the pixel
size divided by the device's scale; each piece takes the size of its own
artwork, scaled and floored to whole points, the edges overlapping the
corners by two points at each end, where DeviceKit overlaps them by one,
so that the faded end of an edge's artwork lies wholly under the corner
at any screen scale; everything is drawn into one image with no
piece clipped, the screen, then the edges, then the corners: DeviceKit
draws the corners before the edges, and an edge's last column of artwork,
half transparent, then lies over the corner's black as a thin grey line, so
Demi draws the corners last to cover it; the screen's
artwork fills only the framebuffer's rectangle; and each button is placed
outside the frame from its anchor and alignment by its artwork's size less
its travel, unrotated, then turned with the frame. The view lays the
picture into its screen.
A device with no frame installed, an Android emulator or a model Xcode
draws none for, gets a frame the view draws itself: a thin outline of the
device around the screen, with the screen's corner radius and the
buttons as short marks on its sides, as a wireframe sketch. The frame
turns with the picture. Its buttons are drawn, not pressed: the toolbar
presses them.

Rotation and appearance are actions in the panel, as in Simulator.app's
own menus (*Rotate Left*, *Toggle Appearance*), never a switch that shows
the state: the agent may change either with `simctl` or `adb` as well, and
a switch would have to ask the device again and again to stay true. The
command names the state the agent wants, `appearance m1 dark`, or the
panel's action, `left`, `right` or `toggle`, which a quarter turn from the
orientation `demi-mobile` set, or the appearance read once, resolves. The
panel's Screenshot and Record save the picture or the MP4 on the user's
computer, as the browser view's download does: the file is written first
into the conversation's scratch directory, `mobile/m1-<time>.png`, which no
change view shows ([Scratch directory](../execution/runner.md#scratch-directory)),
and removed when the device closes.

The view is the browser's [live view](../browser/live-view.md) with a
different source. Both directions are shared:

- **Pictures:** the user stream, relay and direct channel, framing, H.264
  delivery, key frames on request, rate adaptation and the page's decoder.
- **Control:** the input messages, pointer and touch with their buttons and
  contacts, wheel, keys and text, in the picture's coordinates; the order in
  which the user's input and the agent's reach the source; keys and buttons
  still held released when a viewer leaves; and the agent's activity, which
  the view shows as *The agent is using this device* and as the mark where
  it acted. A source turns the messages into its own input, CDP events for
  a tab, hardware events for a simulator, and states what it takes, such as
  touch or a mouse, so the page sends that.

Only what a source alone has stays with it: the browser's tabs, address
bar, dialogs, clipboard and downloads; the simulator's frame and its
buttons. What the browser view
built for Chrome tabs that is not about a browser moves to where both
plugins use it, the Host side next to the native runtime's user streams and
the page side in `web-ui`, exported to plugins through `@demicodes/plugin-sdk`
([Plugin pages](../architecture/plugin-pages.md)); the browser view then
uses it as the mobile one does, and works as before.

## Drivers

Each platform has one driver on the Host, behind one interface: frames out
as H.264, input in, the UI tree, the screen size and orientation, boot and
shutdown.

### iOS

A small helper, `demi-ios-sim`, links Meta's FBSimulatorControl (MIT), the
library under `idb`, as Claude Desktop's simulator pane does. It travels
inside `demi-mobile`, the plugin's command program, on macOS: the build
compiles it with the guest bridge and idb's license notice and embeds them,
and `demi-mobile` unpacks them on first use into
`<installation>/mobile/<sha256>/`, checked against the digest it carries, so
neither the runner nor the backend knows of it. `demi-mobile` starts one
helper for each conversation that opens an iOS device and speaks to it over
its standard input and output.

- **Frames** come from the simulator's framebuffer, encoded with
  VideoToolbox as H.264 High, Annex B, real time: measured at 60 fps and
  about 8 ms a frame, 3 % CPU on a still screen, where the simulator sends
  no new frame. Bit rate and key frames follow the live view's rate control
  while it runs.
- **Input** goes to the simulator as hardware events through the library,
  which picks the simulator's transport for the installed Xcode.
- **The UI tree** comes from the simulator's accessibility service, which,
  asked to scan remote content, also reads a web view an app embeds, its
  headings, fields and buttons, in about 270 ms. Safari's own page it does
  not reach: when that read finds no web content and Safari is the
  frontmost app, `inspect` asks the library's guest bridge, built from the
  same release's source and installed into the simulator then, and puts
  the page's elements where the page is, about 1.2 s. The bridge turns on the simulator's UI
  automation mode; Demi records the mode before and restores it when it
  closes the device.
- **No window and no permission.** The helper needs neither Simulator.app
  nor Screen Recording nor Accessibility, as measured from a background
  process with none of them.
- **Rotation.** The simulator turns its screen but not its framebuffer,
  which stays portrait with its picture sideways, and refuses to say its
  orientation on the Xcode versions measured. So `demi-mobile` sets
  portrait when it opens a device and remembers each orientation it sets
  since, and the helper turns every picture upright before encoding it and
  before a screenshot: a landscape iPhone is 2436 × 1125 in the view, in a
  screenshot and in every coordinate. A user who rotates the device in
  Simulator.app's own window leaves it sideways in the view until the next
  `rotate`.

### Android

- **Frames and touches** come from the emulator's own gRPC interface, the
  one Android Studio's embedded emulator uses, with no adb in the path; a
  touch goes as a finger's, never a stylus's, so the keyboard's handwriting
  prompt does not take it. An
  emulator Demi starts runs without a window, `-no-window -gpu host`, with
  JWT authentication, which also keeps the interface on localhost; without
  it the interface is open to every network the computer is on. The Host
  encodes the frames with OpenH264 on every system, Mac included, Cisco's
  own build of it, which the runner downloads from Cisco the first time an
  Android view opens and checks against its pinned hash, as Firefox does:
  Cisco pays the H.264 patent royalties for its binary only, not for a
  build Demi would make and ship from the source; measured
  at 78 fps for a 1080 × 2400 screen, more than the 60 the emulator sends:
  the iOS helper's VideoToolbox encoder takes its pictures inside the
  helper, which needs Xcode, so a Mac with only Android Studio could not
  use it, and a second VideoToolbox encoder for Android alone would be the
  same thing written twice.
- **Typing and keys** go as a hardware keyboard's, the emulator's gRPC key
  events with USB HID usages, as on iOS, so text lands at the cursor and the
  emulator's clipboard is never touched; `type --replace` sets the focused
  field's text through the helper below. A touch goes to the emulator in
  its panel's own portrait coordinates, as a finger on the glass does,
  whatever the display shows: a point on the picture is turned back by the
  picture's rotation. An element's box, which the helper gives in the
  display's coordinates, is first turned into the picture's by the
  display's rotation, read from the helper as the command runs; an element
  tap whose rotation cannot be read fails and sends nothing.
- **The UI tree** comes from a helper Demi runs inside the
  emulator, as a shell process started over adb, `app_process`, not an
  installed app: it holds one UI automation connection and reads every
  window, the keyboard and system UI included, in about 55 ms, where
  `uiautomator dump` takes seconds and leaves the keyboard out. The helper
  is openatx's `u2.jar` (MIT, from `openatx/android-uiautomator-server-jar`),
  pinned to a version and shipped with the
  runner. One UI automation connection exists at a time, so a test runner
  such as Appium started on the same emulator replaces it; while it holds
  it, a command that needs the tree or the display's rotation fails saying
  so and sends nothing, and Demi takes the connection back only when the
  other client has let it go.
- **Coexistence.** The agent's own `adb`, `emulator` and Gradle commands
  work on the same emulator at the same time, as measured.

## Lifetime

- A device Demi booted shuts down when its conversation closes it, is
  archived or deleted, or an hour after nobody watches it and no
  command used it. A device the user booted is never shut down; closing it
  only detaches it.
- `demi-mobile` ending, as when its runner stops, updates or is killed,
  shuts nothing down, so an update never ends the app the agent was
  testing. It keeps the simulators Demi booted in
  `<installation>/mobile/booted.json`, writing it as it boots or shuts one
  down, and a new `demi-mobile` reads it as it starts: each listed
  simulator still booted gets the hour without a watcher or a command from
  then, as if its conversation had just left it, unless a conversation
  opens it again first; one that is shut down already leaves the list. A
  simulator the user shut down and booted again in between is the user's,
  which the simulator's boot time, later than the record's, tells.
- A paired device going offline ends its devices' views as it ends the
  browser's; the device stays where it was.
