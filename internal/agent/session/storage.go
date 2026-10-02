package session

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/host"
)

// GenerationNumber returns the current command-storage generation, recorded
// by a job started now. Rewrites and disposal invalidate that generation.
func (s *Session) GenerationNumber() uint64 { panic("not written: a-session") }

// JobStorage serves a job's message while its call context and recorded
// generation are current. A write returns once its version commits. Refusals
// return *host.PortError; cancellation of an admission wait returns ctx.Err().
func (s *Session) JobStorage(ctx context.Context, number uint64, op host.StorageOp) (host.StorageReply, error) {
	panic("not written: a-session")
}

// Storage serves one command-storage message guarded by its generation and
// call lifetimes. The guard is checked immediately before commit; a commit
// already started finishes. Reads observe the current version and writes
// return once durable. Edit preparation refuses storage messages.
func (s *Session) Storage(ctx context.Context, op host.StorageOp, guard store.CommitGuard) (host.StorageReply, error) {
	panic("not written: a-session")
}
