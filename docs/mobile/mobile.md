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

- **Simulators only**: the iOS Simulator, on a Mac with Xcode, and the
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
demi mobile rotate <m> portrait|landscape
demi mobile appearance <m> light|dark
demi mobile wait <m> --text <text> | --element <element> | --gone <element> [--timeout <s>]
demi mobile record <m> start | stop
```

The verbs are the browser's where the meaning is the same
([Command reference](../browser/browser.md#command-reference)), so an agent
that uses `demi browser` already knows them.

- **`list`** prints every simulator of the Host it runs on, iOS and Android
  together, as the panel's device menu shows them; on an attached Host it
  runs inside `demi host shell`, as every command does: name, platform, model, OS version, state (shut down, booted,
  open in this conversation as `m1`, or open in another one), and its
  `simctl` id or AVD name.
- **`open <device>`** takes a name, a `simctl` id or an AVD name, boots the
  device when it is shut down, without a window, attaches it to the
  conversation and opens it in the panel. It answers the handle, `m1`, the
  screen size and the native id. Handles are the conversation's, across
  its Hosts, so `m2` may be an Android emulator on a Linux workstation while
  `m1` is an iPhone on the Mac. A conversation has at most four devices
  open; a device open in another conversation is refused, naming it, so two
  conversations never drive one device.
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
  accessibility interface does, where typing would add to it.
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

Each open device is a tab in the work panel beside the browser's tabs,
streamed live, at most four, and the user operates it with the mouse or, on
a phone, the touch screen: a click is a tap, a drag a swipe, the keyboard
types. While the agent's input is being delivered the view shows *The
agent is using this device* and marks where it tapped; the user's input and
the agent's reach the device in the order they arrive.

| The panel | Its command |
| --- | --- |
| A device menu: the simulators of the conversation's Host and of its attached Hosts, grouped by Host, each with its OS version, whether it is booted and whether this conversation has it open; picking one opens it, booting it when it is shut down | `list`, `open` |
| A device's tab, and its close button | `open`, `close` |
| Rotate Left, Rotate Right | `rotate` |
| Toggle Appearance | `appearance` |
| Home, Lock, Volume Up, Volume Down, and on Android Back | `key` |
| Screenshot, Record | `screenshot`, `record` |

Rotation and appearance are actions in the panel, as in Simulator.app's
own menus (*Rotate Left*, *Toggle Appearance*), never a switch that shows
the state: the agent may change either with `simctl` or `adb` as well, and
a switch would have to ask the device again and again to stay true. The
command names the state the agent wants, `appearance m1 dark`, and the
panel's toggle reads the state once when the user presses it and sets the
other.

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
bar, dialogs, clipboard and downloads; the simulator's device menu and its
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
library under `idb`, as Claude Desktop's simulator pane does. It is built
for macOS with the runner and ships beside it, and the runner starts one per
conversation that opens an iOS device, speaking to it over its stdin and
stdout.

- **Frames** come from the simulator's framebuffer, encoded with
  VideoToolbox as H.264 High, Annex B, real time: measured at 60 fps and
  about 8 ms a frame, 3 % CPU on a still screen, where the simulator sends
  no new frame. Bit rate and key frames follow the live view's rate control
  while it runs.
- **Input** goes to the simulator as hardware events through the library,
  which picks the simulator's transport for the installed Xcode.
- **The UI tree** comes from the simulator's accessibility service; a web
  view's content needs the library's guest bridge, built from the same
  release's source and installed into the simulator when first needed,
  which reads Safari's page whole. The bridge turns on the simulator's UI
  automation mode; Demi records the mode before and restores it when it
  closes the device.
- **No window and no permission.** The helper needs neither Simulator.app
  nor Screen Recording nor Accessibility, as measured from a background
  process with none of them.
- **Rotation** is set without reading it back, as the simulator refuses the
  read on the Xcode versions measured; the next frame's size tells it.

### Android

- **Frames and touches** come from the emulator's own gRPC interface, the
  one Android Studio's embedded emulator uses, with no adb in the path; a
  touch goes as a finger's, never a stylus's, so the keyboard's handwriting
  prompt does not take it. An
  emulator Demi starts runs without a window, `-no-window -gpu host`, with
  JWT authentication, which also keeps the interface on localhost; without
  it the interface is open to every network the computer is on. The Host
  encodes the frames, VideoToolbox on a Mac and OpenH264 elsewhere,
  measured at 171 and 78 fps for a 1080 × 2400 screen.
- **The UI tree, text and keys** come from a helper Demi runs inside the
  emulator, as a shell process started over adb, `app_process`, not an
  installed app: it holds one UI automation connection and reads every
  window, the keyboard and system UI included, in about 55 ms, where
  `uiautomator dump` takes seconds and leaves the keyboard out. The helper
  is openatx's `u2.jar` (MIT, from `openatx/android-uiautomator-server-jar`),
  pinned to a version and shipped with the
  runner. One UI automation connection exists at a time, so a test runner
  such as Appium started on the same emulator replaces it; the next command
  says so.
- **Coexistence.** The agent's own `adb`, `emulator` and Gradle commands
  work on the same emulator at the same time, as measured.

## Lifetime

- A device Demi booted shuts down when its conversation closes it, is
  archived or deleted, or an hour after nobody watches it and no
  command used it. A device the user booted is never shut down; closing it
  only detaches it.
- A paired device going offline ends its devices' views as it ends the
  browser's; the device stays where it was.
