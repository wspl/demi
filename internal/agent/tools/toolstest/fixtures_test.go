package toolstest_test

import (
	"errors"
	"testing"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/host"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

func TestNoHostRefusesResolution(t *testing.T) {
	var resolver toolstest.NoHost
	target, err := resolver.Host(t.Context(), tools.NodeContext{})
	var failure *host.Error
	if target != nil || !errors.As(err, &failure) || failure.Kind != host.Unavailable ||
		failure.Message != "this agent runs no shell tools" {
		t.Fatalf("unexpected refusal: %v", err)
	}
}

func TestResultReaders(t *testing.T) {
	result := "status: running\ncommandId: 17\noutput:\nline\nprompt\n[... 10 bytes not shown so " +
		"far; the newest: demi shell output 17 --tail 50 ...]\nnext: check"
	if got := toolstest.Field(result, "commandId"); got != "17" {
		t.Fatal(got)
	}
	if got := toolstest.ShownOutput(result); got != "line\nprompt\n" {
		t.Fatal(got)
	}
	if got := toolstest.ShownOutput("output: (empty)"); got != "" {
		t.Fatal(got)
	}
	defer func() {
		if recover() == nil {
			t.Error("missing field did not fail test reader")
		}
	}()
	toolstest.Field(result, "missing")
}
