package shell

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"context"
	"encoding/json/jsontext"
	"fmt"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/core"
)

// RPCInvocation and the port messages are the entire handler boundary. They
// contain data, never a Host or backend callback.
//
//demi:wire
type RPCInvocation struct {
	Path    []string                      `json:"path"`
	Argv    []string                      `json:"argv"`
	Args    jsontext.Value                `json:"args" check:"func=objectArguments"`
	JSON    bool                          `json:"json"`
	Cwd     string                        `json:"cwd"`
	Env     map[string]string             `json:"env"`
	Context commandservice.CommandContext `json:"context" check:"func=commandservice.Validate"`
	Caller  *JobCaller                    `json:"caller,omitzero"`
	Stdin   bool                          `json:"stdin"`
	Pipes   *RelayedPipes                 `json:"pipes,omitzero"`
}

func objectArguments(value jsontext.Value) error {
	if value.Kind() != '{' {
		return fmt.Errorf("expected object")
	}
	return nil
}

//demi:wire
type RelayedPipes struct {
	Stdin  *string `json:"stdin,omitzero"`
	Stdout string  `json:"stdout"`
}
type RPCHandler interface {
	Call(context.Context, RPCInvocation, RPCPort) (uint8, error)
}
type RPCHandlerFunc func(context.Context, RPCInvocation, RPCPort) (uint8, error)

func (f RPCHandlerFunc) Call(ctx context.Context, call RPCInvocation, port RPCPort) (uint8, error) {
	return f(ctx, call, port)
}

type RPCErrorKind string

const (
	RPCUsage  RPCErrorKind = "usage"
	RPCFailed RPCErrorKind = "failed"
)

type RPCError struct {
	Kind    RPCErrorKind
	Message string
}

func (e *RPCError) Error() string { return e.Message }

type PortErrorKind string

const (
	PortEnded      PortErrorKind = "ended"
	PortStorage    PortErrorKind = "storage"
	PortUnexpected PortErrorKind = "unexpected"
)

type PortError struct {
	Kind                     PortErrorKind
	Message, Asked, Answered string
}

func (e *PortError) Error() string {
	if e.Kind == PortUnexpected {
		return fmt.Sprintf("the rpc port answered %s to a %s request", e.Answered, e.Asked)
	}
	return e.Message
}

//demi:union tag=type
type PortRequest interface{ portRequest() }

//demi:variant stdout
type PortStdout struct {
	Bytes core.B64Bytes `json:"bytes" check:"func=core.Validate"`
}

//demi:variant stderr
type PortStderr struct {
	Bytes core.B64Bytes `json:"bytes" check:"func=core.Validate"`
}

//demi:variant read_stdin
type PortReadStdin struct{}

//demi:variant read_live_stdin
type PortReadLiveStdin struct{}

//demi:variant storage
type PortStorageRequest struct {
	Op StorageOp `json:"op"`
}

func (PortStdout) portRequest()         {}
func (PortStderr) portRequest()         {}
func (PortReadStdin) portRequest()      {}
func (PortReadLiveStdin) portRequest()  {}
func (PortStorageRequest) portRequest() {}

//demi:union tag=type
type PortResponse interface{ portResponse() }

//demi:variant written
type PortWritten struct{}

//demi:variant input
type PortInput struct {
	Bytes *core.B64Bytes `json:"bytes" check:"nullable,func=core.Validate"`
}

//demi:variant storage
type PortStorageResponse struct {
	Reply StorageReply `json:"reply"`
}

func (PortWritten) portResponse()         {}
func (PortInput) portResponse()           {}
func (PortStorageResponse) portResponse() {}

//demi:union tag=op
type StorageOp interface{ storageOp() }

//demi:variant read
type StorageRead struct {
	Key string `json:"key"`
}

//demi:variant list
type StorageList struct {
	Prefix string `json:"prefix"`
}

//demi:variant write_if
type StorageWriteIf struct {
	Key      string         `json:"key"`
	Value    jsontext.Value `json:"value"`
	Expected *Revision      `json:"expected,omitzero"`
}

func (StorageRead) storageOp()    {}
func (StorageList) storageOp()    {}
func (StorageWriteIf) storageOp() {}

//demi:union tag=outcome
type StorageReply interface{ storageReply() }

//demi:variant value
type StorageValue struct {
	Value    jsontext.Value `json:"value"`
	Revision Revision       `json:"revision"`
}

//demi:variant keys
type StorageKeys struct {
	Keys []string `json:"keys"`
}

