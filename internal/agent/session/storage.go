package session

import (
	"bytes"
	"context"
	"encoding/json"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/host"
)

// GenerationNumber returns the current command-storage generation, recorded
// by a job started now. Rewrites and disposal invalidate that generation.
func (s *Session) GenerationNumber() uint64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.generation
}

// JobStorage serves a job's message while its call context and recorded
// generation are current. A write returns once its version commits. Refusals
// return *host.PortError; cancellation of an admission wait returns ctx.Err().
func (s *Session) JobStorage(ctx context.Context, number uint64, op host.StorageOp) (host.StorageReply, error) {
	s.mu.Lock()
	current, lifetime := s.core.generation, s.core.generationCtx
	s.mu.Unlock()
	if current != number {
		return nil, storageError(store.ErrInvalidated)
	}
	return s.Storage(ctx, op, store.NewCommitGuard(lifetime, ctx))
}

// Storage serves one command-storage message guarded by its generation and
// call lifetimes. The guard is checked immediately before commit; a commit
// already started finishes. Reads observe the current version and writes
// return once durable. Edit preparation refuses storage messages.
func (s *Session) Storage(ctx context.Context, op host.StorageOp, guard store.CommitGuard) (host.StorageReply, error) {
	return s.storage(ctx, op, guard)
}

// storageError preserves the store cause while presenting the shell's port error.
func storageError(err error) error {
	return &host.PortError{Kind: host.StorageRefused, Message: err.Error(), Err: err}
}

func (s *Session) storage(ctx context.Context, op host.StorageOp, guard store.CommitGuard) (host.StorageReply, error) {
	key, err := storageKey(op)
	if err != nil {
		return nil, err
	}
	write, isWrite := op.(*host.StorageWriteIf)
	if isWrite {
		permit, err := s.persist.Acquire(ctx)
		if err != nil {
			return nil, err
		}
		defer permit.Release()
	}
	s.mu.Lock()
	if err := guard.Check(); err != nil {
		s.mu.Unlock()
		return nil, storageError(err)
	}
	if s.core.preparingEditLocked() {
		s.mu.Unlock()
		return nil, &host.PortError{
			Kind:    host.StorageRefused,
			Message: "Command storage is reserved for a transcript edit",
		}
	}
	values, revision := s.core.commands.Values(), s.core.commands.Revision()
	if !isWrite {
		s.mu.Unlock()
		reply := readStorage(op, key, values, revision)
		if reply != nil {
			return reply, nil
		}
	}
	if write.Expected != nil && uint64(*write.Expected) != revision {
		s.mu.Unlock()
		return &host.StorageConflict{Revision: host.Revision(revision)}, nil
	}
	if len(write.Value) == 0 || bytes.Equal(bytes.TrimSpace(write.Value), []byte("null")) {
		delete(values, store.CommandStorageKey(key))
	} else {
		values[store.CommandStorageKey(key)] = write.Value
	}
	pending, err := s.core.commands.Prepare(values)
	s.mu.Unlock()
	if err != nil {
		return nil, storageError(err)
	}
	if pending == nil {
		return &host.StorageCommitted{Revision: host.Revision(revision)}, nil
	}
	if err = s.save(ctx, pending, guard); err != nil {
		return nil, storageError(err)
	}
	s.mutate(func(c *coreState) { c.commands.Accept(*pending) })
	return &host.StorageCommitted{Revision: host.Revision(pending.Revision)}, nil
}

func readStorage(
	op host.StorageOp,
	key string,
	values map[store.CommandStorageKey]json.RawMessage,
	revision uint64,
) host.StorageReply {
	switch op.(type) {
	case *host.StorageRead:
		value := values[store.CommandStorageKey(key)]
		if value == nil {
			value = []byte("null")
		}
		return &host.StorageValue{Value: value, Revision: host.Revision(revision)}
	case *host.StorageList:
		keys := []string{}
		for name := range values {
			if strings.HasPrefix(string(name), key) {
				keys = append(keys, string(name))
			}
		}
		slices.Sort(keys)
		return &host.StorageKeys{Keys: keys}
	case *host.StorageWriteIf:
	}
	return nil
}

func storageKey(op host.StorageOp) (string, error) {
	var key string
	switch v := op.(type) {
	case *host.StorageRead:
		key = v.Key
	case *host.StorageList:
		key = v.Prefix
	case *host.StorageWriteIf:
		key = v.Key
	}
	if _, list := op.(*host.StorageList); !list || key != "" {
		if _, err := store.ParseCommandStorageKey(key); err != nil {
			return "", storageError(err)
		}
	}
	return key, nil
}
