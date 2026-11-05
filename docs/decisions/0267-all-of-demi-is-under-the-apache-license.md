---
status: proposed
---

# All of Demi is under the Apache License 2.0

Supersedes: 0139 (What one agent needs is an Apache library), the part that
draws a license line between the libraries and the backend. The split of the
libraries from the backend stays.

## Problem

Demi has two licenses. Most of it is under the Apache License 2.0, and the
server and the web app (`crates/backend*`, `crates/server`,
`crates/machine-manager` and `packages/web`) are under the Demi License: the
Apache License with conditions that forbid offering them to others for a fee
and removing Demi's name and logo from the web app.

A company that wants to run Demi for its team reads both licenses, and its
license scanner reports `LicenseRef-Demi-License` as unknown, because no
standard identifier names it, so a lawyer reviews it before anyone installs
Demi. A contributor to the server agrees to one more clause, that The Demi
Authors may license the contribution to others under other terms, and a
contributor to the runner does not. A reader of the repository has to learn
which directory falls under which license.

## Decision

Every part of Demi, the server and the web app included, is under the Apache
License 2.0 alone, from the first release that holds this change.

*   `LICENSE-DEMI` and the `LICENSE` file in each of the server's and the web
    app's directories are deleted. The repository's `LICENSE` covers
    everything.
*   Every crate takes the workspace's `license = "Apache-2.0"`, and every
    package says `Apache-2.0`.
*   `NOTICE` says that Demi is under the Apache License 2.0.
*   A server release carries `LICENSE` and `NOTICE` at its root, and the
    release's assets carry them, with no `LICENSE-DEMI`.
*   The READMEs say that Demi is under the Apache License 2.0, and
    `CONTRIBUTING.md` says that a contribution is under it, as its section 5
    says.
*   The name and the logo stay The Demi Authors': section 6 of the Apache
    License grants no right to them.
*   A version released before this change keeps the license it was released
    under.

The libraries keep their split from the backend, which 0139 made for what a
part is for: the backend links the libraries, and no library depends on the
backend, as the dependency graphs in `ARCHITECTURE.md` show. Only the reason
that came from the license goes.

## Spike

None.

## Prior art

*   OpenAI's [Codex](https://github.com/openai/codex), an agent with a CLI, a
    runtime and its tools in one repository, is under the Apache License 2.0
    alone.

## Alternatives

*   **Keep the Demi License.** It forbids reselling Demi as a hosted service,
    at the cost the problem describes.
*   **A license that turns into Apache after a delay, such as the Functional
    Source License.** It forbids competing use for two years, and still is no
    open-source license that scanners accept.
*   **The AGPL for the server.** It makes a host publish its changes, and many
    companies do not allow it at all.
*   **MIT.** It has no patent grant, which the Apache License has.

## Open questions

None.
