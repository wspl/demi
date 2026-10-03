//go:build linux && fault_injection

package system_test

import (
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machines/system"
)

func TestFaultPointAbortsOnlyMatchingPoint(t *testing.T) {
	tools := childTools(t)
	t.Setenv("DEMI_MACHINE_MANAGER_FAULT", "elsewhere")
	if _, err := tools.Run(t.Context(), system.Runsc, childArgs("fault"), nil); err != nil {
		t.Fatal(err)
	}
	t.Setenv("DEMI_MACHINE_MANAGER_FAULT", "fixture")
	_, err := tools.Run(t.Context(), system.Runsc, childArgs("fault"), nil)
	var failed *system.FailedError
	if !errors.As(err, &failed) ||
		!strings.Contains(failed.Output.Stderr, "demi-machine-manager: injected fault at fixture\n") {
		t.Fatalf("fault = %v", err)
	}
}
