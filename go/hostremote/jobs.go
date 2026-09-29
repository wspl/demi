package hostremote

import (
	"context"
	"encoding/json/jsontext"

	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

func (h *RemoteHost) StartJob(ctx context.Context, job JobStart) (*RemoteJob, error) {
	release, err := h.admit(ctx)
	if err != nil {
		return nil, err
	}
	shared := newShared[JobEnd, JobOutput]()
	id := requestID()
	link, err := h.link(ctx)
	if err != nil {
		release()
		shared.finish(lostJob("runner disconnected"))
		return &RemoteJob{id: id, shared: shared}, nil
	}
	lifetime, cancel := context.WithCancel(link.ctx)
	entry := &jobEntry{shared: shared, origin: JobOrigin{h.key, job.Context, job.Caller}, commands: job.Commands, ctx: lifetime, cancel: cancel, release: release}
	link.mu.Lock()
	if link.IsClosed() {
		link.mu.Unlock()
		cancel()
		release()
		return nil, link.offline()
	}
	link.jobs[id] = entry
	link.mu.Unlock()
	// A manifest and its start are adjacent even when different Hosts submit jobs.
	turn, err := link.jobTurn.Acquire(ctx)
	if err == nil {
		defer turn.Release()
		if job.Commands != nil {
			link.mu.Lock()
			sent := link.manifest
			link.mu.Unlock()
			if sent != job.Commands.Hash() {
				err = link.send(ctx, runnerproto.InboundManifest{Manifest: jsontext.Value(job.Commands.wire)})
				if err == nil {
					link.mu.Lock()
					link.manifest = job.Commands.Hash()
					link.mu.Unlock()
				}
			}
		}
		if err == nil {
			var hash *string
			if job.Commands != nil {
				hash = new(job.Commands.Hash())
			}
			err = link.send(ctx, runnerproto.InboundJobStart{JobID: id, ManifestHash: hash, Context: job.Context, Script: job.Script, Cwd: job.Cwd, Env: job.Env, Stdin: job.Stdin, Stdout: job.Stdout})
		}
	}
	if err != nil {
		link.mu.Lock()
		delete(link.jobs, id)
		link.manifest = ""
		link.mu.Unlock()
		cancel()
		release()
		shared.finish(lostJob(err.Error()))
		return nil, err
	}
	return &RemoteJob{id: id, shared: shared, link: link}, nil
}

type RemoteJob struct {
	id     string
	shared *shared[JobEnd, JobOutput]
	link   *Link
}

func (j *RemoteJob) ID() string                                        { return j.id }
func (j *RemoteJob) NextOutput(ctx context.Context) (JobOutput, error) { return j.shared.next(ctx) }
func (j *RemoteJob) End(ctx context.Context) (JobEnd, error)           { return j.shared.wait(ctx) }
func (j *RemoteJob) Ended() *JobEnd                                    { return j.shared.ended() }
func (j *RemoteJob) RunningHint() *string                              { return j.shared.runningHint() }
func (j *RemoteJob) Follow(ctx context.Context, follow bool) error {
	if j.link == nil || j.Ended() != nil {
		return nil
	}
	return j.link.send(ctx, runnerproto.InboundJobFollow{JobID: j.id, Follow: follow})
}
func (j *RemoteJob) WriteStdin(ctx context.Context, bytes []byte) error {
	if j.link == nil || j.Ended() != nil {
		return nil
	}
	return sendStdin(ctx, j.link, bytes, func(bytes []byte) runnerproto.Inbound { return runnerproto.InboundJobStdin{JobID: j.id, Bytes: bytes} })
}
func (j *RemoteJob) CloseStdin(ctx context.Context) error {
	if j.link == nil || j.Ended() != nil {
		return nil
	}
	return j.link.send(ctx, runnerproto.InboundJobStdinEnd{JobID: j.id})
}
func (j *RemoteJob) Kill(ctx context.Context, signal runnerproto.Signal) error {
	if j.link == nil || j.Ended() != nil {
		return nil
	}
	return j.link.send(ctx, runnerproto.InboundJobKill{JobID: j.id, Signal: &signal})
}
func (j *RemoteJob) Release(ctx context.Context) error {
	if j.link == nil || j.link.IsClosed() {
		return nil
	}
	return j.link.send(ctx, runnerproto.InboundJobRelease{JobID: j.id})
}
func (j *RemoteJob) ReadOutput(ctx context.Context) (*shell.WholeOutput, error) {
	if j.link == nil {
		return nil, &shell.HostError{Kind: shell.HostOffline, Message: "the job's runner is not connected"}
	}
	reader, err := filled(ctx, j.link, "JobRead", func(id string, output runnerproto.PipeRef) runnerproto.Inbound {
		return runnerproto.InboundJobRead{ID: id, JobID: j.id, Output: output}
	})
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	bytes, err := collectPipe(ctx, reader, runnerproto.JobKeptReadBytes)
	if err != nil {
		return nil, err
	}
	output, err := DecodeOutput(bytes, nil)
	if err != nil {
		return nil, protocolError("the job's kept output does not decode: " + err.Error())
	}
	return output, nil
}
func sendStdin(ctx context.Context, link *Link, bytes []byte, message func([]byte) runnerproto.Inbound) error {
	for len(bytes) > 0 {
		n := min(len(bytes), runnerproto.StdinChunkBytes)
		if err := link.send(ctx, message(bytes[:n])); err != nil {
			return err
		}
		bytes = bytes[n:]
	}
	return nil
}
func (h *RemoteHost) Spawn(ctx context.Context, request shell.SpawnRequest) (*shell.Process, error) {
	release := func() {}
	var err error
	if !request.Retained {
		release, err = h.admit(ctx)
		if err != nil {
			return nil, err
		}
	}
	shared := newShared[shell.ProcessEnd, shell.ProcessOutput]()
	link, err := h.link(ctx)
	control := &spawnControl{shared: shared}
	process := &shell.Process{Output: control, Control: control, Wait: shared.wait}
	if err != nil {
		release()
		shared.finish(shell.ProcessEnd{Kind: shell.ProcessLost, Reason: "runner disconnected"})
		return process, nil
	}
	id := requestID()
	control.id, control.link = id, link
	link.mu.Lock()
	if link.IsClosed() {
		link.mu.Unlock()
		release()
		return nil, link.offline()
	}
	link.spawns[id] = &spawnEntry{shared: shared, retained: request.Retained, release: release}
	link.mu.Unlock()
	var env *map[string]*string
	if request.Env.Values != nil {
		env = &request.Env.Values
	}
	cwd := request.Cwd
	if cwd == nil {
		cwd = &h.cwd
	}
	var args *[]string
	if len(request.Args) > 0 {
		args = &request.Args
	}
	err = link.send(ctx, runnerproto.InboundSpawn{SpawnID: id, Command: request.Command, Args: args, Cwd: cwd, Env: env, InheritEnv: truePointer(request.Env.Inherit)})
	if err != nil {
		link.mu.Lock()
		delete(link.spawns, id)
		link.mu.Unlock()
		release()
		return nil, err
	}
	return process, nil
}

type spawnControl struct {
	link   *Link
	id     string
	shared *shared[shell.ProcessEnd, shell.ProcessOutput]
}

func (p *spawnControl) Next(ctx context.Context) (shell.ProcessOutput, error) {
	return p.shared.next(ctx)
}
func (p *spawnControl) WriteStdin(ctx context.Context, bytes []byte) error {
	if p.link == nil || p.shared.ended() != nil {
		return nil
	}
	return sendStdin(ctx, p.link, bytes, func(bytes []byte) runnerproto.Inbound {
		return runnerproto.InboundSpawnStdin{SpawnID: p.id, Bytes: bytes}
	})
}
func (p *spawnControl) CloseStdin(ctx context.Context) error {
	if p.link == nil || p.shared.ended() != nil {
		return nil
	}
	return p.link.send(ctx, runnerproto.InboundSpawnStdinEnd{SpawnID: p.id})
}
func (p *spawnControl) Kill(ctx context.Context, signal shell.Signal) error {
	if p.link == nil || p.shared.ended() != nil {
		return nil
	}
	value := runnerproto.Signal(signal)
	return p.link.send(ctx, runnerproto.InboundSpawnKill{SpawnID: p.id, Signal: &value})
}
func (p *spawnControl) Close() error {
	if p.link != nil && p.shared.ended() == nil {
		signal := runnerproto.SignalKill
		p.link.post(runnerproto.InboundSpawnKill{SpawnID: p.id, Signal: &signal})
	}
	return nil
}
