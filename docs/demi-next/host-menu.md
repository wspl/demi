# Conversation hosts and directories

A conversation has one main execution environment and optional attached devices.
The host menu changes those bindings. A separate workspace control selects the
main environment's directory or project. Neither control moves conversation
history or copies project files. [Sessions and targets](sessions-and-targets.md)
owns target exchange, execution guards, and announcements to agents.

`web-ui/hosts/HostMenu` and `HostPicker` own menu interaction.
`WorkspaceDirectoryMenu` owns recent-directory selection and its folder dialog.
Product adapters in `web/targets` supply device state, filesystem sources, and
backend handlers; gallery fixtures supply the same presentation models.

## Host menu

```text
Header: [laptop +2] [demi]
          |
          +-- Main host                 laptop > Cloud / Connect / search
          +-- Attached hosts
          |     build-server (online)          > Use as main / Detach
          |     studio (offline)               > Use as main / Detach
          +-- Attach device...                 > Connect / search
          +-- Connect new device...            > pairing dialog
```

The trigger shows the main device's name and attachment count, using a Cloud or
computer icon. Cloud remains the same selection before and after allocation;
sleeping, starting, ready, and failed describe its lifecycle, not different targets.
The attached section appears only when bindings exist.

Main host is a single label/value submenu row. Its `HostPicker` offers Cloud and
Connect new device above a search field and owned-device results. The attachment
picker has the same layout but omits Cloud. Searching filters device names without
hiding top actions. The current main device is disabled; the attachment picker
also disables devices already bound to the conversation.

Offline devices stay visible with an offline label but cannot be newly selected
in these pickers or promoted through the menu. Existing offline attachments remain
visible and can be detached. This is a browser selection rule: the backend can
retain offline bindings and the host API does not use online status as ownership
authorization. Cloud can enter the attached set when it is a departed main target,
although it is absent from the ordinary attachment picker.

Attached rows show a device icon, a green online or gray offline corner dot,
and accessible status text. Their submenu offers Use as main environment and
Detach. Names derive from device names and are made unique by the backend.
The menu has no alias-renaming or cwd-editing action; directories belong to the
workspace control. `MenuGroup` and `MenuItem` supply shared section, metadata,
status, and submenu presentation.

Attach grants this conversation access to a registered device. A new attachment
uses that device's home as its default cwd. Detach removes only the conversation
binding. Connect new device opens [Pairing](device-pairing-ui.md), which registers
a device with the account; it does not attach it to this conversation.

## Changing the main environment

For a user device, Use as main opens the folder browser. Promoting an attachment
starts at its retained cwd; choosing another device starts at its home. Confirming
a folder moves into a matching existing project or sets a direct device target.
Choosing Cloud uses the Cloud target without a user-device folder dialog.

On successful exchange, the incoming device leaves the attached set and the old
main device becomes attached with its last directory. Changing directories on the
same device does not add a duplicate binding. For example, switching from Cloud
to `/work/demi` on a laptop preserves Cloud as an attached host; subsequent main
shell jobs run in that laptop directory.

Main-environment mutations require an idle, unarchived conversation and the
backend's complete execution-tree guard. The product disables the main control
while its root is non-idle or a metadata mutation is pending; the backend also
rejects conflicts such as an active child. Attach and detach remain available
during a turn, with announcements at the next turn boundary. Archived
conversations expose bindings for inspection but disable their mutation.
A failed request retains the existing target and reports the error.

## Directory and project controls

The workspace trigger shows the project name or directory basename, with the
full path in its tooltip. It lists up to eight recent projects on the current
device, marks the current project, and switches directly when selected.
Choose another directory opens the shared folder browser with the device's
workspaces as places and other owned devices in its sidebar. A rejected selection
keeps the dialog open; a pending confirmation prevents a duplicate request.

Main-directory changes are locked during a running turn or after archive.
The product currently disables browsing Cloud directories. Device browsing uses
real backend filesystem adapters; directory creation changes that device.
General file inspection and remote attachments use the independent
[File browser](../file-browser.md) contract.

`WorkspaceDialog` creates a project; moving between existing projects uses the
sidebar's Move action or the workspace header. The new-project form starts with
Device and the first online device. Device projects select a directory and take
its basename as the project name. Cloud projects ask for a name. Add device opens
the shared pairing dialog above the form; Browse opens a folder page within it.
The form shows loading and failure of device data, blocks duplicate creation,
and preserves input while a submitted operation or its error remains active.

## Implementation discrepancies

The menu's main-host presentation currently receives identity and kind, but no
Cloud lifecycle field. Its trigger therefore shows Cloud consistently without
showing the intended sleeping/starting/failed distinction there; Cloud settings
has the lifecycle presentation.

A departed Cloud attachment cannot currently be promoted through its attached-row
action. That action passes its real device ID to the user-device browser, whose
sources exclude managed devices. Only the explicit Cloud picker action takes the
Cloud-target path. The intended exchange above is supported by the backend; the
product adapter still needs to recognize a managed attachment in this action.

Acceptance must cover both a normal device exchange and this Cloud return path,
retained cwd, offline rows, child-running rejection, archive locks, pairing from
nested forms, failed folder confirmation, and project creation. Backend scenario
coverage alone does not establish browser flow completion.
