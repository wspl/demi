package cmdsdk_test

import (
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
)

func TestCheckLaunchAcceptsOnlyTheServiceFlag(t *testing.T) {
	for _, args := range [][]string{nil, {"--help"}, {cmdsdk.CommandService, "extra"}} {
		if err := cmdsdk.CheckLaunch(args); err == nil {
			t.Fatalf("CheckLaunch(%q) accepted", args)
		}
	}
	if err := cmdsdk.CheckLaunch([]string{cmdsdk.CommandService}); err != nil {
		t.Fatal(err)
	}
}
