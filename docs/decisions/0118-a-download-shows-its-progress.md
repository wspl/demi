---
status: proposed
---

# A download shows its progress

## Problem

On a laptop without Chrome, Alex opens a page in the browser panel. The panel
shows "Starting the browser…" with a spinner, then "Couldn't open this page.
The browser couldn't start." with Retry. The agent fails the same way: its
first `demi browser tab new` fails, it runs `demi browser install`, and it
tries again a minute and a half later. Signing in to Claude Code, or installing
it in Settings, shows a spinner for the whole of the CLI's download, which can
take minutes on a slow link.

The runner's artifact cache knows how each download goes, but only the command
that started it hears. A page never does.

## Decision

Software a package installs only when it is needed, Chrome and the Claude Code
CLI, stays on demand. Every view that waits for one of its downloads shows how
far the download came, whoever started it. A package program a runner update
could not bring ahead of time shows the same way.

### What the user sees

The browser panel, while the Host downloads Chrome or the browser's program:

```text
+----------------------------------------------+
|                                              |
|           Downloading the browser…           |
|           [==========-----------]            |
|               74 / 156 MB                    |
|                                              |
+----------------------------------------------+
```

- The bar and its sizes keep their room in every starting phase, empty when
  nothing downloads, so the label never moves.
- A tab the user opens in the panel on a Host without Chrome installs Chrome
  and then opens its page, instead of failing.
- An agent's `demi browser tab new` or `demi browser content fetch` on a Host
  without Chrome installs it first and prints the install's progress lines on
  standard error, as `demi browser install` prints them on standard output.
  The command's own timeout starts once Chrome is in place.
- The mobile panel, while its stream waits for the Host to download the
  plugin's program, says "Downloading…" with the sizes in place of
  "Connecting to the simulator…".
- Settings' Claude Code install row shows the download's bar and sizes in
  place of its spinner once the first bytes arrive. The sign-in dialog does
  the same in its installing stage.

### Design

- The runner's cache keeps the downloads in progress in memory and reports
  them to the backend as `Outbound::Downloading`, when the list changes, at
  most four times a second, and again when the connection comes online:

  ```rust
  pub struct HostDownload {
      pub package: String,
      /// The artifact's line, as `HostArtifact::name`.
      pub name: String,
      pub version: String,
      pub received: u64,
      pub size: u64,
  }
  ```

- The backend's link holds the last list, in memory only, and a device's DTO
  carries it as `downloading`, beside `installed`. A plugin page reads its own
  packages' downloads on the conversation's primary Host:

  ```ts
  // ConversationPlugin
  readonly downloading: ComputedRef<readonly HostDownload[]>
  ```

- `RegionStatus` takes an optional `progress: DownloadProgress | null`; null
  keeps the bar's room empty.
- The browser resident installs Chrome for an operation that starts the
  conversation's browser before the operation's time starts. The install is
  tied to the invocation, so a cancelled command stops it.
- The Claude Code machine belongs to the instance's master, so a user's pages
  do not see its device. While the backend installs the CLI there, it watches
  that machine's downloads of `demi.claude-code` and reports them as a new
  `StartStage::Download`. The backend carries them as `download` on
  `CliInstall::Installing` and on a pending login. Both are null before the
  first report.
- An older runner sends no `Downloading`: its views show the spinner, as
  before. An older page ignores the new fields.

## Spike

None.

## Prior art

- VS Code shows an extension's download progress in the extension's own view,
  wherever the install started.
- Chrome downloads its large optional components on demand and shows their
  progress where they are needed.
- Playwright's `install` prints a progress bar per browser; its library
  installs nothing on first use, which is what made the agent fail here.

## Alternatives

- **Download Chrome on every Host ahead of time.** About 160 MB on each Host,
  most of which never open a browser.
- **Keep failing and offer Install.** Every first use would be a failure the
  user or the agent has to understand and recover from.
- **Report progress through the command that started the download only.**
  The panel and Settings would still show a spinner for an install the agent
  or another page started.

## Open questions

None.
