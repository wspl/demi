//go:build unix

package shell

import (
	"context"
	"fmt"
	"os"
	"syscall"

	"github.com/wspl/demi/go/runner/process"
	"golang.org/x/sys/unix"
)

func executeHelper(setup childSetup) error {
	if setup.Mode == "umask" {
		fmt.Fprintln(os.Stdout, unix.Umask(0))
		return nil
	}
	if setup.Mask != nil {
		unix.Umask(int(*setup.Mask))
	}
	for _, limit := range setup.Limits {
		if err := syscall.Setrlimit(limit.Resource, &syscall.Rlimit{Cur: limit.Soft, Max: limit.Hard}); err != nil {
			return fmt.Errorf("%s (os error %d)", capitalize(err.Error()), err)
		}
	}
	if setup.Mode == "probe" {
		return nil
	}
	_, err := process.Start(context.Background(), func() (struct{}, error) {
		return struct{}{}, syscall.Exec(setup.Path, setup.Args, os.Environ())
	})
	return err
}

func resourceLimit(ctx context.Context, resource int) (uint64, uint64, error) {
	if resource == unix.RLIMIT_NOFILE {
		inheritedFiles.once.Do(func() {
			inheritedFiles.limit, inheritedFiles.err = readInheritedOpenFiles(ctx, "/bin/sh")
		})
		return inheritedFiles.limit.Cur, inheritedFiles.limit.Max, inheritedFiles.err
	}
	var limit unix.Rlimit
	err := unix.Getrlimit(resource, &limit)
	return limit.Cur, limit.Max, err
}

func infinity() uint64 { return uint64(unix.RLIM_INFINITY) }
