package runner

import (
	"errors"

	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runnerproto"
)

// message routes authenticated work; every blocking operation belongs to a child.
func (c *connection) message(message runnerproto.Inbound) error {
	var work func() error
	// Relay and authentication variants have already been consumed by route.
	switch m := any(message).(type) {
	case *runnerproto.ConversationRelease:
		work = func() error {
			return c.releaseConversation(m)
		}
	case *runnerproto.ManifestMessage:
		c.installManifest(m)
		return nil
	case *runnerproto.JobRead:
		work = func() error {
			return c.readJob(m)
		}
	case *runnerproto.JobRelease:
		work = func() error {
			c.directories.Release(c.ctx, m.JobID)
			return nil
		}
	case *runnerproto.Sync:
		work = func() error {
			return c.volumes.Sync(c.ctx, *m)
		}
	case *runnerproto.VolumeGrown:
		return c.volumes.Grown(*m)
	case *runnerproto.GitChangesMessage:
		work = func() error {
			return c.host.GitChanges(c.ctx, *m)
		}
	case *runnerproto.GitShow:
		work = func() error {
			return c.host.GitShow(c.ctx, *m)
		}
	case *runnerproto.NetOpen:
		work = func() error {
			return c.host.NetOpen(c.ctx, *m)
		}
	case *runnerproto.ServiceOpen:
		return c.streams.HandleOpen(m)
	case *runnerproto.LogRead:
		work = func() error {
			return c.readLog(m)
		}
	default:
		work = c.filesystemWork(message)
		if work == nil {
			return c.task(message)
		}
	}
	c.launch(func() connectionWork {
		return connectionWork{err: work()}
	})
	return nil
}

func (c *connection) task(message runnerproto.Inbound) error {
	environment := taskEnvironment{
		values:       c.registration.options.env,
		cwd:          c.registration.options.cwd,
		home:         c.registration.options.runner.Identity.HomeDir,
		installation: c.registration.state.root,
	}
	var id jobs.WorkID
	var spec jobs.TaskSpec
	// Task controls are the remaining subset after authenticated Host routing.
	switch m := any(message).(type) {
	case *runnerproto.Spawn:
		id = jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID}
		spec = environment.processSpec(m)
	case *runnerproto.JobStart:
		id = jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}
		spec = environment.shellSpec(m)
	case *runnerproto.SpawnStdin:
		return c.table.Input(jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID}, m.Bytes)
	case *runnerproto.JobStdin:
		return c.table.Input(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}, m.Bytes)
	case *runnerproto.SpawnStdinEnd:
		return c.table.EndInput(jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID})
	case *runnerproto.JobStdinEnd:
		return c.table.EndInput(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID})
	case *runnerproto.SpawnKill:
		signal := runnerproto.SignalTerminate
		if m.Signal != nil {
			signal = *m.Signal
		}
		return c.table.Signal(jobs.WorkID{Kind: jobs.ProcessWork, ID: m.SpawnID}, signal)
	case *runnerproto.JobKill:
		c.contexts.Cancel(m.JobID)
		signal := runnerproto.SignalTerminate
		if m.Signal != nil {
			signal = *m.Signal
		}
		return c.table.Signal(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}, signal)
	case *runnerproto.JobFollow:
		c.table.Follow(jobs.WorkID{Kind: jobs.ShellWork, ID: m.JobID}, m.Follow)
		return nil
	default:
		return errors.New("unexpected backend message")
	}
	return c.startTask(id, spec)
}

func (c *connection) filesystemWork(message runnerproto.Inbound) (work func() error) {
	switch m := any(message).(type) {
	case *runnerproto.FSExists:
		work = func() error {
			return c.host.Exists(c.ctx, *m)
		}
	case *runnerproto.FSStat:
		work = func() error {
			return c.host.Stat(c.ctx, *m)
		}
	case *runnerproto.FSLstat:
		work = func() error {
			return c.host.Lstat(c.ctx, *m)
		}
	case *runnerproto.FSReadFile:
		work = func() error {
			return c.host.ReadFile(c.ctx, *m)
		}
	case *runnerproto.FSWriteFile:
		work = func() error {
			return c.host.WriteFile(c.ctx, *m)
		}
	case *runnerproto.FSReaddir:
		work = func() error {
			return c.host.Readdir(c.ctx, *m)
		}
	case *runnerproto.FSMkdir:
		work = func() error {
			return c.host.Mkdir(c.ctx, *m)
		}
	case *runnerproto.FSRm:
		work = func() error {
			return c.host.Rm(c.ctx, *m)
		}
	case *runnerproto.FSCp:
		work = func() error {
			return c.host.Cp(c.ctx, *m)
		}
	case *runnerproto.FSMv:
		work = func() error {
			return c.host.Mv(c.ctx, *m)
		}
	default:
		return c.filesystemMetadataWork(message)
	}
	return work
}

func (c *connection) readLog(m *runnerproto.LogRead) error {
	page, err := c.registration.options.log.read(c.ctx, logQuery{since: m.Since, limit: int(m.Limit), source: m.Source})
	if err != nil {
		return c.send(c.ctx, &runnerproto.LogError{ID: m.ID, Message: err.Error()})
	}
	lines := make([]runnerproto.LogLine, 0, len(page.lines))
	for _, line := range page.lines {
		lines = append(
			lines,
			runnerproto.LogLine{
				At:             runnerproto.Timestamp(line.At),
				Source:         line.Source,
				ConversationID: line.ConversationID,
				Text:           line.Text,
			},
		)
	}
	return c.send(c.ctx, &runnerproto.LogLines{ID: m.ID, Lines: lines, Next: page.next})
}

func (c *connection) startTask(id jobs.WorkID, spec jobs.TaskSpec) error {
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

func (c *connection) releaseConversation(m *runnerproto.ConversationRelease) error {
	err := c.registration.services.ReleaseConversation(c.ctx, m.ConversationID)
	reply := &runnerproto.ConversationReleased{ID: m.ID}
	if err != nil {
		text := err.Error()
		reply.Error = &text
	}
	return c.send(c.ctx, reply)
}

func (c *connection) filesystemMetadataWork(message runnerproto.Inbound) (work func() error) {
	switch m := any(message).(type) {
	case *runnerproto.FSChmod:
		work = func() error {
			return c.host.Chmod(c.ctx, *m)
		}
	case *runnerproto.FSSymlink:
		work = func() error {
			return c.host.Symlink(c.ctx, *m)
		}
	case *runnerproto.FSLink:
		work = func() error {
			return c.host.Link(c.ctx, *m)
		}
	case *runnerproto.FSReadlink:
		work = func() error {
			return c.host.Readlink(c.ctx, *m)
		}
	case *runnerproto.FSRealpath:
		work = func() error {
			return c.host.Realpath(c.ctx, *m)
		}
	case *runnerproto.FSUtimes:
		work = func() error {
			return c.host.Utimes(c.ctx, *m)
		}
	}
	return work
}

func (c *connection) installManifest(m *runnerproto.ManifestMessage) {
	c.generation++
	generation := c.generation
	c.installation.Publish(jobs.ManifestInstalling, nil)
	c.launch(func() connectionWork {
		installed, err := jobs.Install(
			c.ctx,
			m.Manifest,
			c.registration.paths,
			c.registration.services,
			c.registration.reserved,
		)
		return connectionWork{installation: generation, installed: installed, err: err}
	})
}
