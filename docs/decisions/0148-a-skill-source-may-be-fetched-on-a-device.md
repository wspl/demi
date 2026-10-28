---
status: proposed
---

# A skill source may be fetched on one of the user's devices

## Problem

Alex's employer keeps its skills in a private repository,
`git@github.example.com:acme/internal-skills.git`, which only his work
laptop can reach: the laptop has the SSH key, and the company's network.
Demi's server fetches every skill source itself, anonymously, over HTTPS,
so Settings › Skills cannot add the repository at all, though the laptop
is paired and online.

## Decision

A source is fetched by the server, as today, or by one of the user's paired
devices, chosen when it is added and changed afterwards. The device runs
`git` as its user, with that user's keys, credential helpers and network,
and the server keeps what it fetched as it keeps a server fetch's files. Where
the skills are offered and installed stays the source's place (0147).

```text
Add Source                                              ×
A Git repository. Each SKILL.md in it is a skill.
  Repository   git@github.example.com:acme/internal-skills.git
  Fetch With   [ alex-mbp               ▾ ]
               Server
               alex-mbp
               build-box (offline)
                                      [Cancel] [Add Source]
```

* **Fetch With** lists Server and every paired device, an offline one
  marked; Server is the default. A device makes SSH origins possible:
  `git@host:owner/repo.git` and `ssh://` URLs, which the server refuses.
  `git@host:/srv/skills` and `ssh://git@host/srv/skills` are one source.
* The source's row says what fetches it under its origin, `via alex-mbp`,
  and its open fold starts with a Fetch With row, the same menu, so the
  choice stays editable, except while the source is being fetched. A
  change fetches the source again on the new fetcher.
* The fetcher fetches the source when it is added and on Update, and checks
  its default branch on the schedule the server's checks keep. An offline
  device fetches and checks nothing: Update says *alex-mbp is offline*, and
  the source shows *Waiting for alex-mbp* until the device is back and has
  checked. Its skills stay on and installed meanwhile.
* A device that lacks `git`, or whose `git` fails, shows `git`'s own words
  as the fetch's failure, as a server fetch's failure is shown.
* A device's unpairing leaves its sources fetched by no one: they keep
  their skills, show *alex-mbp was removed*, and fetch again once the user
  chooses another fetcher.
* `demi skills add` gains `--fetch-with <device>`, and `demi skills list`
  shows each source's fetcher.

The design to build from:

* A source's value gains `fetcher`, a device's id; none is the server.
* The plugin asks the backend for two things on a device, through two port
  calls the backend serves with device access
  (`crates/backend-host-access/src/lib.rs`), which wakes no Cloud and takes
  no conversation's gate:
  * **Fetch:** a bare, shallow clone without the files' contents,
    `git clone --bare --depth 1 --single-branch --no-tags
    --filter=blob:none`, into a temporary directory of the device's; then
    the commit, `git rev-parse HEAD`, the tree, `git ls-tree`, and, one by
    one, the objects of the skills' files, `git cat-file blob`. These are the
    commit's own bytes, as the server's fetch reads them: no checkout's line
    endings, filters or Git LFS. A remote that cannot leave the contents out
    sends them all, which the repository's limit (64 MiB) refuses after the
    clone; the files' (16 MiB) stops the reads as they pass it. The
    directory is removed afterwards, and a fetch first removes the
    directories an hour old that fetches cut off left.
  * **Check:** `git ls-remote <origin> HEAD`.
  * `git` never waits for anyone: its standard input is closed, git's,
    SSH's and Git Credential Manager's prompts are off, each run has a time
    limit after which it is killed, and the variables that would point it
    at another repository, such as `GIT_DIR`, are removed. A device's check
    waits for no other check.
  * A device that goes away during a fetch records nothing: the source
    waits for it. A fetch whose source took another fetcher meanwhile
    records nothing either, and the new fetcher fetches it.
* What a fetch answers is the same as a server fetch's, files and commit, so
  parsing the skills, the name rule and the Host directories are unchanged.
* A device's check is the user's alone: its head is kept apart from the
  heads every user's instance shares, so a private repository's commit
  never reaches another user, and one user's device never checks for
  another.

## Spike

* Device access gives the backend a Host of a connected device whose
  `spawn` the runner serves as any job's
  (`crates/backend-runners/src/devices.rs`,
  `crates/runner/src/connection/owner.rs`), with the device's own
  environment when the request names none: its user's `HOME`, so `~/.ssh`
  and `git`'s credential helpers apply.

## Prior art

* **Claude Code** adds a plugin marketplace from a repository with the
  user's own `git`, on the user's machine, so a private repository works
  as the user's other clones do.
* **skills.sh**, `npx skills add`, clones with the local `git` as well.

Demi's server is not the user's machine, so the user picks which of their
machines clones, and the server keeps the result for every Host.

## Alternatives

* **A token in Credentials for the server's fetch**: works for a repository
  a token opens, not for one only a company network or an SSH key reaches,
  and puts a credential on the server that the device already holds.
* **Fetch on the Host each job runs on**: a source's files would differ
  from Host to Host, and a Host without access would install nothing.

## Open questions

None.
