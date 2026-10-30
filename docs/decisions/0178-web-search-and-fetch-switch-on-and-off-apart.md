---
status: proposed
---

# Web search and web fetch switch on and off apart

## Problem

Alex's Codex, Claude Code and Grok Build subscriptions search the web with
their vendors' own searches, on from the start
([0176](0176-a-subscription-searches-with-its-vendors-search.md)). The `web`
plugin still offers `demi web search`, backed by Exa, beside them, so the
model has two web searches to choose from: one Alex pays for already, and one
on an anonymous allowance every user shares, or an Exa account Alex may not
have.

Alex cannot turn Exa search off alone. Search and `demi web fetch` are one
plugin, and its one switch in Settings › Plugins takes both away, though
every model needs fetch to read a page.

## Decision

The `web` plugin keeps both commands, and each has a switch of its own:
**Web Search**, off until the user turns it on, and **Web Fetch**, on from
the start. A command switched off is not in the model's command set at all.

### Settings › Search

The plugin's page, Settings › Web Search today, becomes **Search**, and
opens with both switches:

```text
Search
+------------------------------------------------------------+
| Web Search                                           [off] |
| The model searches the web with Exa, with your account or  |
| key, or within a limit every user shares.                  |
|------------------------------------------------------------|
| Web Fetch                                             [on] |
| The model reads a page or downloads a file from the        |
| conversation's Host.                                       |
+------------------------------------------------------------+
Exa
  (the account and the key, as today)
```

The plugin's own switch in Settings › Plugins stays: it turns the whole
plugin off, and the page with it, as now.

### What a plugin may declare

A plugin's manifest may name **features**: each a part of one of its command
groups that a user turns on and off apart, with whether it starts on, the
leaves of the group it holds, and its sentence of the group's summary and of
its index entry. The features of a group hold all its leaves. The plugin
host keeps each user's choice and composes a user's command set without the
leaves of a feature turned off. The group then says the sentences of the
features left on, so the model never reads of a command it does not have; a
group left with no feature on goes. The plugin's page reads and sets its
features through the plugin SDK, the same call for every plugin.

`web` names two: `search`, off by default, over `demi web search`, and
`fetch`, on by default, over `demi web fetch`. With search off, the model's
index says of `demi web` only that it fetches a page as Markdown, or a file.

### Design

- `plugin-interface`: `Manifest.features: Vec<Feature>`, `Feature { id,
  on_by_default, group, leaves, summary, index_entry }`, the group by its
  path in one of the plugin's trees.
- `plugin-host`: the registry checks the features against the trees the
  manifest declares: an id of lowercase letters, digits and hyphens, a group
  one tree declares, one or more of its leaves, no leaf two features', and
  every subcommand of the group a leaf one of them holds; a user's feature choices sit beside the plugin
  switches; the command set narrows each tree by the features off; the
  toolset's revision names them, so a switch reaches every conversation's
  next request as a plugin's switch does; turning a feature leaves the
  plugin's instance, tabs and packages as they are.
- Stored data: the control database gains `user_plugin_features (user_id,
  plugin, feature, enabled)`. The migration stores Web Search on for a user
  who signed in to Exa (the sign-in's tokens, or the plugin's mark of one)
  or saved a key, so search does not vanish for them;
  everyone else finds it off.
- `web-api-protocol`: `PluginEntry.features`, each feature's state by its
  id, on the synchronization channel with the plugin list.
- `backend-http`: `PUT /plugins/{plugin}/features/{feature} { enabled }`.
- `plugin-sdk`: `plugin.features` and `plugin.setFeature`; the switch shows
  the choice at once, and a refusal turns it back with a toast. The
  generated page types hold `FEATURES`, each feature's default.
- `plugin-web`'s page: the title Search, the two switches, then Exa's
  account and key; `web-gallery`: the page with search off and on.

## Spike

None: the host already composes a user's command set from the plugins on,
and a feature only narrows it.

## Prior art

- macOS System Settings opens a pane with a switch per part of a feature,
  such as Siri's and Spotlight's, each turning that part alone off.
- The Codex CLI and Claude Code search with their vendor's search and offer
  no second web search of their own, and both fetch pages.

## Alternatives

- **Exa as a plugin of its own**: search would leave the `web` group, and
  the command its name.
- **A setting inside the plugin that makes search fail while off**: the
  model still sees the command, and tries it.

## Open questions

None.
