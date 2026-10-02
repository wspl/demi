//go:build darwin || linux

package runner

import (
	"fmt"
	"log/slog"
	"os"
	"syscall"

	"github.com/wspl/demi/internal/runner/process"
)

func startupLimits() {
	before, after, err := process.RaiseOpenFileLimit()
	if err != nil {
		slog.Warn("the open-file limit could not be raised: " + err.Error())
	} else {
		slog.Info(fmt.Sprintf("open-file limit %d (started with %d)", after, before))
	}
	_ = process.Umask()
}
func terminationSignal() os.Signal { return syscall.SIGTERM }
