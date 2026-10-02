package hostaccess

//revive:disable:unused-parameter

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/webapi"
)

// ServiceBinding is the native operation a user stream or one-shot call runs.
type ServiceBinding struct {
	Package   commandwire.PackageDescriptor
	Operation string
}

// UserStreams is the immutable set of published streams pages may open by name.
type UserStreams struct{}

// StreamDeclaration binds one page stream name to a declared native operation.
type StreamDeclaration struct {
	Name      string
	Operation declare.NativeOperation
}

// NewUserStreams binds declarations served by native; unsupported bindings
// declare nothing. Declaration order determines duplicate-name replacement.
func NewUserStreams(declared []StreamDeclaration, native *runners.NativeCatalog) *UserStreams {
	panic("not written: b-hostaccess")
}

// Lookup returns the named binding as an owned value, if declared.
func (s *UserStreams) Lookup(name string) (ServiceBinding, bool) { panic("not written: b-hostaccess") }

// UserStream transfers its pipes and lease to the edge, which defers Release.
type UserStream struct {
	// ToHost carries the page's bytes as invocation input.
	ToHost *remotehost.PipeWriter
	// FromHost carries invocation output until completion.
	FromHost *remotehost.PipeReader
	Lease    *Lease
}

// UserCallKind decides whether a call wakes Cloud and counts as activity.
type UserCallKind uint8

const (
	// Starts is new work: ordinary Host demand that wakes Cloud.
	Starts UserCallKind = iota
	// Operates changes running state: activity, but never wakes Cloud.
	Operates
	// Looks reads running state: no activity and never wakes Cloud.
	Looks
)

// ServiceCall carries a native operation and the bound on its JSON answer.
// Args is an object encoded by a generated encoder or contract.EncodeJSON;
// the receiving operation validates it through its generated decoder.
type ServiceCall struct {
	Binding  ServiceBinding
	Args     json.RawMessage
	MaxBytes int
}

// OpenUserStream opens the main Host without waking Cloud. It drops the file
// gate after admission and holds stream activity until completion or revocation.
func OpenUserStream(ctx context.Context, shard HostShard, id webapi.ConversationID, binding ServiceBinding) (*UserStream, error) {
	panic("not written: b-hostaccess")
}

// UserCall returns a one-shot call's JSON answer. Starts holds ordinary Host
// access; other calls register like streams and transitions end them.
func UserCall(ctx context.Context, shard HostShard, id webapi.ConversationID, kind UserCallKind, call ServiceCall) ([]byte, error) {
	panic("not written: b-hostaccess")
}
