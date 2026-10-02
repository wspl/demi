//go:build linux && !fault_injection

package system_test

import (
	"testing"

	"github.com/wspl/demi/internal/machines/system"
)

func TestFaultPointsIgnoredInNormalBuild(t *testing.T) {
	tools := childTools(t)
	t.Setenv("DEMI_MACHINE_MANAGER_FAULT", "fixture")
	if _, err := tools.Run(t.Context(), system.Runsc, childArgs("fault"), nil); err != nil {
		t.Fatal(err)
	}
}
