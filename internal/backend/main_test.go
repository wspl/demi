package backend_test

import (
	"testing"

	"go.uber.org/goleak"
)

// TestMain checks that no scenario of the backend's test binary leaves a
// goroutine behind.
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
