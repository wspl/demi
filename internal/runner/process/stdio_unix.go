//go:build darwin || linux

package process

import (
	"context"
	"fmt"
	"os"

	"github.com/wspl/demi/internal/cmdsdk"
	"golang.org/x/sys/unix"
)

func standardFile(ctx context.Context, descriptor uint32) (*os.File, error) {
	fd := int(descriptor)
	if fd > 1 {
		fd = 2
	}
	duplicate, err := cmdsdk.Retry(
		ctx,
		func() (int, error) { return unix.FcntlInt(uintptr(fd), unix.F_DUPFD_CLOEXEC, 0) },
	)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(duplicate), "standard"), nil
}

func liveReference(file *os.File) (string, error) {
	var stat unix.Stat_t
	if err := unix.Fstat(int(file.Fd()), &stat); err != nil {
		return "", err
	}
	return fmt.Sprintf("%d:%d", stat.Dev, stat.Ino), nil
}

func matchesReference(file *os.File, reference string) (bool, error) {
	actual, err := liveReference(file)
	return actual == reference, err
}
