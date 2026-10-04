package remotehost

import (
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
)

// receive routes checked runner messages without waiting for a consumer or a policy.
func (l *Link) receive(message runnerproto.Outbound) {
	switch message := message.(type) {
	case *runnerproto.ConversationReleased:
		l.answer(message.ID, "Release", message, replyFailure(message.Error))
	case *runnerproto.JobReadReply:
		l.answer(message.ID, "JobRead", message, replyFailure(message.Error))
	case *runnerproto.SyncDone:
		l.answer(message.ID, "Sync", message, replyFailure(message.Error))
	case *runnerproto.Hello:
	case *runnerproto.Installs, *runnerproto.Pong, *runnerproto.VolumeGrow:
		l.receiveStatus(message)
	case *runnerproto.FSOK:
		l.answer(message.ID, fmt.Sprintf("Fs(%q)", message.Result.Op()), message, nil)
	case *runnerproto.FSError:
		l.receiveFileError(message)
	case *runnerproto.GitOK:
		l.answer(message.ID, fmt.Sprintf("Git(%q)", message.Result.Op()), message, nil)
	case *runnerproto.GitError:
		l.answer(message.ID, "", nil, &host.Error{Kind: host.Failed, Code: message.Code, Message: message.Message})
	case *runnerproto.LogLines:
		l.answer(message.ID, "Log", message, nil)
	case *runnerproto.LogError:
		l.answer(message.ID, "", nil, &host.Error{Kind: host.Failed, Message: message.Message})
	case *runnerproto.NetOpened:
		l.answer(message.StreamID, "Net", message, nil)
	case *runnerproto.ServiceOpened:
		l.answer(message.StreamID, "Service", message, nil)
	case *runnerproto.NetError:
		l.answer(
			message.StreamID,
			"",
			nil,
			&host.Error{Kind: host.Failed, Code: string(message.Code), Message: message.Message},
		)
	case *runnerproto.ServiceError:
		l.answer(
			message.StreamID,
			"",
			nil,
			&host.Error{Kind: host.Failed, Code: string(message.Code), Message: message.Message},
		)
	case *runnerproto.ServiceDone, *runnerproto.SpawnOutput, *runnerproto.SpawnExit:
		l.receiveProcess(message)
	case *runnerproto.JobOutput, *runnerproto.JobRunningHint, *runnerproto.JobExit:
		l.receiveJob(message)
	case *runnerproto.RPCCall, *runnerproto.RPCStdin, *runnerproto.RPCStdinEnd, *runnerproto.RPCCancel:
		l.receiveRPC(message)
	case *runnerproto.PipeDone, *runnerproto.ArtifactResolve, *runnerproto.NumbersReserve:
		l.receiveTransfer(message)
	}
}

// replyFailure preserves the runner's optional operation failure.
func replyFailure(message *string) error {
	if message == nil {
		return nil
	}
	return &host.Error{Kind: host.Failed, Message: *message}
}

// endJob cancels calls and artifact requests before publishing the runner's job end.
func (l *Link) endJob(message *runnerproto.JobExit) {
	l.mu.Lock()
	job := l.jobs[message.JobID]
	delete(l.jobs, message.JobID)
	var calls []*relayCall
	for _, call := range l.calls {
		if call.jobID == message.JobID {
			calls = append(calls, call)
		}
	}
	l.mu.Unlock()
	for _, call := range calls {
		call.stop(fmt.Sprintf("calling job %s exited before its RPC completed", message.JobID), false)
	}
	if job != nil {
		job.finish(
			JobEnd{
				Status:         processEnd(message.ExitCode, message.Signal, message.SpawnError),
				CWD:            message.CWD,
				Output:         message.Output,
				Files:          message.Files,
				FilesTruncated: message.FilesTruncated,
			},
		)
	}
}

// teardown atomically closes admission, then completes work outside the state mutex.
func (l *Link) teardown(reason string) {
	l.mu.Lock()
	if l.endReason != "" {
		l.mu.Unlock()
		return
	}
	l.endReason = reason
	waiting := l.waiting
	l.waiting = make(map[string]*replyWait)
	jobs := l.jobs
	l.jobs = make(map[string]*Job)
	spawns := l.spawns
	l.spawns = make(map[string]*runnerProcess)
	services := l.services
	l.services = make(map[string]*ServiceStream)
	calls := l.calls
	l.calls = make(map[string]*relayCall)
	l.mu.Unlock()
	l.cancel()
	for _, request := range waiting {
		request.ready <- reply{err: &host.Error{Kind: host.Offline, Message: reason}}
	}
	for _, job := range jobs {
		job.finish(JobEnd{Status: host.ProcessEnd{Kind: host.ProcessLost, Reason: reason}})
	}
	for _, process := range spawns {
		process.finish(host.ProcessEnd{Kind: host.ProcessLost, Reason: reason})
	}
	for _, service := range services {
		service.finish(ServiceEnd{}, &host.Error{Kind: host.Offline, Message: reason})
	}
	for _, call := range calls {
		call.stop(reason, false)
	}
	l.pipes.DeviceGone(l.device)
}

