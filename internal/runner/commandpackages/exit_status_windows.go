//go:build windows

package commandpackages

import (
	"fmt"

	"github.com/wspl/demi/internal/runner/process"
)

// platformExitStatus spells a Windows status `exit code: N`, in hex (`exit code: 0xc0000005`)
// when the high bit marks an exception code. Windows process records do not carry signals.
func platformExitStatus(exit process.Exit) string {
	if exit.Code == nil {
		return fmt.Sprintf("invalid signal record: %q", *exit.Signal)
	}
	code := uint32(*exit.Code)
	if code&0x80000000 != 0 {
		return fmt.Sprintf("exit code: %#x", code)
	}
	return fmt.Sprintf("exit code: %d", code)
}
