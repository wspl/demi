package databasetest

import (
	"testing"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/core"
)

// Fixture tests use temporary SQLite files and no external services.
func TestControlFixtureSupportsCorruption(t *testing.T) {
	c := Control(t.Context(), t, core.SystemClock{})
	master := Master(t.Context(), t, c)
	Execute(t.Context(), t, c, "UPDATE users SET email=? WHERE id=?", "corrupt", string(master.ID))
	if _, err := c.Users(t.Context()); err == nil {
		t.Fatal("corrupt email accepted")
	}
}

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
