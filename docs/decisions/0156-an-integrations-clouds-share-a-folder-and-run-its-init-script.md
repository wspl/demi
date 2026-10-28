---
status: proposed
---

# An integration’s Clouds share a folder and run its init script

## Problem

A team's chat bot, built as an integration (0149), gives each person a
Cloud of their own. Today every new Cloud starts bare: the bot's agents
spend their first turn installing the same tools on each, and lose them at
every reset, which replaces the system layer. The team's files cannot be
handed from one person's Cloud to another's. The program cannot send a
person's screenshot to an agent, nor receive the file an agent attaches to
its reply. It cannot tell what each person's work costs, and it cannot
remove a Cloud it no longer needs, nor can its user remove the integration.

## Decision

### The init script

An integration has one init script, a shell script that its Settings page
and `PATCH /v1/integration {initScript}` edit; none by default.

* A Cloud of the integration runs it as `demi` in `~`, where `sudo` works,
  once its system is new: at its first use, and after every reset. A
  conversation's work on that Cloud waits until it ends.
* The Cloud shows the run: `GET /v1/clouds/{cloud}` answers `init` with
  `pending`, `running`, `done`, or `failed`, with its exit code and the last
  64 KiB of its output, which is the room a person needs to see why. A
  Cloud whose run failed still serves its conversations: the program, not
  Demi, decides what a failure means for its users. A run has no time
  limit, since a script that installs packages may take long.
* After a change of the script, each Cloud whose system an earlier script
  made gets a new system at its next start, as a reset's disk step makes
  one, and runs the new script. A Cloud that runs keeps its system until it
  stops: a reset now would stop the turns that use it. `/home` stays, so
  what the script put there before stays.

The script never names an image: a Cloud keeps the base Demi ships, and the
script is the one way to shape it.

### The shared folder

Every Cloud of an integration mounts one shared folder, writable by all of
them at once. A file one person's agent writes there, every other Cloud of
the integration reads at once.

* Its mount path is a setting of the integration's, on its Settings page and
  `PATCH /v1/integration {sharedPath}`, `~/shared` by default. A path is
  either in the home or under a top-level directory the base does not have,
  such as `/data`: a path over the base's own directories (`/usr`, `/etc`,
  `/opt` and the like) would hide what a Cloud needs to run, and is
  refused.
* A Cloud mounts it when it starts, so a new path takes effect at each
  Cloud's next start; the files stay and appear at the new path.
* The machine manager keeps it as an ext4 image of its own, whose root
  belongs to `demi`, at the capacity of a home, mounted on the server while
  any Cloud of the integration runs and bound into each. An ext4
  filesystem is mounted once: two mounts of one image would corrupt it.
  gVisor's shared file access makes a write in one sandbox visible to the
  others.
* The sandbox profile gains one mount: the shared folder of the integration
  whose Cloud it is, which the backend names at each wake. A Cloud of a
  person's own, or of another integration, never reaches it.
* A reset keeps it, as it keeps `/home`.
* The development setup stands it in with a temporary directory, linked
  into each scripted machine's home.

### Attachments

The API carries files both ways, with the web app's own blob storage:

| Route | Does |
| --- | --- |
| `POST /v1/attachments?name=` | A file's bytes, at most 25 MiB, sent with its media type, into the user's blobs: its attachment |
| `GET /v1/conversations/{conversation}/attachments/{attachment}` | An attachment the conversation holds, such as the `a3` of an agent's `<attachment id="a3" />`: its name, media type, size and SHA-256 |
| `GET /v1/blobs/{sha256}` | A blob of the integration's user's, by its SHA-256 |

`SendMessage` gains `attachments`, each an id `POST /v1/attachments`
answered and the name the file is written under on the Cloud, which the
message carries as the composer's does.

### Usage

`GET /v1/usage?from=&to=&by=cloud|conversation|model` answers the
integration's usage in the period, grouped, with the tokens, cache reads and
writes and the API cost equivalent that Settings › Usage shows. Each usage
ledger row now keeps the device its agent worked on and the integration
whose conversation it was, fixed as the row is written: an agent may move
between Clouds, and the integration's usage stays its own after its
conversations and Clouds are deleted.

### Deletion

* `DELETE /v1/clouds/{cloud}` deletes a Cloud: its runner goes, which ends
  what runs there, the manager removes its storage, and its device and key
  go. A Cloud that a conversation works on is refused with `409`, naming
  the conversations; the program deletes them or moves them first.
* Settings › Integrations › an integration › **Delete Integration…** asks
  once, saying what goes, then ends its token and sockets and deletes its
  conversations, its Clouds and its shared folder. It is deleted at once,
  not marked removed as a project is: a project is a folder that is still on
  disk when it is added back, while nothing of an integration's would bring
  it back. Its usage stays in Settings › Usage, as a deleted conversation's
  does.

### Stored and spoken

* `integrations.init_script` and `integrations.shared_path`;
  `integration_clouds.init`, the state of its run, with the last run's exit
  code and output; `usage_ledger.device_id` and `integration_id`.
* The manager keeps each shared folder's image at
  `<data>/shared/<integration>/shared.ext4`.
* The manager's wire, version 7, gains `delete` (a device's storage),
  `delete_shared` (a shared folder's image, refused while a sandbox mounts
  it), and the shared folder a `wake` names.

## Spike

Run on the manager in a Linux VM with gVisor's systrap platform, driven over
its socket, with two Cloud sandboxes naming one shared folder and commands run
in each as the Cloud's user:

* The folder is mounted once on the server, from its image on a loop device,
  and appears in each sandbox at its path, owned by the Cloud's user.
* A file or a directory one sandbox writes, the other reads at once, and an
  append from the second is read back in the first: gVisor checks a mount's
  files against the server's at each lookup under
  `--file-access-mounts=shared`.
* `inotify` reports only the changes made in its own sandbox. A watch in one
  sandbox saw its own new file and none of the other's writes, so an agent
  that waits for another Cloud's file has to look for it.
* The folder stays mounted while one sandbox has it, is unmounted when the
  last one stops, and is mounted again with its files at the next start.
* Its deletion is refused while a sandbox has it; afterwards no image, mount
  or mount point is left.
* After the manager restarts under a running sandbox, the folder is no
  longer mounted, and the next start mounts it with its files.

## Prior art

* [Codespaces](https://docs.github.com/en/codespaces/setting-up-your-project-for-codespaces/adding-a-dev-container-configuration/introduction-to-dev-containers)
  and [Gitpod](https://www.gitpod.io/docs/configure/workspaces/tasks): a
  configuration's `postCreateCommand` or `init` task runs once when a
  workspace is created, which a rebuild runs again.
* [Fly Volumes](https://fly.io/docs/volumes/overview/): a volume belongs to
  one machine; sharing across machines is left to the application. Demi
  shares one directory because the integration's agents need it and the
  program cannot reach the Clouds' disks.

## Alternatives

* **A base image per integration:** the person who writes the integration
  would build and publish images, and an update of Demi's base would leave
  theirs behind.
* **An init script per Cloud:** every Cloud of an integration serves the
  same product; a script per Cloud invites drift that the program then has
  to repair.
* **Rerunning the script on each wake:** a wake must be fast, and a script
  that installs packages is not.
* **A shared directory on network storage:** Clouds run on one server
  today, so a local image serves them without a network file system.

## Open questions

None.
