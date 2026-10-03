# Release scenarios

Local files and fixture HTTP servers; no model or vendor calls.

- Named targets must be each executable's; unnamed targets are all of theirs.
- Each runner release has its directory; the manifest names the last complete release.
- Backend and manager releases record their version and are immutable.
- Development releases carry selected targets and declared operations.
- Command releases carry resource archives, reuse their cache and reject corruption.
- Browser pins record every official archive and its executable; reject wrong metadata, unofficial URLs and missing entries.
- Image packaging embeds verified inputs and publishes a manager-readable manifest.
- Corrupt artifacts and unfinished dpkg installations publish no image.
- Fork comparison names changes and counts omitted upstream files, against a fixture module.
- Development backend signs in the seeded account, answers hello with Echo: hello, and stops every process and removes its data on interrupt (pending b-backend).

Rust-only SDK and sparse crates.io layout tests do not apply to Go. The five
Rust architecture tests are accounted for individually in the handoff report.
