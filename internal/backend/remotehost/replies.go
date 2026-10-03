package remotehost

import (
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// receive routes checked runner messages without waiting for a consumer or a policy.
func (l *Link) receive(message runnerwire.Outbound) {
	switch message := message.(type) {
	case *runnerwire.ConversationReleased:
		l.answer(message.ID, "Release", message, replyFailure(message.Error))
	case *runnerwire.JobReadReply:
		l.answer(message.ID, "JobRead", message, replyFailure(message.Error))
	case *runnerwire.SyncDone:
		l.answer(message.ID, "Sync", message, replyFailure(message.Error))
	case *runnerwire.Hello:
	case *runnerwire.Installs, *runnerwire.Pong, *runnerwire.VolumeGrow:
		l.receiveStatus(message)
	case *runnerwire.FSOK:
		l.answer(message.ID, fmt.Sprintf("Fs(%q)", message.Result.Op()), message, nil)
	case *runnerwire.FSError:
		l.receiveFileError(message)
	case *runnerwire.GitOK:
		l.answer(message.ID, fmt.Sprintf("Git(%q)", message.Result.Op()), message, nil)
	case *runnerwire.GitError:
		l.answer(message.ID, "", nil, &host.Error{Kind: host.Failed, Code: message.Code, Message: message.Message})
	case *runnerwire.LogLines:
		l.answer(message.ID, "Log", message, nil)
	case *runnerwire.LogError:
		l.answer(message.ID, "", nil, &host.Error{Kind: host.Failed, Message: message.Message})
	case *runnerwire.NetOpened:
		l.answer(message.StreamID, "Net", message, nil)
	case *runnerwire.ServiceOpened:
		l.answer(message.StreamID, "Service", message, nil)
	case *runnerwire.NetError:
		l.answer(
			message.StreamID,
			"",
			nil,
			&host.Error{Kind: host.Failed, Code: string(message.Code), Message: message.Message},
		)
	case *runnerwire.ServiceError:
		l.answer(
			message.StreamID,
			"",
			nil,
			&host.Error{Kind: host.Failed, Code: string(message.Code), Message: message.Message},
		)
	case *runnerwire.ServiceDone, *runnerwire.SpawnOutput, *runnerwire.SpawnExit:
		l.receiveProcess(message)
	case *runnerwire.JobOutput, *runnerwire.JobRunningHint, *runnerwire.JobExit:
		l.receiveJob(message)
	case *runnerwire.RPCCall, *runnerwire.RPCStdin, *runnerwire.RPCStdinEnd, *runnerwire.RPCCancel:
		l.receiveRPC(message)
	case *runnerwire.PipeDone, *runnerwire.ArtifactResolve, *runnerwire.NumbersReserve:
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
func (l *Link) endJob(message *runnerwire.JobExit) {
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
func (l *Link) receiveProcess(message runnerwire.Outbound) {
	if message, ok := message.(*runnerwire.ServiceDone); ok {
		l.mu.Lock()
		service := l.services[message.StreamID]
		l.mu.Unlock()
		if service != nil {
			service.finish(ServiceEnd{ExitCode: message.ExitCode, Stderr: message.Stderr}, nil)
		}
		return
	}
	if message, ok := message.(*runnerwire.SpawnOutput); ok {
		l.mu.Lock()
		process := l.spawns[message.SpawnID]
		l.mu.Unlock()
		if process != nil {
			process.state.push(host.ProcessOutput{Stream: core.StreamKind(message.Stream), Bytes: message.Bytes})
		}
		return
	}
	if message, ok := message.(*runnerwire.SpawnExit); ok {
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
func (l *Link) receiveJob(message runnerwire.Outbound) {
	if message, ok := message.(*runnerwire.JobOutput); ok {
		l.mu.Lock()
		job := l.jobs[message.JobID]
		l.mu.Unlock()
		if job != nil {
			job.state.push(
				JobOutput{Stream: core.StreamKind(message.Stream), Offset: message.Offset, Bytes: message.Bytes},
			)
		}
		return
	}
	if message, ok := message.(*runnerwire.JobRunningHint); ok {
		l.mu.Lock()
		job := l.jobs[message.JobID]
		l.mu.Unlock()
		if job != nil {
			job.state.hint(message.InvocationID, message.Hint)
		}
		return
	}
	if message, ok := message.(*runnerwire.JobExit); ok {
		l.endJob(message)
		return
	}
}

// receiveRPC routes runner messages for RPC calls and input.
func (l *Link) receiveRPC(message runnerwire.Outbound) {
	if message, ok := message.(*runnerwire.RPCCall); ok {
		l.startCall(message)
		return
	}
	if message, ok := message.(*runnerwire.RPCStdin); ok {
		l.mu.Lock()
		call := l.calls[message.CallID]
		l.mu.Unlock()
		if call != nil {
			call.liveInput(message.Bytes)
		}
		return
	}
	if message, ok := message.(*runnerwire.RPCStdinEnd); ok {
		l.mu.Lock()
		call := l.calls[message.CallID]
		l.mu.Unlock()
		if call != nil {
			call.endInput()
		}
		return
	}
	if message, ok := message.(*runnerwire.RPCCancel); ok {
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
func (l *Link) receiveTransfer(message runnerwire.Outbound) {
	if message, ok := message.(*runnerwire.PipeDone); ok {
		reason := "device transfer failed"
		if message.Error != nil {
			reason = *message.Error
		}
		if !message.Ok && l.pipes.FailFromDevice(message.PipeID, l.device, reason) {
			slog.Info("pipe failed on the device: "+reason, "device", l.device, "pipe", message.PipeID)
		}
		return
	}
	if message, ok := message.(*runnerwire.ArtifactResolve); ok {
		l.resolveArtifact(message)
		return
	}
	if message, ok := message.(*runnerwire.NumbersReserve); ok {
		l.reserveNumbers(message)
		return
	}
}

// receiveStatus updates installation and liveness state or starts volume growth.
func (l *Link) receiveStatus(message runnerwire.Outbound) {
	if message, ok := message.(*runnerwire.Installs); ok {
		l.mu.Lock()
		previous := l.installsChanged
		l.installs = message.Installs
		l.installsChanged = make(chan struct{})
		l.mu.Unlock()
		close(previous)
		return
	}
	if message, ok := message.(*runnerwire.Pong); ok {
		l.mu.Lock()
		l.pongJobs = message.Jobs
		if l.liveness == pingWaiting {
			l.liveness = pingIdle
		}
		l.mu.Unlock()
		return
	}
	if message, ok := message.(*runnerwire.VolumeGrow); ok {
		l.growVolume(message)
		return
	}
}

// receiveFileError maps a filesystem refusal to the Host error vocabulary.
func (l *Link) receiveFileError(message *runnerwire.FSError) {
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
