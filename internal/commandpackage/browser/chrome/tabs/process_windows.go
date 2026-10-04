package tabs

import (
	"context"
	"os/exec"
)

func (p *chromeProcess) start(command *exec.Cmd, _ string) error {
	if err := command.Start(); err != nil {
		return err
	}
	p.command = command
	p.done = make(chan struct{})
	go func() {
		p.waitErr = command.Wait()
		close(p.done)
	}()
	return nil
}

// Windows has no process group or inherited-marker sweep to terminate.
func (p *chromeProcess) terminate(ctx context.Context) error { return ctx.Err() }
