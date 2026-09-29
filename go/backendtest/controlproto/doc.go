// Package controlproto is the contract between a black-box suite and the test
// build of the backend it runs (docs/internal/go-migration/design/g6-api-suite.md
// § The test control socket): the control socket's messages and their line
// codec, and the tuning file. The Rust backend's testing feature and the Go
// backend under its build tag both serve it, and the suite speaks it, so each
// is defined once.
//
// The types are declared for cmd/wiregen (go generate). The requests are an
// adjacently tagged union of op and params flattened into the request, as the
// machine manager's are.
//
// A test build opens the control socket once the backend serves, so a
// connection to it tells a suite the backend is ready. A hold or a lease
// belongs to the connection that took it, and ends when it is released or the
// connection closes. Replies come in the order the requests finish, so a
// request that waits does not hold up the ones behind it.
package controlproto
