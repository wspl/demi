---
status: proposed
---

# The user opens a simulator from the taskbar

Supersedes: nothing recorded. It ends a rule that only the mobile plugin's
code said: the panel opens no device, the agent does.

## Problem

Alex wants to try the app on an iPhone 17 Pro next to the conversation. The
taskbar's Open (+) lists Simulators, but choosing it opens an empty window:
a simulator tab appears only when the agent runs `demi mobile open`. Alex
has to ask the agent to open the device, and wait a turn for it.

## Decision

Simulators in Open has a submenu of the conversation's device's simulators
and emulators, as Simulator.app's File > Open Simulator lists them. Choosing
one opens it in the Simulators window, booting it first when it is shut
down, as `demi mobile open` does.

```text
+  ──────────────────────
   Browser       New Tab
   Simulators          >  ┌ iOS 26.5 ─────────────────┐
   Changes                │ iPhone 17 Pro      Booted │
   Files                  │ iPad Air 13"              │
                          ├ Android 16 ───────────────┤
                          │ Pixel 10           Open ✓ │
                          └───────────────────────────┘
```

- The rows group by system, newest first, and read as `list` gives them:
  the name, the model where it differs, and the state: Open for one this
  conversation has, Running for one that is booted. One another
  conversation has open is disabled, saying so.
- The submenu reads the list each time it opens, with the menu's own
  loading row until it comes, at one width in both states. A platform the
  device cannot run simulators of shows as a disabled row of its own, with
  the device's reason, such as no Xcode, as its tooltip.
- Choosing a row closes the menu and shows the window at once. While the
  simulator opens, the window says Opening iPhone 17…, or Starting iPhone
  17… for one that was shut down, over whatever tab it shows; a cold boot
  took 69 s on a test Mac. Once the device is open its tab comes and is
  selected. Choosing one that is open already selects its tab.
- A refusal, such as a device that is offline, is a toast; a window the
  opening left empty closes.
- On a phone, the Tabs list's Simulators row has the same submenu, as a
  sheet.

The design to build from:

- **Plugin.** The mobile page gains two methods, each the operation of the
  command that does the same, as every other control is:
  `list {}` returns `ListResult` (`CallKind::Looks`); and `open { device }`,
  `device` being the native id `list` gives, runs `open`
  (`CallKind::Starts`, so it wakes a stopped Cloud as opening a browser tab
  does) and returns `Opened { device, tab }`, the tab's id being the
  plugin's own. Opening changes the conversation's devices, so the plugin's
  sync adds the tab, as it does for the agent's open.
- **Kinds.** `PanelKind` and `PanelTabKind` gain two components, each with
  the `conversation` prop:

  ```ts
  /** Its window's rows in the taskbar's Open, as a submenu. Emits `opened`. */
  openMenu?: Component
  /** What its window shows over its tabs while the kind opens one. Emits `failed`. */
  opening?: Component
  ```

  `WorkWindow.offers` names the window's kind that has `openMenu`, and
  `TaskbarOpen.menu` carries it.
- **Windows.** A window of kinds that are not pinned stands while it holds
  a tab, as today, or while it awaits one: `opened` adds it to the windows
  that await, and the first tab, `failed` or the user's close removes it.
- **Page.** The mobile store's controller keeps the simulators the user
  opens until their tabs are there, and selects each tab with
  `page.panel.select` as the open answers.
- **Older parts.** A runner without the page methods answers the call as
  an unknown method; the submenu's row says the list could not be read,
  with the reason as its tooltip.

## Spike

None: both operations exist and the agent uses them; the page methods only
route to them, as Close and Rotate do.

## Prior art

- Simulator.app: File > Open Simulator, a submenu by runtime, then devices.
- Android Studio: the device picker in the toolbar lists AVDs and connected
  devices, and starts an AVD when it is chosen.
- VS Code's Flutter and React Native extensions: a device picker in the
  status bar that boots the chosen emulator.

Demi departs in where the device shows: in the conversation's Simulators
window, not a window of the system's.

## Alternatives

- **A device picker inside the empty Simulators window.** It costs a click
  more, and the window would hold a state no other window has.
- **Keep opening devices to the agent.** Alex waits a turn for what one
  click does.

## Open questions

None: it lands as a package of the 0308 series.
