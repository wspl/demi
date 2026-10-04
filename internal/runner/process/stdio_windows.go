package process

import (
	"context"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/commandsdk"
	"golang.org/x/sys/windows"
)

func standardFile(ctx context.Context, descriptor uint32) (*os.File, error) {
	var file *os.File
	switch descriptor {
	case 0:
		file = os.Stdin
	case 1:
		file = os.Stdout
	default:
		file = os.Stderr
	}
	handle, err := commandsdk.Retry(ctx, func() (windows.Handle, error) {
		var result windows.Handle
		err := windows.DuplicateHandle(
			windows.CurrentProcess(),
			windows.Handle(file.Fd()),
			windows.CurrentProcess(),
			&result,
			0,
			false,
			windows.DUPLICATE_SAME_ACCESS,
		)
		return result, err
	})
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(handle), "standard"), nil
}

func liveReference(file *os.File) (string, error) {
	return fmt.Sprintf("%d:%d", os.Getpid(), file.Fd()), nil
}

var compareHandles = windows.NewLazySystemDLL("kernelbase.dll").NewProc("CompareObjectHandles")

func matchesReference(file *os.File, reference string) (bool, error) {
	pidText, handleText, ok := strings.Cut(reference, ":")
	if !ok {
		return false, fmt.Errorf("invalid live input reference")
	}
	pid, err := strconv.ParseUint(pidText, 10, 32)
	if err != nil {
		return false, fmt.Errorf("invalid live input reference: %w", err)
	}
	handle, err := strconv.ParseUint(handleText, 10, 64)
	if err != nil {
		return false, fmt.Errorf("invalid live input reference: %w", err)
	}
	process, err := windows.OpenProcess(windows.PROCESS_DUP_HANDLE, false, uint32(pid))
	if err != nil {
		return false, err
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = windows.CloseHandle(process)
	}()
	var duplicate windows.Handle
	if err := windows.DuplicateHandle(
		process,
		windows.Handle(handle),
		windows.CurrentProcess(),
		&duplicate,
		0,
		false,
		windows.DUPLICATE_SAME_ACCESS,
	); err != nil {
		return false, err
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = windows.CloseHandle(duplicate)
	}()
	// x/sys does not wrap CompareObjectHandles. Its BOOL is identity, not an
	// error indication; last-error is not part of this API's result.
	if err := compareHandles.Find(); err != nil {
		return false, err
	}
	result, _, _ := compareHandles.Call(file.Fd(), uintptr(duplicate))
	return result != 0, nil
}
