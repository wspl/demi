---
status: proposed
---

# Shortcuts come in two sets, one for a Mac and one for a PC

## Problem

Alex opens Demi in Chrome on Windows and presses Ctrl+K to search. Nothing
happens: every shortcut is written in the Mac's notation (`⌘K`), and ⌘ is
matched as the Meta key on every system, which on Windows is the Windows
key. Win+K is the system's own, so the shortcut never runs, and Settings ›
Keyboard shows `⌘K` on a keyboard that has no ⌘. The account keeps one set
of shortcuts, so a shortcut changed on a Mac would have to mean something
on a PC too.

## Decision

Demi has two sets of shortcuts: the Mac's, on macOS, and the PC's, on
Windows and Linux. A page uses the set of the system it runs on. Each set
has its own defaults, and the user's changes to one leave the other as it
is.

| | Mac | PC (Windows, Linux) |
|---|---|---|
| Modifiers | ⌃ ⌥ ⇧ ⌘ | Ctrl, Shift, Alt |
| Shown as | `⇧⌘O` | `Ctrl+Shift+O` |
| New conversation | ⇧⌘O | Ctrl+Shift+O |
| Search | ⌘K | Ctrl+K |
| Sidebar | ⌘\ | Ctrl+\ |
| Settings | ⌘, | Ctrl+, |

* On a PC, a shortcut needs Ctrl, as a shortcut on a Mac needs ⌘ or ⌃:
  without it, the keys type. The Windows and Super key is not part of any
  shortcut: the recorder does not take it, as the system keeps it.
* Every place that shows a shortcut, Settings › Keyboard and its recorder,
  menus and tooltips, shows the set of its system, in its notation, and
  the keys the user set where the user can set them.
* A key or a click that holds the command key holds ⌘ on a Mac and Ctrl
  on a PC, as in Finder and Explorer: a click that adds a conversation to
  the selection, one that opens a link in the composer, ⌘⏎ and Ctrl+Enter.
  A Mac's Control-click stays a right-click.
* The app's fixed keys are written for each set: the file browser goes up,
  back and forward with ↑, ← and → and ⌥ or ⌘ on a Mac, and with Alt on a
  PC, as in Explorer.
* The composer's own keys, such as bold, use ⌘ on a Mac and Ctrl on a PC,
  as today.

The design to build from:

* `Preferences.shortcuts` becomes `ShortcutSets`:

  ```rust
  pub struct ShortcutSets {
      pub mac: Shortcuts,
      pub pc: Shortcuts,
  }
  ```

  and the patch, `shortcuts: { mac?: ShortcutsPatch, pc?: ShortcutsPatch }`.
* A shortcut is stored in its set's notation: the Mac's as today (`⇧⌘O`);
  the PC's as it is shown, the modifiers in the order Ctrl, Shift, Alt,
  joined by `+` with the key (`Ctrl+Shift+O`).
* `ui/shortcut.ts` owns both notations: which set a page uses, by the
  system it runs on, how an event is written in it, and how a shortcut is
  shown. Matching compares the event, written in the page's set, to the
  stored shortcut. Every fixed key and every command-key test of the app
  goes through it too; a plugin's page, which does not reach it, asks
  `clientPlatform`, the same test it makes.
* The database moves the stored shortcuts to the Mac's set, since all of
  them are written in its notation; the PC's set starts at its defaults.

## Spike

None.

## Prior art

* VS Code's Settings Sync keeps the keyboard shortcuts of each operating
  system apart by default, `settingsSync.keybindingsPerPlatform`
  ([Settings Sync][sync]).

[sync]: https://code.visualstudio.com/docs/configure/settings-sync
* VS Code shows a shortcut as symbols on macOS and as `Ctrl+Shift+Alt+K` on
  Windows and Linux, modifiers in that order
  ([keybindingLabels.ts](https://github.com/microsoft/vscode/blob/main/src/vs/base/common/keybindingLabels.ts)).

## Alternatives

* **One set, ⌘ read as Ctrl on a PC.** ⌃ then has no key left on a PC but
  the Windows key, and a change made on one system changes the other.

## Open questions

None.
