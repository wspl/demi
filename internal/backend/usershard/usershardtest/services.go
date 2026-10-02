package usershardtest

//revive:disable:unused-parameter

import (
	"testing"

	"github.com/wspl/demi/internal/backend/usershard"
)

// StartServices opens temporary storage and shared services without provider
// families or a listening machine manager. Cleanup joins all workers and closes
// services and storage; start routing afterwards so its cleanup runs first.
func StartServices(t testing.TB) *usershard.Services { panic("not written: b-usershard") }

// StartServicesWithLifecycle supplies shortened idle and retention timing with
// the same ownership and cleanup as StartServices.
func StartServicesWithLifecycle(t testing.TB, lifecycle usershard.LifecycleTuning) *usershard.Services {
	panic("not written: b-usershard")
}

// StartShards starts routing over services and registers joined test cleanup.
func StartShards(t testing.TB, services *usershard.Services) *usershard.Shards {
	panic("not written: b-usershard")
}