//demi:variant committed
type StorageCommitted struct {
	Revision Revision `json:"revision"`
}

//demi:variant conflict
type StorageConflict struct {
	Revision Revision `json:"revision"`
}

func (StorageValue) storageReply()     {}
func (StorageKeys) storageReply()      {}
func (StorageCommitted) storageReply() {}
func (StorageConflict) storageReply()  {}

type Revision uint64

type PortTransport interface {
	Request(context.Context, PortRequest) (PortResponse, error)
}
type RPCPort struct {
	Transport PortTransport
	Context   context.Context
}

func (p RPCPort) Cancelled() <-chan struct{} { return p.Context.Done() }
func (p RPCPort) IsCancelled() bool          { return p.Context.Err() != nil }
func (p RPCPort) Stdout(ctx context.Context, data []byte) error {
	return p.write(ctx, PortStdout{core.NewB64Bytes(data)}, "stdout")
}
func (p RPCPort) Stderr(ctx context.Context, data []byte) error {
	return p.write(ctx, PortStderr{core.NewB64Bytes(data)}, "stderr")
}
func (p RPCPort) write(ctx context.Context, request PortRequest, asked string) error {
	response, err := p.Transport.Request(ctx, request)
	if err != nil {
		return err
	}
	if _, ok := response.(PortWritten); ok {
		return nil
	}
	return unexpectedPort(asked, response)
}
func (p RPCPort) ReadStdin(ctx context.Context) ([]byte, error) {
	return p.input(ctx, PortReadStdin{}, "read_stdin")
}
func (p RPCPort) ReadLiveStdin(ctx context.Context) ([]byte, error) {
	return p.input(ctx, PortReadLiveStdin{}, "read_live_stdin")
}
func (p RPCPort) input(ctx context.Context, request PortRequest, asked string) ([]byte, error) {
	response, err := p.Transport.Request(ctx, request)
	if err != nil {
		return nil, err
	}
	if input, ok := response.(PortInput); ok {
		if input.Bytes == nil {
			return nil, nil
		}
		return input.Bytes.Bytes(), nil
	}
	return nil, unexpectedPort(asked, response)
}
func (p RPCPort) Storage(ctx context.Context, op StorageOp) (StorageReply, error) {
	response, err := p.Transport.Request(ctx, PortStorageRequest{op})
	if err != nil {
		return nil, err
	}
	if storage, ok := response.(PortStorageResponse); ok {
		return storage.Reply, nil
	}
	return nil, unexpectedPort("storage", response)
}

// unexpectedPort reports a reply of the wrong port operation, without inspecting
// serialized text or exposing request values.
func unexpectedPort(asked string, response any) error {
	var answered string
	switch response.(type) {
	case PortWritten:
		answered = "written"
	case PortInput:
		answered = "input"
	case PortStorageResponse:
		answered = "storage"
	case StorageValue:
		answered = "value"
	case StorageKeys:
		answered = "keys"
	case StorageCommitted:
		answered = "committed"
	case StorageConflict:
		answered = "conflict"
	}
	return &PortError{Kind: PortUnexpected, Asked: asked, Answered: answered}
}

// Update reads and retries a pure storage change on a competing write. decode
// must be the type's generated decoder: corrupt stored values are never replaced.
func Update[T any](ctx context.Context, p RPCPort, key string, decode func([]byte) (T, error), encode func(T) ([]byte, error), change func(*T) (T, error)) (T, error) {
	var zero T
	for {
		reply, err := p.Storage(ctx, StorageRead{key})
		if err != nil {
			return zero, err
		}
		stored, ok := reply.(StorageValue)
		if !ok {
			return zero, unexpectedPort("read", reply)
		}
		var current *T
		if string(stored.Value) != "null" && len(stored.Value) > 0 {
			value, err := decode(stored.Value)
			if err != nil {
				return zero, &RPCError{RPCFailed, fmt.Sprintf("stored %s is unreadable: %s", key, err)}
			}
			current = &value
		}
		next, err := change(current)
		if err != nil {
			return zero, err
		}
		data, err := encode(next)
		if err != nil {
			return zero, &RPCError{RPCFailed, fmt.Sprintf("%s cannot be stored: %s", key, err)}
		}
		reply, err = p.Storage(ctx, StorageWriteIf{key, jsontext.Value(data), &stored.Revision})
		if err != nil {
			return zero, err
		}
		switch reply.(type) {
		case StorageCommitted:
			return next, nil
		case StorageConflict:
			continue
		default:
			return zero, unexpectedPort("write_if", reply)
		}
	}
}
