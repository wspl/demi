package host

//revive:disable:exported
// Contract doc comments below are product text, which does not start with the declared name.

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// One `rpc` call: the leaf it names, its validated arguments, and the
// invoking process's surroundings (`commands.md` § Handle an rpc call).
// +demi:root
// +demi:check validateRPCInvocation
type RPCInvocation struct {
	// The leaf's path, root first.
	Path []string `json:"path"`
	// The command line after the root's name, as the process wrote it.
	Argv []string `json:"argv"`
	// The arguments, valid against the leaf's input; a `stdinField` body is
	// among them.
	Args json.RawMessage `json:"args"`
	// Whether the caller passed `--json`.
	JSON bool              `json:"json"`
	CWD  string            `json:"cwd"`
	Env  map[string]string `json:"env"`
	// The invoking job's command context, from the backend's record of it.
	Context commandwire.CommandContext `json:"context"`
	// Whose command storage the invoking job reaches, which a job the
	// handler starts elsewhere carries on; none for a job no agent started.
	Caller *JobCaller `json:"caller,omitempty"`
	// Whether the calling process has a pipe on its standard input.
	Stdin bool `json:"stdin"`
	// The pipes relayed for the call's standard input and output, which a
	// handler that starts a job elsewhere can hand to it.
	Pipes *RelayedPipes `json:"pipes,omitempty"`
}

// The ids of a call's relayed pipes.
type RelayedPipes struct {
	Stdin  *string `json:"stdin,omitempty"`
	Stdout string  `json:"stdout"`
}

// RPCHandler handles an rpc leaf. Its output must be consumed before its exit is reported.
type RPCHandler interface {
	Call(context.Context, RPCInvocation, RPCPort) (uint8, error)
}

// RPCHandlerFunc adapts a function into an RPCHandler.
type RPCHandlerFunc func(context.Context, RPCInvocation, RPCPort) (uint8, error)

// Call invokes the function.
func (f RPCHandlerFunc) Call(ctx context.Context, invocation RPCInvocation, port RPCPort) (uint8, error) {
	return f(ctx, invocation, port)
}

// RPCErrorKind distinguishes usage refusals from handler failures.
type RPCErrorKind uint8

const (
	Usage RPCErrorKind = iota
	HandlerFailed
)

// RPCError reports why a handler gave up, retaining any underlying error.
type RPCError struct {
	Kind    RPCErrorKind
	Message string
	Err     error
}

// Error returns the failure message.
func (e *RPCError) Error() string { return e.Message }

// Unwrap returns the underlying failure.
func (e *RPCError) Unwrap() error { return e.Err }

// PortErrorKind classifies a failed port operation.
type PortErrorKind uint8

const (
	PortEnded PortErrorKind = iota
	StorageRefused
	PortFailed
	UnexpectedReply
)

// PortError reports a port failure, or a response of the wrong kind.
type PortError struct {
	Kind            PortErrorKind
	Message         string
	Asked, Answered string
	Err             error
}

// Error returns the failure message.
func (e *PortError) Error() string {
	if e.Kind == UnexpectedReply {
		return fmt.Sprintf("the rpc port answered %s to a %s request", e.Answered, e.Asked)
	}
	return e.Message
}

// Unwrap returns the underlying failure.
func (e *PortError) Unwrap() error { return e.Err }

// One request of a handler through its port.
// +demi:union tag=type
// +demi:root
//
//sumtype:decl
type PortRequest interface{ portRequest() }

// Standard output, which reaches the caller through the call's relayed
// pipe: the reply waits until the caller has read enough.
// +demi:variant PortRequest stdout
type PortStdout struct {
	Bytes core.B64Bytes `json:"bytes"`
}

// PortStderr writes standard error.
// +demi:variant PortRequest stderr
type PortStderr struct {
	Bytes core.B64Bytes `json:"bytes"`
}

// The next chunk of a finite standard input.
// +demi:variant PortRequest read_stdin
type PortReadStdin struct{}

// The next interactive write to the calling job, until it ends.
// +demi:variant PortRequest read_live_stdin
type PortReadLiveStdin struct{}

// PortStorage requests a storage operation.
// +demi:variant PortRequest storage
type PortStorage struct {
	Op StorageOp `json:"op"`
}

func (*PortStdout) portRequest()        {}
func (*PortStderr) portRequest()        {}
func (*PortReadStdin) portRequest()     {}
func (*PortReadLiveStdin) portRequest() {}
func (*PortStorage) portRequest()       {}

