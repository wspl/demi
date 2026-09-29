package machines

import "github.com/wspl/demi/go/internal/wire"

// An InvalidError means a record breaks the rules of its wire type. It names the
// field and the rule, never the value.
type InvalidError = wire.InvalidError

// A namespaceOwner is the owner record next to the handle: whose state the
// pinned namespace holds.
//
//demi:wire
type namespaceOwner struct {
	DataDir string `json:"dataDir"`
}
