package scenarios_test

import (
	"testing"

	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

// TestMain checks that no scenario of the backend's test binary leaves a
// goroutine behind.
func TestMain(m *testing.M) { goleak.VerifyTestMain(programTests{m}) }

// programTests releases the backend scenarios' shared program builds before leak checking.
type programTests struct{ m *testing.M }

// Run releases shared program builds after running the scenarios.
func (p programTests) Run() int { return programtest.Run(p.m) }