// The reply to a [PortRequest].
// +demi:union tag=type
// +demi:root
//
//sumtype:decl
type PortResponse interface{ portResponse() }

// The output was written.
// +demi:variant PortResponse written
type PortWritten struct{}

// A chunk of input; none once the input has ended.
// +demi:variant PortResponse input
type PortInput struct {
	// +demi:nullable
	Bytes *core.B64Bytes `json:"bytes"`
}

// PortStored carries a storage reply.
// +demi:variant PortResponse storage
type PortStored struct {
	Reply StorageReply `json:"reply"`
}

func (*PortWritten) portResponse() {}
func (*PortInput) portResponse()   {}
func (*PortStored) portResponse()  {}

// An operation on the invoking agent node's command storage
// (`command-state-history.md` § Mutation API and concurrency).
// +demi:union tag=op
// +demi:root
//
//sumtype:decl
type StorageOp interface{ storageOp() }

// The key's value in the current version, and the node's revision.
// +demi:variant StorageOp read
type StorageRead struct {
	Key string `json:"key"`
}

// The keys that start with `prefix`, sorted.
// +demi:variant StorageOp list
type StorageList struct {
	Prefix string `json:"prefix"`
}

// Sets the key, or removes it when `value` is none, if the node's
// revision is still `expected`; without `expected`, whatever it is.
// +demi:variant StorageOp write_if
type StorageWriteIf struct {
	Key      string          `json:"key"`
	Value    json.RawMessage `json:"value"`
	Expected *Revision       `json:"expected,omitempty"`
}

func (*StorageRead) storageOp()    {}
func (*StorageList) storageOp()    {}
func (*StorageWriteIf) storageOp() {}

// The answer to a [StorageOp].
// +demi:union tag=outcome
// +demi:root
//
//sumtype:decl
type StorageReply interface{ storageReply() }

// StorageValue reports the current value and node revision.
// +demi:variant StorageReply value
type StorageValue struct {
	Value    json.RawMessage `json:"value"`
	Revision Revision        `json:"revision"`
}

// StorageKeys lists keys in sorted order.
// +demi:variant StorageReply keys
type StorageKeys struct {
	Keys []string `json:"keys"`
}

// The write is the node's current version, `revision`.
// +demi:variant StorageReply committed
type StorageCommitted struct {
	Revision Revision `json:"revision"`
}

// Another write came first; the node is at `revision`.
// +demi:variant StorageReply conflict
type StorageConflict struct {
	Revision Revision `json:"revision"`
}

func (*StorageValue) storageReply()     {}
func (*StorageKeys) storageReply()      {}
func (*StorageCommitted) storageReply() {}
func (*StorageConflict) storageReply()  {}

// The version a node's command storage is at. Every committed write of any
// of its keys advances it.
type Revision uint64

// PortTransport carries each request to a single response.
type PortTransport interface {
	Request(context.Context, PortRequest) (PortResponse, error)
}

// RPCPort is a handler's request/reply port. Cancellation is carried by the call's context.
type RPCPort struct{ transport PortTransport }

// NewRPCPort constructs a port over a transport.
func NewRPCPort(transport PortTransport) RPCPort { return RPCPort{transport: transport} }

// Forward sends a request as supplied by a remote handler.
func (p RPCPort) Forward(ctx context.Context, request PortRequest) (PortResponse, error) {
	return p.transport.Request(ctx, request)
}

// Stdout writes once the caller has room.
func (p RPCPort) Stdout(ctx context.Context, data []byte) error {
	return p.write(ctx, &PortStdout{Bytes: data}, "stdout")
}

// Stderr writes standard error.
func (p RPCPort) Stderr(ctx context.Context, data []byte) error {
	return p.write(ctx, &PortStderr{Bytes: data}, "stderr")
}

// write checks the response to a port output request.
func (p RPCPort) write(ctx context.Context, request PortRequest, asked string) error {
	response, err := p.Forward(ctx, request)
	if err != nil {
		return err
	}
	if _, ok := response.(*PortWritten); !ok {
		return unexpectedPort(asked, response)
	}
	return nil
}

// ReadStdin returns the next finite input chunk, or nil at its end.
func (p RPCPort) ReadStdin(ctx context.Context) ([]byte, error) {
	return p.input(ctx, &PortReadStdin{}, "read_stdin")
}

// ReadLiveStdin returns the next interactive input chunk, or nil at the job's end.
func (p RPCPort) ReadLiveStdin(ctx context.Context) ([]byte, error) {
	return p.input(ctx, &PortReadLiveStdin{}, "read_live_stdin")
}

