# Conversation host menu

The conversation's host menu manages one main execution host and any number of
attached hosts, following `sessions-and-targets.md`. These are execution devices,
not other chat conversations. Workspace files and Git branches remain separate
header controls for the main environment.

```text
Conversation c1 header: [zan-mbp +2] [demi] [main]
                         |
                         +-- Main execution environment
                         |     zan-mbp · Online
                         |     Switch main environment… > devices / Cloud / Hostless
                         +-- Attached hosts · 2
                         |     build-01 · Offline > build-01
                         |                  > Use as main / Detach
                         |     studio · Online > studio
                         +-- Attach device… > searchable owned devices
                         +-- Connect new device… > account device settings
```

The trigger shows the main device and the attachment count. Cloud uses its icon;
Hostless uses a small host-management icon and an optional count, without main
device, workspace or branch metadata. Attaching is available in every main-host
state. The empty attachment section explains that no additional devices are bound.

Attach selects an existing owned device and starts its cwd at its home. The
picker excludes the main device and devices already attached; offline devices
may be attached but cannot be promoted until online. Managed Cloud is not in the
attachment picker; it can enter the list as a departed main host. Connecting a
new device opens the global device settings, which is separate from granting a
particular conversation access to that device.

Attachment names are generated from device names and kept unique within the
conversation. The menu exposes device identity, online status, promotion and
detach; it has no rename action or directory display. Directory selection and
browsing belong to the separate workspace control. The menu contains actions and
status only, without explanatory help paragraphs. Detach removes only the
conversation binding, not the device registration.

Switching main environments requires an idle, unarchived conversation. The picker
starts at an attachment's cwd when promoting it. On completion, the incoming host
leaves the attachment list and the departed main host joins it with its last
directory. Switching directories on the same device creates no duplicate binding.
Attaching and detaching remain available during a turn; their model
announcement belongs to the next turn boundary per the execution contract.
Archived conversations allow inspection but not binding mutations.

The frontend prototype keeps bindings, aliases and cwd in memory. It does not
connect devices or grant real execution permissions. Store coverage in
`packages/web/src/conversation/store.test.ts` exercises unique device identity,
default home directories, duplicate attachment prevention, alias collision,
main/attached exchange, retained cwd and detach. Browser verification covers the
menu's binding, detach and Hostless entry flows.
