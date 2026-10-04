package runners

import (
	"testing"

	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

// The package suite uses tiny artifacts, loopback fixtures and virtual timers.
// The real-runner scenario has a ten-second budget, excluding its program build.
func TestMain(m *testing.M) {
	goleak.VerifyTestMain(programTests{m})
}

type programTests struct{ m *testing.M }

func (p programTests) Run() int {
	return programtest.Run(p.m)
}
