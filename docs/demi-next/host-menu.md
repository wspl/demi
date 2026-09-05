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
                         |     /Users/zan/Projects/demi
                         |     Switch main environment… > devices / Cloud / Hostless
                         +-- Attached hosts · 2
                         |     ci · Offline > build-01 /home/build
                         |                  > Use as main / Rename / Detach
                         |     design · Online > studio /Users/designer
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

Each attachment has a conversation-local, unique, editable name. Its submenu
shows the underlying device identity and cwd, offers promotion through the
directory picker, rename and detach. The cwd is displayed as execution state,
not an access boundary. Detach removes only the conversation binding, not the
device registration. Rename collisions keep the editor open with an error.

Switching main environments requires an idle, unarchived conversation. The picker
starts at an attachment's cwd when promoting it. On completion, the incoming host
leaves the attachment list and the departed main host joins it with its last
directory. Switching directories on the same device creates no duplicate binding.
Attaching, renaming and detaching remain available during a turn; their model
announcement belongs to the next turn boundary per the execution contract.
Archived conversations allow inspection but not binding mutations.

The frontend prototype keeps bindings, aliases and cwd in memory. It does not
connect devices or grant real execution permissions. Store coverage in
`packages/web/src/conversation/store.test.ts` exercises unique device identity,
default home directories, duplicate attachment prevention, alias collision,
main/attached exchange, retained cwd and detach. Browser verification covers the
menu's binding, rename, detach and Hostless entry flows.
