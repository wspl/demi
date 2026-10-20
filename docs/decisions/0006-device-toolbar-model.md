---
status: accepted
---

# A device's toolbar shows its name and its model

## Context and Problem Statement

The device panel's toolbar shows a simulator or an emulator by its name
alone, such as *pixel10 api36* or *Work iPhone*. The name is whatever the user
or a tool called the device, so it often does not say which phone it is. Both
device drivers already know the model, but the panel never receives it.

## Considered Options

* The name, then the model in grey parentheses.
* The model alone.
* The name alone, as now.

## Decision Outcome

Chosen option: the name, then the model in grey parentheses, because the name
tells the user's devices apart and the model says what each one is.

* The model is the driver's: the simulator's device type on iOS and the
  virtual device's hardware profile on Android. It is never derived from the
  name, which the user can change.
* A device whose model is unknown shows its name alone.
* When the toolbar is narrow, the model gives way before the name.

```text
 │ [logo] Work iPhone (iPhone 17 Pro)              ⟲  ⟳  ◐  ⌂  ⋯ │
 │ [logo] pixel10 api36 (Pixel 10)                 ⟲  ⟳  ◐  ⌂  ⋯ │
                       └───── grey ─────┘
```

### Consequences

* Good, because the user sees which phone a tab shows without opening it.
* Modules touched: the mobile plugin's panel data and live device messages,
  which now carry the model the drivers already read, and the toolbar.
