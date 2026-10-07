# Skills

A skill is a packaged workflow the agent can follow: a directory with a
`SKILL.md` that says when the skill applies and what to do, and whatever
scripts or references that file names. Skills follow the
[Agent Skills](https://agentskills.io) format, so a skill written for another
agent works in Demi unchanged. They come from two places: git repositories a
user adds as sources and turns on skill by skill, and the repository a
conversation works in. The agent sees the skills that are available, reads one
when a task matches it, and runs its scripts with its ordinary shell tools.
Skills are a [plugin](../architecture/plugins.md), `plugin-skills`; nothing
else in Demi knows them.

For example, a user adds the source `vercel-labs/agent-skills` and turns on
`web-design-guidelines`. Their repository `~/app` holds a skill of its own in
`.agents/skills/release/`. In a conversation on their laptop in `~/app`, they
ask the agent to review a page's markup:

1. Before the turn's first request, the model receives a context block with
   the catalog of both skills and the path of each `SKILL.md`: the source's
   skill in its installed directory, the repository's skill where it is.
2. The model sees that `web-design-guidelines` matches and runs
   `cat ~/.demi/plugins/skills/web-design-guidelines-41ab07c2d9e5/SKILL.md`.
3. Before that job starts, host access finds that the laptop does not hold the
   skill's directory yet and installs it
   ([Host directories](../architecture/plugins.md#host-directories)).
4. The model follows the file, which may tell it to run a script from the same
   directory.

## A skill

A skill is a directory that holds a file named `SKILL.md`. The file starts
with YAML front matter, between two `---` lines. Demi reads three of its
fields:

| Field | Rule |
| --- | --- |
| `name` | 1 to 64 characters: lowercase letters, digits and hyphens, neither first nor last a hyphen and no two in a row, and the same as the skill's directory name |
| `description` | 1 to 1,024 characters: what the skill does and when to use it |
| `disable-model-invocation` | `true` keeps the skill out of the catalog |

Validation is lenient, as the format's guide for clients asks:

- A `name` that breaks its rule, or differs from the directory's name, is a
  warning, and the skill loads under the name its front matter gives. A
  missing `name` takes the directory's name.
- A `description` longer than 1,024 characters is a warning, and the skill
  loads.
- A missing or empty `description`, front matter that does not parse as YAML,
  or a `SKILL.md` larger than 256 KiB is not a skill: it is skipped, with its
  path and the reason.

Other front matter fields are ignored. A skill that sets
`disable-model-invocation` is meant to be invoked by the user alone, which
Demi does not offer, so it is never listed to the model.

The skill's files are every regular file under its directory, except the
files of a directory below it that is a skill of its own. Symbolic links and
submodules are not followed. A file keeps its executable bit, so a skill's
scripts run as the repository has them.

## User skills

A source is one git repository, added by one user. It is written as
`owner/repo`, for a repository on GitHub, or as an `https` URL of any git
repository. Only public repositories can be added: Demi sends no credential
when it fetches. Every directory of the repository that holds a `SKILL.md` is
a skill.

Each source is pinned to one commit:

- **Adding** a source fetches the repository's default branch, pins its newest
  commit and lists the skills that commit holds. Each skill starts as a new
  skill does, below.
- **Updating** fetches again and pins the new newest commit. A skill that was
  on stays on if the new commit still has a skill of that name, and one that
  was off stays off; a skill the commit no longer has is gone. A skill whose
  name the previous commit did not have, which is every skill when the first
  fetch failed, is new.
- **Removing** forgets the source and its skills.

A new skill starts on, so that adding a source gives the agent what the user
added it for, with two exceptions that start off:

- A skill whose name another user skill that is on already has, in any
  source, or earlier in this commit's order of paths. Two skills that are on
  never share a name (below), so this one waits, and the page names the
  other skill's source.
- A skill that sets `disable-model-invocation`. The agent is never offered
  it, so on it would only install its files on every Host and take its name
  from another skill; the user can still turn it on.

For example, the user has `review` from `acme/tools` on and adds
`acme/more`, which holds `review` and `lint`: `lint` starts on, and `review`
starts off with the page saying that a skill of that name from `acme/tools`
is on.

A source never changes by itself. Its skills change only when the user adds,
updates, removes or turns on or off.

A fetch is shallow: it reads one commit, without history. It runs on the
blocking pool and stops as soon as it has received 64 MiB. One source has at
most 100 skills, whose files hold at most 16 MiB together. Every user skill
that is on is installed on each Host its user's conversations run jobs on, so
these bounds bound what a Host keeps. A fetch that exceeds one of them fails.

A fetch that fails, because the repository does not answer, does not exist,
is too large or holds no skill, leaves the source as it was and records the
failure, which the page shows until the next fetch of that source succeeds.
A source whose first fetch failed has no commit and no skills, and the user
can update or remove it. One fetch of a source runs at a time; updating a
source that is being fetched changes nothing. A fetch cut off by the backend's
shutdown records nothing, so the source shows what it showed before.

Two user skills that are on never have the same name. Turning on a skill
whose name another user skill that is on already has is refused, and the
refusal names the other skill's source. An update cannot break this: a skill
stays on only under the name it was on with, and a new skill whose name is
taken starts off.

### Updates available

A source never updates by itself, but the page says when an update would
change it. When the user opens the Skills page, the page asks the plugin to
check (`check_updates`). For each source last checked more than five minutes
ago, the plugin reads the commit the repository's default branch points to
from the remote's ref advertisement: the list of refs and their commits a git
server sends before any object, which `git ls-remote <url> HEAD` prints. It
fetches no content, through the same library and with the same isolation as
a fetch. When that commit differs from the pinned one, the page shows
**Update available** on the source, until the user updates it.

- The checks of one call run together, each on the blocking pool, and the
  pages receive the new state once they all ended, if a source's newest
  commit changed.
- A check that fails is logged and changes nothing: the page shows what it
  showed, and the source is checked again five minutes later.
- A fetch that succeeds counts as a check that found the commit it pinned,
  so a source just updated never shows an older check's result.
- What the checks found is kept in the instance's memory, with when each
  source was checked. A new instance, after a restart, checks again when the
  page next opens.

Five minutes keeps a user who opens and closes settings from sending a
request to every repository each time, while a commit pushed during a session
shows the next time the page opens after the interval.

### What the plugin keeps

The plugin keeps one [value](../architecture/plugins.md#the-contract) per
source, keyed by the source's id, the first 12 hexadecimal digits of the
SHA-256 of its repository's URL (`owner/repo` stands for
`https://github.com/owner/repo`; a trailing `.git` or `/` and the host's case
do not count), so two origins of one repository are one source:

| Field | Holds |
| --- | --- |
| `origin` | The repository, as the user wrote it |
| `added` | Its place in the order the user added sources |
| `commit` | The pinned commit; absent until the first fetch succeeds |
| `fetchedAt` | When the pinned commit was fetched |
| `skills` | Each skill's name, description, directory in the repository, files (path, whether it is executable, and the SHA-256 of its bytes), warnings, whether it sets `disable-model-invocation`, and whether it is on |
| `skipped` | Each `SKILL.md` that is not a skill, with its path and reason |
| `failure` | The last fetch's failure, with its time and message; absent once a fetch succeeds |

The files' bytes are blobs in the user's namespace, which the value names, so
they stay as long as the source does. Whether a fetch is running, and the
newest commit a check found, are not stored: the plugin's instance holds them
in memory.

Whenever the user skills that are on change, the plugin sets its user's
[Host directories](../architecture/plugins.md#host-directories) to one
directory per skill that is on, named after the skill, and marks its part of
the product state as changed. A name that breaks the name rule gives its
directory its lowercase letters and digits, joined by single hyphens; two
skills whose directories would have the same name count as two of the same
name, so only one of them is on. Such a skill's path in the catalog is its
directory on a Host, `~/.demi/plugins/skills/<name>-<short digest>/SKILL.md`,
the same on every Host, so a target switch changes no path: the new Host
receives the directory when its first job needs it.

## Project skills

A repository can carry its own skills, in the directories other agents read
too. For a node working in `~/app/web`, inside the git repository `~/app`, the
plugin looks in:

```text
~/app/web/.agents/skills/    ~/app/web/.claude/skills/
~/app/.agents/skills/        ~/app/.claude/skills/
```

that is, `.agents/skills` and `.claude/skills` in the node's working
directory and in each directory above it up to the root of its git
repository, the nearest directory that holds `.git`. Outside a git
repository, only the working directory is searched. Every directory under
these that holds a `SKILL.md` is a skill, and the search does not descend into
a skill's directory. It reads at most 2,000 directories, to a depth of six
below each skills directory, and finds at most 100 skills.

Project skills are always available: they come with the repository, and
changing the repository is how they change. Their files stay where they are,
and the catalog names the `SKILL.md` at its own path on the Host. Demi does
not check them first: the user chose the working directory, and the agent
runs its code in any case.

The plugin reads them through its port's
[Host files](../architecture/plugins.md#reading-a-conversations-files), which
never wakes a Host:

- It searches at the first request of each input turn of a node, and again at
  the next request of that turn when the Host was not running, until a search
  succeeds. For example, on a stopped Cloud the first request lists no
  project skill; the model's first command wakes the Cloud, and the request
  after it lists them.
- It keeps the result in memory, for the conversation's Host and the node's
  working directory, and lists it while a search cannot run. An edit to the
  repository's skills reaches the catalog at the next input turn.
- A project skill that is not a skill, or has warnings, is logged. The page
  shows only user skills.

When two skills that are available have the same name, one of them is listed:
a project skill over a user skill, as the format's guide asks; of two project
skills, the one in the nearer directory; and in one directory, `.agents/skills`
over `.claude/skills`. The other is logged as shadowed.

## What the model sees

The plugin is a context source
([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)).
Its block holds the catalog of every skill that is available to the node: the
user skills that are on and the project skills it found, without the skills
that set `disable-model-invocation`. The catalog has the format other agents
and the Agent Skills guide use, sorted by name, with every value XML-escaped:

```text
The following skills provide specialized instructions for specific tasks.
When a task matches a skill's description, read its SKILL.md at the listed
location before you start, and resolve relative paths in it against the
skill's directory.

<available_skills>
  <skill>
    <name>release</name>
    <description>Cut a release: bump, tag and publish the changelog.</description>
    <location>/home/me/app/.agents/skills/release/SKILL.md</location>
  </skill>
  <skill>
    <name>web-design-guidelines</name>
    <description>Review UI against Vercel's web interface guidelines.</description>
    <location>~/.demi/plugins/skills/web-design-guidelines-41ab07c2d9e5/SKILL.md</location>
  </skill>
</available_skills>
```

The block holds at most 8,000 characters. When the catalog is longer, every
description is shortened to the same length, cut at a word and ended with
`…`, the longest that fits. When the entries do not fit even without
descriptions, the last entries are left out, and the block ends with the line
`<n> more skills are not listed.` 8,000 characters are about 2,000 tokens,
about 1% of a 200,000-token context window, the budget other agents give their
catalogs.

Before each request of a node, the plugin compares the block it would write
now with the newest of its blocks the model receives, and writes a new one
only when they differ. When no skill is available and the model has been told
of some, the block says `No skills are available now.`; when no skill is
available and the model was never told of one, the plugin writes nothing.

So a subagent receives the catalog before its first request as its parent
did, for its own working directory; a conversation whose user turns a skill on
mid-conversation learns of it before its next request; and after a compaction
the model is told the catalog again.

## The page

The settings section is `@demicodes/plugin-skills`, which composes `web-ui`'s
Skills page ([The page](../architecture/plugins.md#the-page)), and the
sidebar's Skills entry opens it
([The page object](../architecture/plugin-pages.md#the-page-object)). It shows
user skills; project skills belong to their repositories.

The plugin's state for the user's pages holds every source, in the order they
were added: its id, its origin, its commit and when it was fetched, whether a
fetch is running, its failure, whether an update is available, its skills,
and its skipped files. Each skill has its name, description and warnings,
whether it is on, whether it sets `disable-model-invocation`, which the page
shows as never offered to the agent, and, while it is off, the origin of the
source whose skill that is on has its name, which the page shows as the
reason it cannot be turned on.

A source shows its state as one status label right after its name, so it
never covers the origin under it at any width: **Adding** while a new
source's first fetch runs, **Updating** while a later fetch runs, when its update button also turns; **Failed**, with the failure's
message, until a fetch succeeds; or **Update available**. A source whose
first fetch failed has no commit and no skills, and shows Failed alone.

| Method | Parameters | Result |
| --- | --- | --- |
| `add_source` | `origin` | The new source's id, once the source is recorded; its fetch continues after the call |
| `update_source` | `source` | Nothing; the fetch continues after the call |
| `remove_source` | `source` | Nothing |
| `set_enabled` | `source`, `skill`, `enabled` | Nothing |
| `set_source_enabled` | `source`, `enabled` | Nothing; turns every skill of the source on or off, and refuses as `set_enabled` would |
| `check_updates` | None | Nothing; checks the sources that are due ([Updates available](#updates-available)) after the call |

An origin that is neither `owner/repo` nor an `https` URL, or that names a
source the user has already added, is refused. A call that names a source or
a skill the user does not have is refused. Every change, and the end of every
fetch, sends the new state to each of the user's pages.

## Commands

The agent manages the user's skills with the `demi skills` group, the only
way it installs a skill. For example, asked to "set up Vercel's web design
skill", it runs `demi skills add vercel-labs/agent-skills --skill
web-design-guidelines`; once the user has allowed the conversation to manage
skills ([Conversation permissions](permissions.md)), the skill is in the
catalog of the next request of every node of every conversation of the user.

The group's help tells the model to install and manage skills only with these
commands, never with other tools such as `npx skills add`: Demi does not see
a skill another tool installs, which would reach no other Host and no other
conversation. A source is named by its repository, written as the page's
`add_source` takes it; any spelling of the same repository names the same
source ([What the plugin keeps](#what-the-plugin-keeps)).

| Command | Does |
| --- | --- |
| `list [--json]` | Prints every source with its origin, commit, failure and whether an update is available, and its skills with whether each is on, its description and, while it is off for a taken name, the other skill's source |
| `add <repository> [--skill <name>...]` | [Adds](#user-skills) the source and waits for its first fetch; with `--skill`, only the named skills are on |
| `update <repository>` | Updates the source and waits for the fetch |
| `remove <repository>` | Removes the source and its skills |
| `enable <repository> [--skill <name>...]`, `disable <repository> [--skill <name>...]` | Turns the named skills of the source on or off, or every skill of it without `--skill` |

The group declares one [category](permissions.md#categories), `skills.manage`,
Manage skills: "add, update and remove skill sources and turn skills on or
off", with the description the card shows. Every command but `list` names it,
so in a conversation without the grant it fails before it fetches or changes
anything, and the user is asked ([The check](permissions.md#the-check)). With
the grant, a command acts as the page's method does and fails as it would:

- `add` fails for an origin the page refuses or one already added, and when
  the fetch fails, with the failure on stderr; the source then stays, showing
  its failure, as one the page added does. A `--skill` the commit does not
  hold, or one whose name a skill that is on already has, fails the command
  after the fetch and leaves the source with its skills as the
  [start rule](#user-skills) sets them; the error names each such skill and,
  for a taken name, the other skill's source.
- `update` of a source at its newest commit pins the same commit and prints
  that nothing changed.
- `enable` refuses a taken name as the page's `set_enabled` does, and names
  the other skill's source.
- Every command prints what changed: the commit pinned, and the skills turned
  on or off.

A change made by a command reaches the pages and the agents as one the user
made on the page does.

## Acceptance

- Adding a source lists its skills, on, with their warnings, and its skipped
  files with their reasons; a skill whose name another skill that is on has,
  and a skill that sets `disable-model-invocation`, start off, the first
  showing the other skill's source; a skill whose name breaks its rule loads
  with a warning; a source too large or without a skill shows its failure and
  keeps no skill.
- A user skill turned on reaches the next request of every node of every
  conversation of its user, in the catalog, and a model that reads its listed
  `SKILL.md` on the Host reads the skill's file; a script of the skill keeps
  its executable bit.
- A conversation that runs no job installs nothing on its Host; one that runs
  a job installs each directory once per runner connection, and a directory
  no longer on is removed from the Host at its next installation.
- A repository's `.agents/skills` and `.claude/skills` skills, in the working
  directory and up to the repository's root, are in the catalog at their own
  paths; on a stopped Cloud they appear at the request after the Cloud woke,
  and no search wakes it; a project skill shadows a user skill of the same
  name.
- A skill that sets `disable-model-invocation` is never in the catalog.
- A catalog over 8,000 characters is shortened, then cut with the count of
  the skills left out.
- After a compaction, the next request carries the catalog again.
- An update keeps the skills that are on by name and drops the ones the new
  commit lacks; a new skill starts as an added one does; a failed update
  leaves the source as it was and shows the failure.
- Opening the page shows Update available on a source whose repository's
  default branch points to another commit than the pinned one, and on no
  other; a failed check shows nothing new.
- Turning on a second user skill of a taken name is refused with the other
  skill's source.
- A shutdown during a fetch leaves the source as it was before the fetch.
- In a conversation without the grant of `skills.manage`, every
  `demi skills` command but `list` fails before it fetches or changes
  anything; with it, `demi skills add --skill` leaves exactly the named skills
  of the new source on, and a `--skill` that names a taken name fails the
  command with the other skill's source.
