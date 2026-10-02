package runner

import (
	"errors"

	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runnerwire"
)

// message routes authenticated work; every blocking operation belongs to a child.
func (c *connection) message(message runnerwire.Inbound) error {
	r := c.registration
	var work func() error
	// Relay and authentication variants have already been consumed by route.
	switch m := any(message).(type) {
	case *runnerwire.ConversationRelease:
		work = func() error {
			err := r.services.ReleaseConversation(c.ctx, m.ConversationID)
			reply := &runnerwire.ConversationReleased{ID: m.ID}
			if err != nil {
				text := err.Error()
				reply.Error = &text
			}
			return c.send(c.ctx, reply)
		}
	case *runnerwire.ManifestMessage:
		c.generation++
		generation := c.generation
		c.installation.Publish(jobs.ManifestInstalling, nil)
		c.launch(func() connectionWork {
			installed, err := jobs.Install(c.ctx, m.Manifest, r.paths, r.services, r.reserved)
			return connectionWork{installation: generation, installed: installed, err: err}
		})
		return nil
	case *runnerwire.JobRead:
		work = func() error { return c.readJob(m) }
	case *runnerwire.JobRelease:
		work = func() error {
			c.directories.Release(c.ctx, m.JobID)
			return nil
		}
	case *runnerwire.Sync:
		work = func() error { return c.volumes.Sync(c.ctx, *m) }
	case *runnerwire.VolumeGrown:
		return c.volumes.Grown(*m)
	case *runnerwire.FSExists:
		work = func() error { return c.host.Exists(c.ctx, *m) }
	case *runnerwire.FSStat:
		work = func() error { return c.host.Stat(c.ctx, *m) }
	case *runnerwire.FSLstat:
		work = func() error { return c.host.Lstat(c.ctx, *m) }
	case *runnerwire.FSReadFile:
		work = func() error { return c.host.ReadFile(c.ctx, *m) }
	case *runnerwire.FSWriteFile:
		work = func() error { return c.host.WriteFile(c.ctx, *m) }
	case *runnerwire.FSReaddir:
		work = func() error { return c.host.Readdir(c.ctx, *m) }
	case *runnerwire.FSMkdir:
		work = func() error { return c.host.Mkdir(c.ctx, *m) }
	case *runnerwire.FSRm:
		work = func() error { return c.host.Rm(c.ctx, *m) }
	case *runnerwire.FSCp:
		work = func() error { return c.host.Cp(c.ctx, *m) }
	case *runnerwire.FSMv:
		work = func() error { return c.host.Mv(c.ctx, *m) }
	case *runnerwire.FSChmod:
		work = func() error { return c.host.Chmod(c.ctx, *m) }
	case *runnerwire.FSSymlink:
		work = func() error { return c.host.Symlink(c.ctx, *m) }
	case *runnerwire.FSLink:
		work = func() error { return c.host.Link(c.ctx, *m) }
	case *runnerwire.FSReadlink:
		work = func() error { return c.host.Readlink(c.ctx, *m) }
	case *runnerwire.FSRealpath:
		work = func() error { return c.host.Realpath(c.ctx, *m) }
	case *runnerwire.FSUtimes:
		work = func() error { return c.host.Utimes(c.ctx, *m) }
	case *runnerwire.GitChangesMessage:
		work = func() error { return c.host.GitChanges(c.ctx, *m) }
	case *runnerwire.GitShow:
		work = func() error { return c.host.GitShow(c.ctx, *m) }
	case *runnerwire.NetOpen:
		work = func() error { return c.host.NetOpen(c.ctx, *m) }
	case *runnerwire.ServiceOpen:
		return c.streams.HandleOpen(m)
	case *runnerwire.LogRead:
		work = func() error {
			page, err := r.options.log.read(c.ctx, logQuery{since: m.Since, limit: int(m.Limit), source: m.Source})
			if err != nil {
				return c.send(c.ctx, &runnerwire.LogError{ID: m.ID, Message: err.Error()})
			}
			lines := make([]runnerwire.LogLine, 0, len(page.lines))
			for _, line := range page.lines {
				lines = append(lines, runnerwire.LogLine{At: runnerwire.Timestamp(line.At), Source: line.Source, ConversationID: line.ConversationID, Text: line.Text})
			}
			return c.send(c.ctx, &runnerwire.LogLines{ID: m.ID, Lines: lines, Next: page.next})
		}
	default:
		return c.task(message)
	}
	c.launch(func() connectionWork { return connectionWork{err: work()} })
	return nil
}

func (c *connection) task(message runnerwire.Inbound) error {
	environment := taskEnvironment{values: c.registration.options.env, cwd: c.registration.options.cwd, home: c.registration.options.runner.Identity.HomeDir, installation: c.registration.state.root}
	var id jobs.WorkID
	var spec jobs.TaskSpec
	// Task controls are the remaining subset after authenticated Host routing.
	switch m := any(message).(type) {
	case *runnerwire.Spawn:
		id = jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID}
		spec = environment.processSpec(m)
	case *runnerwire.JobStart:
		id = jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}
		spec = environment.shellSpec(m)
	case *runnerwire.SpawnStdin:
		return c.table.Input(jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID}, m.Bytes)
	case *runnerwire.JobStdin:
		return c.table.Input(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}, m.Bytes)
	case *runnerwire.SpawnStdinEnd:
		return c.table.EndInput(jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID})
	case *runnerwire.JobStdinEnd:
		return c.table.EndInput(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID})
	case *runnerwire.SpawnKill:
		signal := runnerwire.SignalTerminate
		if m.Signal != nil {
			signal = *m.Signal
		}
		return c.table.Signal(jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID}, signal)
	case *runnerwire.JobKill:
		c.contexts.Cancel(m.JobID)
		signal := runnerwire.SignalTerminate
		if m.Signal != nil {
			signal = *m.Signal
		}
		return c.table.Signal(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}, signal)
	case *runnerwire.JobFollow:
		c.table.Follow(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}, m.Follow)
		return nil
	default:
		return errors.New("unexpected backend message")
	}
	var err error
	if c.registration.management.snapshot().Draining {
		err = errors.New("runner is draining for upgrade")
	} else {
		err = c.table.Start(spec)
	}
	c.registration.management.setJobs(c.table.JobCount())
	if err != nil {
		frame, encodeErr := jobs.FailureExit(id, err.Error())
		if encodeErr != nil {
			return encodeErr
		}
		return c.sendFrame(c.ctx, frame)
	}
	select {
	case c.tasksChanged <- struct{}{}:
	default:
	}
	return nil
}
