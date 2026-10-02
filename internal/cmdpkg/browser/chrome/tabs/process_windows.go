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

// Rust's Windows process owner has no Unix group or inherited-marker sweep.
func (p *chromeProcess) terminate(ctx context.Context) error { return ctx.Err() }
