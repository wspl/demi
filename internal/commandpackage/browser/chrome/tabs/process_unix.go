//go:build unix

package tabs

import (
	"context"
	"errors"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"golang.org/x/sys/unix"
)

// terminate signals only this Chrome environment's group and marked helpers.
func (p *chromeProcess) terminate(ctx context.Context) error {
	bounded, cancel := context.WithTimeout(ctx, cdp.ControlTimeout)
	defer cancel()
	for {
		alive, err := p.killGroup()
		if err != nil {
			return err
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
			return &cdp.BrowserError{
				Kind:    cdp.KindIO,
				Message: "Chrome process tree did not exit",
				Cause:   bounded.Err(),
			}
		case <-timer.C:
			timer.Stop()
		}
	}
}

// installedProcess narrows inspection to Chrome installations before reading markers.
func installedProcess(roots []string, path string) bool {
	for _, root := range roots {
		relative, err := filepath.Rel(root, path)
		if err == nil && relative != ".." && !strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
			return true
		}
	}
	return false
}

func (p *chromeProcess) killGroup() (bool, error) {
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
				return false, err
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
			return false, err
		}
	}

	return alive, nil
}
