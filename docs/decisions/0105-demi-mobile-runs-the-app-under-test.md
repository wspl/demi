---
status: proposed
---

# `demi mobile` installs, launches and reads the app it tests, and erases a device

## Problem

Alex asks the agent to check the analytics events an app sends on an
Android emulator and an iPhone simulator. `demi mobile` drives the screen;
everything else the agent does with the platforms' own tools, as the
package's design leaves it to them:

- It installs with `adb install` and `simctl install`, and launches with
  `monkey`, which fails for some apps, so it taps the app's icon on the
  home screen instead.
- It follows the app's log with `adb logcat --pid=<pid> > file &`, started
  anew at each launch since the pid changes, and on iOS with `simctl launch
  --console-pty > file &` to see what the app prints. Seven of these ran in
  the background; one ran for three hours, wrote 1.2 GB, and left the disk
  with 3 GB free.
- Alex asks for a fresh device, so the app gets a new device ID. The agent
  deletes the AVD and makes another with `avdmanager`, which made one
  without a hardware keyboard (decision 0094).

## Decision

`demi mobile` takes four new commands. The package's design changes from
leaving the app to the platform's tools to running the app under test, as
a person does with Xcode or Android Studio: building stays with the
project's tools. This replaces the line of the package's entry comment
that leaves installing, launching an app and logs to the platforms' tools.

```text
$ demi mobile install m1 build/app-debug.apk
Installed com.example.app 2.3.0 on m1.

$ demi mobile launch m1 com.example.app --restart
Launched com.example.app on m1, in front.

$ demi mobile logs m1 --app com.example.app --filter checkout
12:04:31.120 I Analytics: event checkout_started
12:04:31.452 W Payments: card form shown without a saved card
Cursor: 65e9a9ae0a0ad:1842

$ demi mobile erase m1
Erased m1 to its factory state and started it again; nothing of before is
on it, the apps installed included.
```

### install

`install <device> <file>` installs an `.apk` on an emulator, or an `.app`
on a simulator, replacing an installed version and keeping its data, as
`adb install -r` and `simctl install` do. It answers the app's id, read
from the file, which `launch` and `logs` take.

### launch

`launch <device> <app id>` brings the app in front, starting it if it is
not running, as a tap on its icon does. `--restart` stops it first, so it
starts from the beginning. It returns once the app's first screen shows.
On Android it starts the app's launcher activity (`am start -W`), which
needs no icon on the screen, and returns once its first screen is drawn.
On a simulator it starts the app with its console (`simctl launch
--console-pty`), so what the app prints goes to the log, and waits until
the app is in front with an element on its screen, which its launch screen
has none of, for up to 10 s, as long as a command waits for an element;
`launch` says when the first screen has not shown by then, as for a game
whose screen has no accessible element, and fails when the app ended as
it started. An app that runs already and is only brought in front keeps
the console it has, and `launch` says when the log does not hold what it
prints. A line the app prints is cut at 4 KB, as Android's log cuts an
entry, so an app that prints without a newline fills only the log's 16
MB.

### logs

From the moment a device opens until it closes, Demi follows its log:
Android's `logcat`, every level; the simulator's unified log at the
default level. An app Demi launched on a simulator adds its own debug and
info messages, those of its bundle's code, and what it prints to standard
output and error, which the unified log does not hold.

`logs <device>` reads the entries after `--after <cursor>`, the cursor the
last read printed, as `demi browser logs` does, or the last ones without
it. `--app <app id>` keeps the app's entries, across its restarts: on
Android those of its user id, on iOS those of its executable. `--filter
<text>` keeps those whose text or tag holds the text. `--level` keeps those
at or above a level: verbose, debug, info, notice, warning, error or
fault, Android's levels and the unified log's types in one order, the
unified log's default level being notice, as Swift's `Logger` names it.

Each entry has its time, level, process id, Android's user id or the iOS
process's executable, a tag (Android's tag, the unified log's category, or
`console` for what an app prints) and its text. The text form is a line
per entry, its control characters escaped:

