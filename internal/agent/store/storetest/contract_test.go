package storetest_test

import (
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
)

// The in-memory store passes the same contract as the database store, so
// tests that use it observe the behavior the product's store has.
func TestStoreContract(t *testing.T) {
	storetest.StoreContract(t, func(_ *testing.T) store.Tree { return storetest.NewMemoryTreeStore() })
}
