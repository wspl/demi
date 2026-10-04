package toolstest_test

import (
	"testing"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
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
