//go:build darwin || linux

package cmdpkgs

import (
	"fmt"
	"strconv"
	"strings"
	"syscall"

	"github.com/wspl/demi/internal/runner/process"
	"golang.org/x/sys/unix"
)

// platformExitStatus spells a Unix status `exit status: N`, `signal: N (SIGNAME)` or `signal: N`. The process record
// omits the core-dump bit; omitting that suffix is an accepted normalization.
func platformExitStatus(exit process.Exit) string {
	if exit.Code != nil {
		return fmt.Sprintf("exit status: %d", *exit.Code)
	}
	signal := *exit.Signal
	number := unix.SignalNum(signal)
	if number == 0 {
		digits, ok := strings.CutPrefix(signal, "SIG")
		if !ok {
			return fmt.Sprintf("invalid signal record: %q", signal)
		}
		parsed, err := strconv.Atoi(digits)
		if err != nil {
			return fmt.Sprintf("invalid signal record: %q (%v)", signal, err)
		}
		number = syscall.Signal(parsed)
	}
	if name := unix.SignalName(number); name != "" {
		return fmt.Sprintf("signal: %d (%s)", number, name)
	}
	return fmt.Sprintf("signal: %d", number)
}
