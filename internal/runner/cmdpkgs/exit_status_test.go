package cmdpkgs

import (
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runner/process"
)

func TestServiceExitStatusRustDisplay(t *testing.T) {
	for _, tt := range []struct {
		name     string
		platform string
		code     int32
		signal   string
		want     string
	}{
		{name: "unix success", platform: "linux", want: "exit status: 0"},
		{name: "unix failure", platform: "darwin", code: 3, want: "exit status: 3"},
		{name: "windows failure", platform: "windows", code: 3, want: "exit code: 3"},
		{name: "windows exception", platform: "windows", code: -1073741819, want: "exit code: 0xc0000005"},
		{name: "darwin kill", platform: "darwin", signal: "SIGKILL", want: "signal: 9 (SIGKILL)"},
		{name: "darwin unknown", platform: "darwin", signal: "SIG34", want: "signal: 34"},
		{name: "kill", platform: "linux", signal: "SIGKILL", want: "signal: 9 (SIGKILL)"},
		{name: "numeric known", platform: "linux", signal: "SIG11", want: "signal: 11 (SIGSEGV)"},
		{name: "linux user", platform: "linux", signal: "SIGUSR1", want: "signal: 10 (SIGUSR1)"},
		{name: "darwin user", platform: "darwin", signal: "SIGUSR1", want: "signal: 30 (SIGUSR1)"},
		{name: "darwin numeric", platform: "darwin", signal: "SIG7", want: "signal: 7 (SIGEMT)"},
		{name: "unknown signal", platform: "linux", signal: "SIG34", want: "signal: 34"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			if runtime.GOOS != tt.platform {
				t.Skip("status belongs to " + tt.platform)
			}
			exit := process.Exit{Code: &tt.code}
			if tt.signal != "" {
				exit = process.Exit{Signal: &tt.signal}
			}
			if got := serviceExitStatus(exit); got != tt.want {
				t.Fatalf("status = %q; want %q", got, tt.want)
			}
		})
	}
}

func TestMalformedSignalRecordIsReported(t *testing.T) {
	for _, signal := range []string{"KILL", "SIGbogus", "SIG", "SIG999999999999999999999999999999999999"} {
		t.Run(signal, func(t *testing.T) {
			got := serviceExitStatus(process.Exit{Signal: &signal})
			if !strings.HasPrefix(got, "invalid signal record: ") || !strings.Contains(got, signal) {
				t.Fatalf("malformed signal was not reported: %q", got)
			}
		})
	}
}
