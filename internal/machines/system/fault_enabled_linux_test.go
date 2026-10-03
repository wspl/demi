//go:build linux && fault_injection

package system_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machines/system"
)

func TestFaultPointAbortsOnlyMatchingPoint(t *testing.T) {
	tools := childTools(t)
	t.Setenv("DEMI_MACHINE_MANAGER_FAULT", "elsewhere")
	if _, err := tools.Run(t.Context(), system.Runsc, childArgs("fault"), 0); err != nil {
		t.Fatal(err)
	}
	t.Setenv("DEMI_MACHINE_MANAGER_FAULT", "fixture")
	output, err := tools.Output(t.Context(), system.Runsc, childArgs("fault"), 0)
	if _, rejected := system.Accept(system.Runsc, output, []int{0}); err != nil || rejected == nil ||
		!strings.Contains(output.Stderr, "demi-machine-manager: injected fault at fixture\n") {
		t.Fatalf("fault = %+v, %v", output, err)
	}
}