```text
12:04:31.120 N ExampleApp Analytics: event checkout_started
```

The cursor names the log it belongs to, so the cursor of a device opened
again is refused rather than read as a position of the new log. The
buffer and its cursors are the ones `demi browser logs` uses, moved to
`demi-shared-types` so both read one implementation.

Demi keeps the last 16 MB of each device's log in memory, about half an
hour of a simulator or an emulator with an app that logs heavily; a read
whose cursor fell behind that says how many entries were dropped
(`dropped`). Nothing goes to the disk, and the following ends as the
device closes. A connection that ends and is made again keeps the log;
the program's end loses it.

### erase

`erase <device>` returns the device to its factory state, as Simulator's
Erase All Content and Settings and Android Studio's Wipe Data do: every
app and its data, the settings and the device's identifiers are gone. Demi
shuts the device down, erases it (`simctl erase`, or a boot with
`-wipe-data`), and starts it again; it keeps its handle and its log, and
its view says it reconnects meanwhile, and no conversation opens the
simulator until the erase is over, even once the device was closed. It took 25 s for a simulator and
58 s for an emulator. A recording of the device ends. Whoever booted the
device still has it booted: one Demi booted shuts down when it closes, one
the user booted stays booted, though an emulator the user started with a
window comes back without one, as Demi boots every emulator.

## Spike

- **Android launch.** `cmd package resolve-activity` names an app's
  launcher activity, and `am start -W` returned in 0.34 s once its first
  screen showed.
- **Android app log.** `logcat --uid=<the app's uid>` kept the app's
  entries across restarts, where `--pid` follows one process.
- **Log volume.** An idle emulator logs about 1 KB a second; an idle
  simulator 4 KB at the default level, and 900 KB at the debug level while
  an app launches, which is why only an app Demi launched is followed at
  the debug level.
- **Erase.** A booted simulator took 33 s to shut down, erase and boot. An
  emulator took 54 s to boot with `-wipe-data`, and had a new
  `android_id`.
- **iOS launch.** `simctl launch --console-pty` passes the app's output on
  at once, but writes its own line naming the process only as it ends,
  buffered into a pipe; the launch reads the app's new process from
  `launchctl list` instead, and returned in 1.8 to 2.9 s. The app's own
  debug messages are followed from before it starts, once `log stream`
  says it follows the log, or its first ones are lost.
- **Ending a log stream.** `simctl spawn` hands a SIGTERM on to the `log
  stream` inside the simulator; one whose simctl is killed keeps running
  until it next writes, which a stream of one app's messages may never do.

## Prior art

- [Maestro](https://docs.maestro.dev/api-reference/commands/launchapp)
  installs nothing itself; `launchApp` launches, with `clearState` for an
  app's data and `stopApp` to restart.
- [Appium](https://appium.io/docs/en/latest/) has `installApp`,
  `activateApp` and `terminateApp`, and `getLogs` for `logcat` and the
  simulator's syslog, read since the last read, as Demi's cursor does.
- `demi browser logs` reads a tab's console after a cursor; `demi mobile
  logs` takes the same options, so an agent that knows one knows the other;
  its `--level` is a least level, as `logcat` and `log stream` take one,
  where the browser's lists the console's levels.

## Alternatives

- **Leaving logs to `adb logcat` and `simctl spawn log stream`.** That is
  today, where every follow is a process the agent must stop, and a file
  that grows without end.
- **Writing the log to a file.** Survives a long read, and fills the disk
  as the session's file did; 16 MB in memory holds what an agent reads.
- **Clearing one app's data instead of erasing.** `pm clear` and
  uninstalling keep the device's identifiers, which is what Alex wanted
  gone.
- **Letting go first of the entries of the process that holds the most.**
  One simulator's Fitness app logged about 940 entries a second in a
  loop, where three other simulators logged about 30, and the 16 MB held
  about a minute. Keeping the newest entries whatever process wrote them
  is what Android's own log buffer does, and `logs` says how many went.
- **Making a new simulator for each erase.** A new device in the user's
  list each time, and on Android the hardware choices of `avdmanager`.
