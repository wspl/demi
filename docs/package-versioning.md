# Package Versioning

Demi versions all first-party packages with one shared version. The Changesets
configuration places these packages in a single fixed group, so any
release-worthy change bumps the whole group together and publishes every public
package in the group.

## Release Set

The fixed group contains:

- `@demicodes/agent`
- `@demicodes/backend` (private)
- `@demicodes/coding-agent`
- `@demicodes/command-loader`
- `@demicodes/core`
- `@demicodes/host-remote`
- `@demicodes/host-virtual`
- `@demicodes/provider`
- `@demicodes/provider-anthropic-api`
- `@demicodes/provider-claude-code`
- `@demicodes/provider-codex`
- `@demicodes/provider-google`
- `@demicodes/provider-grok-build`
- `@demicodes/provider-openai-api`
- `@demicodes/runner`
- `@demicodes/runner-protocol`
- `@demicodes/shell`
- `@demicodes/tinybash`
- `@demicodes/utils`
- `@demicodes/web` (private)
- `@demicodes/web-ui`

Private workspaces participate in the shared version but are not published.

## Version Selection

Every changeset declares the semantic impact of its change. Changesets selects
the highest required bump for the fixed group and applies the resulting version
to every package in the release set. Package manifests, changelogs, npm
artifacts, and Git tags therefore expose the same Demi release version.

The repository release command publishes every group package whose shared
version is absent from the registry. It refreshes the lockfile, builds all
packages, validates packed dependency ranges and export targets, publishes the
tarballs, and creates one package tag per artifact.
