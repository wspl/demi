//go:build darwin || linux

package process

import (
	"bytes"
	"os"
	"os/exec"
	"testing"

	"golang.org/x/sys/unix"
)

// Real processes, normally below one second. The shell sets the launch limit
// before Go initializes; completion synchronizes every observation.
func TestLaunchLimits(t *testing.T) {
	if os.Getenv("DEMI_TEST_LAUNCH_LIMIT") != "256" {
		executable, err := os.Executable()
		if err != nil {
			t.Fatal(err)
		}
		cmd := exec.CommandContext(
			t.Context(),
			"/bin/sh",
			"-c",
			`ulimit -Sn 256 && exec "$1" -test.run='^TestLaunchLimits$' -test.count=1`,
			"sh",
			executable,
		)
		cmd.Env = append(os.Environ(), "DEMI_TEST_LAUNCH_LIMIT=256")
		if output, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("launch fixture: %v\n%s", err, output)
		}
		return
	}
	var original unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &original); err != nil {
		t.Fatal(err)
	}
	for range 2 {
		before, after, err := RaiseOpenFileLimit()
		if err != nil || before != 256 || after != original.Cur {
			t.Fatalf("raise report = %d/%d, %v; current %d", before, after, err, original.Cur)
		}
		soft, hard, err := ChildLimit(unix.RLIMIT_NOFILE)
		if err != nil || soft != 256 || hard != original.Max {
			t.Fatalf("child limit = %d/%d, %v", soft, hard, err)
		}
	}
	mask := uint32(0o77)
	for _, tc := range []struct {
		name       string
		attributes ChildAttributes
		want       string
	}{
		{"direct", ChildAttributes{}, "256\n"},
		{"bootstrap mask only", ChildAttributes{Umask: &mask}, "256\n"},
		{
			"bootstrap own limit",
			ChildAttributes{Limits: []ResourceLimit{{Resource: unix.RLIMIT_NOFILE, Soft: 128, Hard: original.Max}}},
			"128\n",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var output bytes.Buffer
			cmd := exec.Command("/bin/sh", "-c", "ulimit -Sn")
			cmd.Stdout = &output
			wrapped := Wrap(cmd, true, tc.attributes)
			if err := wrapped.Start(t.Context()); err != nil {
				t.Fatal(err)
			}
			if err := unsuccessfulExit(wrapped.Wait(t.Context())); err != nil {
				t.Fatal(err)
			}
			if output.String() != tc.want {
				t.Fatalf("limit = %q, want %q", output.String(), tc.want)
			}
		})
	}
	var current unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &current); err != nil {
		t.Fatal(err)
	}
	if current != original {
		t.Fatalf("runner limits changed: %v -> %v", original, current)
	}
	if err := unix.Getrlimit(unix.RLIMIT_CORE, &current); err != nil {
		t.Fatal(err)
	}
	soft, hard, err := ChildLimit(unix.RLIMIT_CORE)
	if err != nil || soft != current.Cur || hard != current.Max {
		t.Fatalf("core limits = %d/%d, %v; want %v", soft, hard, err, current)
	}
}

func TestParseOpenFiles(t *testing.T) {
	for _, input := range []string{
		"256\nunlimited\n",
		"256\n512\n",
		"unlimited\nunlimited\n",
	} {
		if _, err := parseOpenFiles(input); err != nil {
			t.Fatalf("%q: %v", input, err)
		}
	}
	for _, input := range []string{
		"",
		"256\n",
		"256\n512",
		"256\n512\nextra\n",
		" 256\n512\n",
		"+256\n512\n",
		"-1\n512\n",
		"256\n\n",
		"512\n256\n",
		"256\n18446744073709551616\n",
	} {
		if _, err := parseOpenFiles(input); err == nil {
			t.Fatalf("accepted %q", input)
		}
	}
}