// receiveProcess routes runner messages for service and process lifetimes.
func (l *Link) receiveProcess(message runnerproto.Outbound) {
	if message, ok := message.(*runnerproto.ServiceDone); ok {
		l.mu.Lock()
		service := l.services[message.StreamID]
		l.mu.Unlock()
		if service != nil {
			service.finish(ServiceEnd{ExitCode: message.ExitCode, Stderr: message.Stderr}, nil)
		}
		return
	}
	if message, ok := message.(*runnerproto.SpawnOutput); ok {
		l.mu.Lock()
		process := l.spawns[message.SpawnID]
		l.mu.Unlock()
		if process != nil {
			process.state.push(host.ProcessOutput{Stream: types.StreamKind(message.Stream), Bytes: message.Bytes})
		}
		return
	}
	if message, ok := message.(*runnerproto.SpawnExit); ok {
		l.mu.Lock()
		process := l.spawns[message.SpawnID]
		delete(l.spawns, message.SpawnID)
		l.mu.Unlock()
		if process != nil {
			process.finish(processEnd(message.ExitCode, message.Signal, message.SpawnError))
		}
		return
	}
}

// receiveJob routes runner messages for shell job output and completion.
func (l *Link) receiveJob(message runnerproto.Outbound) {
	if message, ok := message.(*runnerproto.JobOutput); ok {
		l.mu.Lock()
		job := l.jobs[message.JobID]
		l.mu.Unlock()
		if job != nil {
			job.state.push(
				JobOutput{Stream: types.StreamKind(message.Stream), Offset: message.Offset, Bytes: message.Bytes},
			)
		}
		return
	}
	if message, ok := message.(*runnerproto.JobRunningHint); ok {
		l.mu.Lock()
		job := l.jobs[message.JobID]
		l.mu.Unlock()
		if job != nil {
			job.state.hint(message.InvocationID, message.Hint)
		}
		return
	}
	if message, ok := message.(*runnerproto.JobExit); ok {
		l.endJob(message)
		return
	}
}

// receiveRPC routes runner messages for RPC calls and input.
func (l *Link) receiveRPC(message runnerproto.Outbound) {
	if message, ok := message.(*runnerproto.RPCCall); ok {
		l.startCall(message)
		return
	}
	if message, ok := message.(*runnerproto.RPCStdin); ok {
		l.mu.Lock()
		call := l.calls[message.CallID]
		l.mu.Unlock()
		if call != nil {
			call.liveInput(message.Bytes)
		}
		return
	}
	if message, ok := message.(*runnerproto.RPCStdinEnd); ok {
		l.mu.Lock()
		call := l.calls[message.CallID]
		l.mu.Unlock()
		if call != nil {
			call.endInput()
		}
		return
	}
	if message, ok := message.(*runnerproto.RPCCancel); ok {
		l.mu.Lock()
		call := l.calls[message.CallID]
		l.mu.Unlock()
		if call != nil {
			call.stop("command cancelled", true)
		}
		return
	}
}

// receiveTransfer routes runner messages for pipes, artifacts and sequence reservations.
func (l *Link) receiveTransfer(message runnerproto.Outbound) {
	if message, ok := message.(*runnerproto.PipeDone); ok {
		reason := "device transfer failed"
		if message.Error != nil {
			reason = *message.Error
		}
		if !message.Ok && l.pipes.FailFromDevice(message.PipeID, l.device, reason) {
			slog.Info("pipe failed on the device: "+reason, "device", l.device, "pipe", message.PipeID)
		}
		return
	}
	if message, ok := message.(*runnerproto.ArtifactResolve); ok {
		l.resolveArtifact(message)
		return
	}
	if message, ok := message.(*runnerproto.NumbersReserve); ok {
		l.reserveNumbers(message)
		return
	}
}

// receiveStatus updates installation and liveness state or starts volume growth.
func (l *Link) receiveStatus(message runnerproto.Outbound) {
	if message, ok := message.(*runnerproto.Installs); ok {
		l.mu.Lock()
		previous := l.installsChanged
		l.installs = message.Installs
		l.installsChanged = make(chan struct{})
		l.mu.Unlock()
		close(previous)
		return
	}
	if message, ok := message.(*runnerproto.Pong); ok {
		l.mu.Lock()
		l.pongJobs = message.Jobs
		if l.liveness == pingWaiting {
			l.liveness = pingIdle
		}
		l.mu.Unlock()
		return
	}
	if message, ok := message.(*runnerproto.VolumeGrow); ok {
		l.growVolume(message)
		return
	}
}

// receiveFileError maps a filesystem refusal to the Host error vocabulary.
func (l *Link) receiveFileError(message *runnerproto.FSError) {
	failure := &host.Error{Kind: host.Failed, Message: message.Message}
	if message.Code != nil {
		failure.Code = *message.Code
		if failure.Code == "too_large" {
			failure.Kind = host.TooLarge
			failure.Code = ""
		}
	}
	l.answer(message.ID, "", nil, failure)
}
