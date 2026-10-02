package runners

import (
	"testing"

	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

// The package suite uses tiny artifacts, loopback fixtures and virtual timers.
// Its measured race-test cost is under three seconds while the runner is a
// placeholder; the real-runner scenario has its separate ten-second budget.
func TestMain(m *testing.M) { goleak.VerifyTestMain(programTests{m}) }

type programTests struct{ m *testing.M }

func (p programTests) Run() int { return programtest.Run(p.m) }
