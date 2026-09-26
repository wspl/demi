package toolctxtest_test

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/toolctx/toolctxtest"
)

// A utility sees the runner's directory, environment and stdin, and reaches
// other utilities through Run.
func TestRunnerSuppliesTheInvocation(t *testing.T) {
	dir := t.TempDir()
	upper := func(inv *toolctx.Invocation, args []string) int {
		data, err := io.ReadAll(inv.Stdin)
		if err != nil {
			return 1
		}
		_, err = inv.Stdout.Write([]byte(string(data) + args[1]))
		if err != nil {
			return 1
		}
		return 0
	}
	write := func(inv *toolctx.Invocation, args []string) int {
		file, err := inv.Files.OpenFile(args[1], os.O_WRONLY|os.O_CREATE, 0o644)
		if err != nil {
			return 1
		}
		value, _ := inv.Env.Get("VALUE")
		_, err = file.Write([]byte(value))
		closeErr := file.Close()
		if err != nil || closeErr != nil {
			return 1
		}
		code, err := inv.Run(inv.Context, toolctx.Command{Args: []string{"upper", "!"}, Stdout: inv.Stdout})
		if err != nil {
			return 127
		}
		return code
	}
	runner := &toolctxtest.Runner{
		Dir:       dir,
		Env:       toolctxtest.Env{"VALUE": "hello"},
		Utilities: map[string]toolctx.Utility{"upper": upper},
	}

	result := runner.Run(context.Background(), write, []string{"write", "out.txt"}, "")

	if result.Code != 0 || result.Stdout != "!" {
		t.Fatalf("result = %+v", result)
	}
	data, err := os.ReadFile(filepath.Join(dir, "out.txt"))
	if err != nil || string(data) != "hello" {
		t.Fatalf("out.txt = %q, %v", data, err)
	}
}
