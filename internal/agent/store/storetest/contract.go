package storetest

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"testing"

	"github.com/wspl/demi/internal/agent/store"
)

// StoreContract runs the eight shared tree-store scenarios. Each subtest gets
// a fresh, empty store from newStore; the factory registers resource cleanup
// on that subtest. Cases use t.Context and wait for events, never wall time.
func StoreContract(t *testing.T, newStore func(t *testing.T) store.TreeStore) {
	panic("not written: a-store")
}
