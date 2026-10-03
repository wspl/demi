//go:build darwin || linux

package process

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/sys/unix"
)

type platformGroup struct{}

// startPlatform either executes directly or awaits the child-only attribute
// bootstrap's close-on-exec acknowledgement before publishing the process.
func startPlatform(ctx context.Context, template *exec.Cmd, group bool, attributes ChildAttributes, _ *platformGroup) (*exec.Cmd, error) {
	cmd := *template
	attr := syscall.SysProcAttr{}
	if cmd.SysProcAttr != nil {
		attr = *cmd.SysProcAttr
	}
	if group {
		attr.Setpgid = true
		attr.Pgid = 0
	}
	cmd.SysProcAttr = &attr
	if cmd.Err != nil {
		return nil, cmd.Err
	}
	if attributes.Umask == nil && len(attributes.Limits) == 0 {
		if err := cmd.Start(); err != nil {
			return nil, err
		}
		return &cmd, nil
	}
	reader, writer, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	defer func() { _ = reader.Close() }() // The handshake has no remaining consumer on return.
	defer func() { _ = writer.Close() }() // Cleanup follows the operation result; cancellation may already have closed it.
	executable, err := os.Executable()
	if err != nil {
		return nil, err
	}
	mask := "-"
	if attributes.Umask != nil {
		mask = strconv.FormatUint(uint64(*attributes.Umask), 10)
	}
	args := []string{executable, bootstrapArgument, strconv.Itoa(3 + len(cmd.ExtraFiles)), mask, strconv.Itoa(len(attributes.Limits))}
	for _, limit := range attributes.Limits {
		args = append(args, strconv.Itoa(limit.Resource), strconv.FormatUint(limit.Soft, 10), strconv.FormatUint(limit.Hard, 10))
	}
	args = append(args, cmd.Path)
	args = append(args, cmd.Args...)
	cmd.Path = executable
	cmd.Args = args
	cmd.ExtraFiles = append(append([]*os.File(nil), cmd.ExtraFiles...), writer)
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	_ = writer.Close() // Only the bootstrap's copy may keep the handshake open.
	interrupted := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		_ = (&platformGroup{}).kill(cmd.Process, group) // Reaped below; cancellation is the reported failure.
		_ = reader.Close()
		close(interrupted)
	})
	var code [4]byte
	count, readErr := io.ReadFull(reader, code[:])
	if !stop() {
		<-interrupted
	}
	if err := ctx.Err(); err != nil {
		_ = (&platformGroup{}).kill(cmd.Process, group) // Cancellation may have arrived after the callback was disarmed.
		_ = cmd.Wait()                                  // Reap even if cancellation raced with successful exec.
		return nil, err
	}
	if count == 0 && errors.Is(readErr, io.EOF) {
		return &cmd, nil
	}
	_ = (&platformGroup{}).kill(cmd.Process, group) // A failing bootstrap never escapes its owner.
	_ = cmd.Wait()
	if readErr != nil {
		return nil, fmt.Errorf("child bootstrap: %w", readErr)
	}
	return nil, &os.PathError{Op: "fork/exec", Path: template.Path, Err: syscall.Errno(binary.LittleEndian.Uint32(code[:]))}
}

// wait joins the owned group after exec.Cmd has reaped its leader. On Linux a
// subreaper can adopt grandchildren, so drain only this group's children; a
// process-wide wait could steal another command's exit status. ECHILD alone
// does not mean completion: another parent (including init on macOS) may still
// be reaping a member. Signal 0 keeps zombies in the completion predicate.
func (*platformGroup) wait(ctx context.Context, process *os.Process) error {
	for {
		pid, err := unix.Wait4(-process.Pid, nil, unix.WNOHANG, nil)
		if errors.Is(err, unix.EINTR) || pid > 0 {
			continue
		}
		if err != nil && !errors.Is(err, unix.ECHILD) {
			return fmt.Errorf("reap process group: %w", err)
		}
		err = unix.Kill(-process.Pid, 0)
		if errors.Is(err, unix.ESRCH) {
			return nil
		}
		if err != nil && !errors.Is(err, unix.EPERM) {
			return fmt.Errorf("inspect process group: %w", err)
		}
		// There is no waitable child event for members still owned by another
		// parent. Poll their group without a termination deadline.
		timer := time.NewTimer(10 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		case <-timer.C:
		}
	}
}

func (*platformGroup) close() {}
func (p *platformGroup) kill(process *os.Process, group bool) error {
	if group {
		return groupSignal(process.Pid, unix.SIGKILL)
	}
	err := process.Kill()
	if errors.Is(err, os.ErrProcessDone) {
		return nil
	}
	return err
}
func (*platformGroup) signal(process *os.Process, group bool, signal runnerwire.Signal) error {
	number, ok := signalNumber(signal)
	if !ok {
		return fmt.Errorf("unsupported process signal: %s", signal)
	}
	if group {
		return groupSignal(process.Pid, number)
	}
	err := process.Signal(number)
	if errors.Is(err, os.ErrProcessDone) {
		return nil
	}
	return err
}

// groupSignal treats the already-ending group as successfully signalled.
func groupSignal(pid int, signal syscall.Signal) error {
	err := unix.Kill(-pid, signal)
	if errors.Is(err, unix.ESRCH) || runtime.GOOS == "darwin" && errors.Is(err, unix.EPERM) {
		return nil
	}
	return err
}
func signalNumber(signal runnerwire.Signal) (syscall.Signal, bool) {
	switch signal {
	case runnerwire.SignalTerminate:
		return unix.SIGTERM, true
	case runnerwire.SignalKill:
		return unix.SIGKILL, true
	case runnerwire.SignalInterrupt:
		return unix.SIGINT, true
	case runnerwire.SignalHangup:
		return unix.SIGHUP, true
	case runnerwire.SignalQuit:
		return unix.SIGQUIT, true
	case runnerwire.SignalUser1:
		return unix.SIGUSR1, true
	case runnerwire.SignalUser2:
		return unix.SIGUSR2, true
	case runnerwire.SignalStop:
		return unix.SIGSTOP, true
	case runnerwire.SignalContinue:
		return unix.SIGCONT, true
	}
	return 0, false
}
func exitSignal(state *os.ProcessState) *string {
	status, ok := state.Sys().(syscall.WaitStatus)
	if !ok || !status.Signaled() {
		return nil
	}
	signal := status.Signal()
	for _, candidate := range []runnerwire.Signal{runnerwire.SignalTerminate, runnerwire.SignalKill, runnerwire.SignalInterrupt, runnerwire.SignalHangup, runnerwire.SignalQuit, runnerwire.SignalUser1, runnerwire.SignalUser2, runnerwire.SignalStop, runnerwire.SignalContinue} {
		if number, _ := signalNumber(candidate); signal == number {
			name := string(candidate)
			return &name
		}
	}
	name := fmt.Sprintf("SIG%d", signal)
	if signal == unix.SIGPIPE {
		name = "SIGPIPE"
	}
	return &name
}
