# Skills

A skill is a packaged workflow the agent can follow: a directory with a
`SKILL.md` that says when the skill applies and what to do, and whatever
scripts or references that file names. Users add skills from git
repositories and turn each one on or off. The agent sees the skills that are
on, reads one when a task matches it, and runs its scripts with its ordinary
shell tools. Skills are a [plugin](../architecture/plugins.md), `plugin-skills`;
nothing else in Demi knows them.

For example, a user adds the source `vercel-labs/agent-skills` and turns on
`web-design-guidelines`. In a conversation on their laptop, they ask the
agent to review a page's markup:

1. Before the turn's first request, the model receives a context block that
   lists the skills that are on, with the path of each `SKILL.md`.
2. The model sees that `web-design-guidelines` matches and runs
   `cat ~/.demi/plugins/skills/<digest>/SKILL.md`.
3. Before that job starts, host access finds that the laptop does not hold the
   skill's directory yet and installs it
   ([Host directories](../architecture/plugins.md#host-directories)).
4. The model follows the file, which may tell it to run a script from the same
   directory.

## A skill

A skill is a directory of a source's repository that holds a file named
`SKILL.md`. The file starts with YAML front matter, between two `---` lines,
with two fields Demi reads:

| Field | Rule |
| --- | --- |
| `name` | 1 to 64 characters: lowercase letters, digits and hyphens |
| `description` | 1 to 1,024 characters: what the skill does and when to use it |

Other front matter fields are ignored. The skill's files are every regular
file under its directory, except the files of a directory below it that is a
skill of its own. Symbolic links and submodules are not followed. A file keeps
its executable bit, so a skill's scripts run as the repository has them.

A `SKILL.md` without these two fields, or with one that breaks its rule, is
not a skill. The source lists it as skipped, with its path and the reason, and
its other skills are offered as usual.

## Sources

A source is one git repository, added by one user. It is written as
`owner/repo`, for a repository on GitHub, or as an `https` URL of any git
repository. Only public repositories can be added: Demi sends no credential
when it fetches.

Each source is pinned to one commit:

- **Adding** a source fetches the repository's default branch, pins its newest
  commit and lists the skills that commit holds. Every skill starts off.
- **Updating** fetches again and pins the new newest commit. A skill that was
  on stays on if the new commit still has a skill of that name; a skill the
  commit no longer has is gone.
- **Removing** forgets the source and its skills.

A source never changes by itself. What the agent sees changes only when the
user adds, updates, removes or turns on or off.

A fetch is shallow: it reads one commit, without history. It runs on the
blocking pool and stops as soon as it has received 64 MiB. One source has at
most 100 skills, whose files hold at most 16 MiB together. Every skill that is
on is installed on each Host its user's conversations run jobs on, so these
bounds bound what a Host keeps. A fetch that exceeds one of them fails.

A fetch that fails, because the repository does not answer, does not exist,
is too large or holds no skill, leaves the source as it was and records the
failure, which the page shows until the next fetch of that source succeeds.
A source whose first fetch failed has no commit and no skills, and the user
can update or remove it. One fetch of a source runs at a time; updating a
source that is being fetched changes nothing. A fetch cut off by the backend's
shutdown records nothing, so the source shows what it showed before.

Two skills that are on never have the same name, so a name in the agent's
list always means one skill. Turning on a skill whose name another skill
that is on already has is refused, and the refusal names the other skill's
source. An update cannot break this: a skill stays on only under the name it
was on with, and a new skill starts off.

## What the plugin keeps

The plugin keeps one [value](../architecture/plugins.md#the-contract) per
source, keyed by the source's id:

| Field | Holds |
| --- | --- |
| `origin` | The repository, as the user wrote it |
| `commit` | The pinned commit; absent until the first fetch succeeds |
| `fetchedAt` | When the pinned commit was fetched |
| `skills` | Each skill's name, description, directory in the repository, files (path, mode and the SHA-256 of its bytes) and whether it is on |
| `skipped` | Each `SKILL.md` that is not a skill, with its path and reason |
| `failure` | The last fetch's failure, with its time and message; absent once a fetch succeeds |

The files' bytes are blobs in the user's namespace, which the value names, so
they stay as long as the source does. Whether a fetch is running is not
stored: the plugin's instance holds it in memory.

Whenever the skills that are on change, the plugin sets its user's
[Host directories](../architecture/plugins.md#host-directories) to one
directory per skill that is on, and marks its part of the product state as
changed.

## What the model sees

The plugin's system-prompt text is fixed:

```text
Skills are packaged instructions for particular tasks. The context lists the
skills that are on, each with the path of its SKILL.md. When a task matches a
skill's description, read its SKILL.md before you start and follow it; the
files it names are in the same directory. Skill directories are read-only.
```

The plugin is a context source
([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)).
Its block lists every skill that is on, in the order of their names, with the
path of its `SKILL.md` on a Host:

```text
[Skills]
The skills that are on:
- tdd: Write the failing test before the change. (~/.demi/plugins/skills/9f2c…/SKILL.md)
- web-design-guidelines: Review UI against Vercel's web interface guidelines. (~/.demi/plugins/skills/41ab…/SKILL.md)
```

Before each request of a node, the plugin compares the block it would write
now with the newest of its blocks the model receives, and writes a new one
only when they differ. When no skill is on and the model has been told of
some, the block says `No skill is on.`; when no skill is on and the model was
never told of one, the plugin writes nothing.

So a subagent receives the list before its first request as its parent did;
a conversation whose user turns a skill on mid-conversation learns of it
before its next request; and after a compaction the model is told the list
again.

The path names the skill's digest, so it is the same on every Host and
changes only when the skill's files change. A target switch therefore needs
no new block: the new Host receives the directory when its first job needs
it.

## The page

The settings section is `@demicodes/plugin-skills`, which composes `web-ui`'s
Skills page ([The page](../architecture/plugins.md#the-page)).

The plugin's state for the user's pages holds every source, in the order they
were added: its id, its origin, its commit and when it was fetched, whether a
fetch is running, its failure, its skills with their names, descriptions and
whether each is on, and its skipped files.

| Method | Parameters | Result |
| --- | --- | --- |
| `add_source` | `origin` | The new source's id, once the source is recorded; its fetch continues after the call |
| `update_source` | `source` | Nothing; the fetch continues after the call |
| `remove_source` | `source` | Nothing |
| `set_enabled` | `source`, `skill`, `enabled` | Nothing |
| `set_source_enabled` | `source`, `enabled` | Nothing; turns every skill of the source on or off, and refuses as `set_enabled` would |

An origin that is neither `owner/repo` nor an `https` URL, or that names a
source the user has already added, is refused. A call that names a source or
a skill the user does not have is refused. Every change, and the end of every
fetch, sends the new state to each of the user's pages.

## Acceptance

- Adding a source lists its skills, all off, and its skipped files with their
  reasons; a source too large or without a skill shows its failure and keeps
  no skill.
- A skill turned on reaches the next request of every node of every
  conversation of its user as a context block with the paths of the skills
  that are on, and a model that reads a listed `SKILL.md` on the Host reads
  the skill's file; a script of the skill keeps its executable bit.
- A conversation that runs no job installs nothing on its Host; one that runs
  a job installs each directory once per runner connection, and a directory
  no longer on is removed from the Host at its next installation.
- After a compaction, the next request carries the list again.
- An update keeps the skills that are on by name and drops the ones the new
  commit lacks; a failed update leaves the source as it was and shows the
  failure.
- Turning on a second skill of a taken name is refused with the other
  skill's source.
- A shutdown during a fetch leaves the source as it was before the fetch.
