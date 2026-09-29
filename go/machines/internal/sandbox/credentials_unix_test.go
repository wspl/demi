//go:build unix

package sandbox_test

import (
	"errors"
	"io/fs"
	"net/netip"
	"os"
	"syscall"
	"testing"

	"github.com/wspl/demi/go/machines/internal/sandbox"
	"github.com/wspl/demi/go/runnerproto"
)

func TestCredentialModesDoNotDependOnTheUmask(t *testing.T) {
	// The service runs with umask 077; this process has its own now.
	previous := syscall.Umask(0o077)
	defer syscall.Umask(previous)
	directory := sandbox.NewRuntimeDirectory(t.TempDir(), "demi-test")
	if err := directory.Create(); err != nil {
		t.Fatal(err)
	}
	boot, err := runnerproto.DecodeManagedBoot([]byte(`{"backendUrl":"https://backend.example.com","deviceToken":"tok"}`))
	if err != nil {
		t.Fatal(err)
	}
	// Giving the record to the sandbox's user needs root, and comes last.
	written := directory.WriteCredentials(boot, []netip.Addr{netip.MustParseAddr("1.1.1.1")})
	if written != nil && !errors.Is(written, fs.ErrPermission) {
		t.Fatal(written)
	}
	mode := func(path string) (fs.FileMode, *syscall.Stat_t) {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		return info.Mode().Perm(), info.Sys().(*syscall.Stat_t)
	}
	if got, stat := mode(directory.Boot()); got != 0o400 || written == nil && stat.Uid != sandbox.UserID {
		t.Errorf("boot record: mode %o, owner %d", got, stat.Uid)
	}
	if got, _ := mode(directory.Resolver()); got != 0o444 {
		t.Errorf("resolver: mode %o", got)
	}
	if got, _ := mode(directory.Hosts()); got != 0o444 {
		t.Errorf("hosts: mode %o", got)
	}
	if resolver, err := os.ReadFile(directory.Resolver()); err != nil || string(resolver) != "nameserver 1.1.1.1\n" {
		t.Errorf("resolver %q, %v", resolver, err)
	}
	record, err := os.ReadFile(directory.Boot())
	if err != nil {
		t.Fatal(err)
	}
	if decoded, err := runnerproto.DecodeManagedBoot(record); err != nil || decoded.DeviceToken != "tok" {
		t.Errorf("the boot record: %s, %v", record, err)
	}
}
