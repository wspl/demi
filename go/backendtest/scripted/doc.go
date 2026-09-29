// Package scripted holds what the black-box suite hosts itself
// (docs/internal/go-migration/design/g6-api-suite.md § What the suite hosts
// itself): a machine manager that speaks machines-protocol on a socket the
// backend is configured with and runs each Cloud's sandbox as a real runner
// process, and the model vendors, HTTP servers that answer the requests a
// scenario scripts and record every one. Neither needs a hook in the backend.
package scripted
