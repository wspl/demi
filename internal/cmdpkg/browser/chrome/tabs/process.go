package tabs

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// chromeProcess retains Chrome's process group and marked detached helpers.
type chromeProcess struct {
	runtime       string
	installations []string
	command       *exec.Cmd
	done          chan struct{}
	waitErr       error // Written before done closes.
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

// killLeader reaps the launched Chrome even when the CDP connection has gone.
func (p *chromeProcess) killLeader(ctx context.Context) error {
	if p.command == nil || p.command.Process == nil {
		return nil
	}
	select {
	case <-p.done:
		return nil
	default:
	}
	if err := p.command.Process.Kill(); err != nil && !errors.Is(err, os.ErrProcessDone) {
		return err
	}
	select {
	case <-p.done:
		return nil // A requested kill has no useful exit status.
	case <-ctx.Done():
		return ctx.Err()
	}
}

// retire ends the Chrome leader before draining the group and detached helpers.
func (p *chromeProcess) retire(ctx context.Context) error {
	return cdp.AfterCleanup(p.killLeader(ctx), p.terminate(ctx))
}
