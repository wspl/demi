package hostremote

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

const (
	PingInterval     = 30 * time.Second
	OutboundFrames   = 64
	ArtifactRequests = 32
	NumbersRequests  = 32
)

// LinkPolicy is the product's decision boundary. The connection routes bytes
// and messages; it neither admits an agent nor owns its persistent storage.
// Callbacks run in concurrent connection workers. A policy uses its shard's
// executor for state changes and honors cancellation before returning.
type LinkPolicy interface {
	AdmitCall(JobOrigin) error
	Dispatch(context.Context, JobOrigin, shell.RPCInvocation, shell.RPCPort) (uint8, error)
	Storage(context.Context, JobOrigin, shell.StorageOp) (shell.StorageReply, error)
	GrowVolume(context.Context, runnerproto.VolumeName, uint64) error
	ReserveNumbers(context.Context, string, commandservice.Sequence, uint32) (uint64, error)
}
type JobOrigin struct {
	Host    shell.HostKey
	Context commandservice.CommandContext
	Caller  *shell.JobCaller
}
type JobOutput struct {
	Stream core.StreamKind
	Offset uint64
	Bytes  []byte
}
type JobEnd struct {
	Status         shell.ProcessEnd
	Cwd            *string
	Output         *runnerproto.OutputLengths
	Files          []runnerproto.JobFileChange
	FilesTruncated bool
}

func lostJob(reason string) JobEnd {
	return JobEnd{Status: shell.ProcessEnd{Kind: shell.ProcessLost, Reason: reason}}
}

type ServiceEnd struct {
	ExitCode uint8
	Stderr   string
}
type ServiceCallError struct {
	ExitCode uint8
	Stderr   string
	Stdout   []byte
	// Limit is nonnil only when a reply exceeds the allowed byte count.
	Limit *int
}

func (e *ServiceCallError) Error() string {
	if e.Limit != nil {
		return fmt.Sprintf("Service answer exceeds %d bytes", *e.Limit)
	}
	return fmt.Sprintf("Service call exited with %d: %s", e.ExitCode, strings.TrimSpace(e.Stderr))
}

// processEnd preserves the runner's priority among its optional ending fields.
func processEnd(exitCode *int32, signal *string, spawnError *runnerproto.SpawnError) shell.ProcessEnd {
	if exitCode != nil {
		return shell.ProcessEnd{Kind: shell.ProcessExited, ExitCode: *exitCode}
	}
	if spawnError != nil {
		return shell.ProcessEnd{Kind: shell.ProcessNotStarted, Error: &shell.SpawnError{Kind: shell.SpawnErrorKind(spawnError.Kind), Detail: spawnError.Detail}}
	}
	if signal != nil {
		return shell.ProcessEnd{Kind: shell.ProcessSignalled, Signal: *signal}
	}
	return shell.ProcessEnd{Kind: shell.ProcessLost, Reason: "the process ended without a status"}
}
func fsError(code *string, message string) error {
	if code != nil && *code == "too_large" {
		return &shell.HostError{Kind: shell.HostTooLarge, Message: message}
	}
	failure := &shell.HostError{Kind: shell.HostFailed, Message: message}
	if code != nil {
		failure.Code = *code
	}
	return failure
}
