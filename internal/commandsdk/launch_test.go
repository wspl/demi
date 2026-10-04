package commandsdk_test

import (
	"testing"

	"github.com/wspl/demi/internal/commandsdk"
)

func TestCheckLaunchAcceptsOnlyTheServiceFlag(t *testing.T) {
	for _, args := range [][]string{nil, {"--help"}, {commandsdk.CommandService, "extra"}} {
		if err := commandsdk.CheckLaunch(args); err == nil {
			t.Fatalf("CheckLaunch(%q) accepted", args)
		}
	}
	if err := commandsdk.CheckLaunch([]string{commandsdk.CommandService}); err != nil {
		t.Fatal(err)
	}
}