// input checks the response to a port input request.
func (p RPCPort) input(ctx context.Context, request PortRequest, asked string) ([]byte, error) {
	response, err := p.Forward(ctx, request)
	if err != nil {
		return nil, err
	}
	input, ok := response.(*PortInput)
	if !ok {
		return nil, unexpectedPort(asked, response)
	}
	if input.Bytes == nil {
		return nil, nil
	}
	return *input.Bytes, nil
}

// Storage performs one command storage operation.
func (p RPCPort) Storage(ctx context.Context, op StorageOp) (StorageReply, error) {
	response, err := p.Forward(ctx, &PortStorage{Op: op})
	if err != nil {
		return nil, err
	}
	stored, ok := response.(*PortStored)
	if !ok {
		return nil, unexpectedPort("storage", response)
	}
	return stored.Reply, nil
}

// Update reads, computes and conditionally writes, retrying on revision conflicts.
// Decode and encode are generated contract codecs. Change performs no other IO.
func Update[T any](
	ctx context.Context,
	p RPCPort,
	key string,
	decode func([]byte) (T, error),
	encode func(T) ([]byte, error),
	change func(current T, found bool) (T, error),
) (T, error) {
	var zero T
	for {
		stored, err := p.Storage(ctx, &StorageRead{Key: key})
		if err != nil {
			return zero, err
		}
		value, ok := stored.(*StorageValue)
		if !ok {
			return zero, unexpectedStorage("read", stored)
		}
		var current T
		found := false
		if len(value.Value) != 0 && !contract.IsNull(value.Value) {
			decoded, err := decode(value.Value)
			if err != nil {
				return zero, &RPCError{
					Kind:    HandlerFailed,
					Message: fmt.Sprintf("stored %s is unreadable: %v", key, err),
					Err:     err,
				}
			}
			current = decoded
			found = true
		}
		next, err := change(current, found)
		if err != nil {
			return zero, err
		}
		data, err := encode(next)
		if err != nil {
			return zero, &RPCError{
				Kind:    HandlerFailed,
				Message: fmt.Sprintf("%s cannot be stored: %v", key, err),
				Err:     err,
			}
		}
		raw := json.RawMessage(data)
		reply, err := p.Storage(ctx, &StorageWriteIf{Key: key, Value: raw, Expected: &value.Revision})
		if err != nil {
			return zero, err
		}
		switch reply.(type) {
		case *StorageCommitted:
			return next, nil
		case *StorageConflict:
			continue
		case *StorageValue, *StorageKeys:
			return zero, unexpectedStorage("write_if", reply)
		}
	}
}

// unexpectedPort describes a mismatched port response.
func unexpectedPort(asked string, response PortResponse) error {
	answered := ""
	switch response.(type) {
	case *PortWritten:
		answered = "written"
	case *PortInput:
		answered = "input"
	case *PortStored:
		answered = "storage"
	}
	return &PortError{Kind: UnexpectedReply, Asked: asked, Answered: answered}
}

// unexpectedStorage describes a mismatched command storage reply.
func unexpectedStorage(asked string, reply StorageReply) error {
	answered := ""
	switch reply.(type) {
	case *StorageValue:
		answered = "value"
	case *StorageKeys:
		answered = "keys"
	case *StorageCommitted:
		answered = "committed"
	case *StorageConflict:
		answered = "conflict"
	}
	return &PortError{Kind: UnexpectedReply, Asked: asked, Answered: answered}
}

// Call carries decoded arguments and the rest of their invocation.
type Call[A any] struct {
	Args       A
	Invocation RPCInvocation
}

// TypedRPC adapts a handler using its generated argument decoder.
func TypedRPC[A any](
	decode func([]byte) (A, error),
	run func(context.Context, Call[A], RPCPort) (uint8, error),
) RPCHandlerFunc {
	return RPCHandlerFunc(func(ctx context.Context, invocation RPCInvocation, port RPCPort) (uint8, error) {
		args, err := decode(invocation.Args)
		if err != nil {
			return 0, &RPCError{
				Kind: HandlerFailed,
				Message: fmt.Sprintf(
					"the arguments of %q do not decode as declared: %v",
					strings.Join(invocation.Path, " "),
					err,
				),
				Err: err,
			}
		}
		return run(ctx, Call[A]{Args: args, Invocation: invocation}, port)
	})
}

// validateRPCInvocation preserves the object requirement of the command's opaque arguments.
func validateRPCInvocation(invocation RPCInvocation) error {
	if _, err := contract.Object(invocation.Args); err != nil {
		return fmt.Errorf("args: %w", err)
	}
	return nil
}
