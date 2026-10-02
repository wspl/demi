package cmdpkgs

import (
	"fmt"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/runner/process"
)

// serviceExitStatus renders the process owner's status as Rust's ExitStatus
// Display does on the service's platform. Signal names follow Rust's table,
// rather than Go's descriptive syscall.Signal.String output. The process
// record currently omits the core-dump bit, so that suffix cannot be rendered.
func serviceExitStatus(exit process.Exit, platform string) string {
	if exit.Code != nil {
		if platform == "windows" {
			code := uint32(*exit.Code)
			if code&0x80000000 != 0 {
				return fmt.Sprintf("exit code: %#x", code)
			}
			return fmt.Sprintf("exit code: %d", code)
		}
		return fmt.Sprintf("exit status: %d", *exit.Code)
	}
	if exit.Signal != nil {
		// Both supported Unix platforms number signals consecutively from one.
		names := strings.Fields("SIGHUP SIGINT SIGQUIT SIGILL SIGTRAP SIGABRT SIGBUS SIGFPE SIGKILL SIGUSR1 SIGSEGV SIGUSR2 SIGPIPE SIGALRM SIGTERM SIGSTKFLT SIGCHLD SIGCONT SIGSTOP SIGTSTP SIGTTIN SIGTTOU SIGURG SIGXCPU SIGXFSZ SIGVTALRM SIGPROF SIGWINCH SIGIO SIGPWR SIGSYS")
		if platform == "darwin" {
			names = strings.Fields("SIGHUP SIGINT SIGQUIT SIGILL SIGTRAP SIGABRT SIGEMT SIGFPE SIGKILL SIGBUS SIGSEGV SIGSYS SIGPIPE SIGALRM SIGTERM SIGURG SIGSTOP SIGTSTP SIGCONT SIGCHLD SIGTTIN SIGTTOU SIGIO SIGXCPU SIGXFSZ SIGVTALRM SIGPROF SIGWINCH SIGINFO SIGUSR1 SIGUSR2")
		}
		// The owner emits a known name or SIG followed by a decimal number.
		// Named signals fail parsing and are resolved by the table below.
		number, _ := strconv.Atoi(strings.TrimPrefix(*exit.Signal, "SIG"))
		for i, name := range names {
			if name == *exit.Signal {
				number = i + 1
				break
			}
		}
		if number > 0 && number <= len(names) {
			return fmt.Sprintf("signal: %d (%s)", number, names[number-1])
		}
		return fmt.Sprintf("signal: %d", number)
	}
	// No OS status exists when the process owner reports a wait failure.
	if exit.Error != nil {
		return *exit.Error
	}
	return "unknown exit status"
}
