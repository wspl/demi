//go:build unix

package tabs

import (
	"context"
	"errors"
	"runtime"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"golang.org/x/sys/unix"
)

// terminate signals only this Chrome environment's group and marked helpers.
func (p *chromeProcess) terminate(ctx context.Context) error {
	bounded, cancel := context.WithTimeout(ctx, cdp.ControlTimeout)
	defer cancel()
	for {
		alive := false
		if p.command != nil && p.command.Process != nil {
			group := p.command.Process.Pid
			// Only this group's adopted children are reaped, after the leader's Wait.
			for {
				pid, err := unix.Wait4(-group, nil, unix.WNOHANG, nil)
				if errors.Is(err, unix.EINTR) {
					continue
				}
				if err != nil && !errors.Is(err, unix.ECHILD) {
					return err
				}
				if pid <= 0 {
					break
				}
			}
			err := unix.Kill(-group, unix.SIGKILL)
			switch {
			case err == nil:
				alive = true
			case errors.Is(err, unix.ESRCH):
			case runtime.GOOS == "darwin" && errors.Is(err, unix.EPERM):
				alive = true
			default:
				return err
			}
		}
		helpers, err := markedProcesses(p.installations, "DEMI_BROWSER_PROFILE="+p.runtime)
		if err != nil {
			return err
		}
		for _, pid := range helpers {
			alive = true
			if err := unix.Kill(pid, unix.SIGKILL); err != nil && !errors.Is(err, unix.ESRCH) {
				return err
			}
		}
		if !alive {
			return nil
		}
		timer := time.NewTimer(20 * time.Millisecond)
		select {
		case <-bounded.Done():
			timer.Stop()
			return &cdp.BrowserError{Kind: cdp.KindIO, Message: "Chrome process tree did not exit", Cause: bounded.Err()}
		case <-timer.C:
			timer.Stop()
		}
	}
}
