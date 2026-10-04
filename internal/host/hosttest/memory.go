package hosttest

import (
	"bytes"
	"context"
	"encoding/json"
	"runtime"
	"sort"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

// MemoryStorage implements one node's revisioned command storage.
type MemoryStorage struct {
	mu       sync.Mutex
	values   map[string]json.RawMessage
	revision host.Revision
}

// Value returns an independent copy of a stored value, or nil when absent.
func (s *MemoryStorage) Value(key string) json.RawMessage {
	s.mu.Lock()
	defer s.mu.Unlock()
	return bytes.Clone(s.values[key])
}

// Apply performs an atomic storage operation; every committed write advances the revision.
func (s *MemoryStorage) Apply(op host.StorageOp) host.StorageReply {
	s.mu.Lock()
	defer s.mu.Unlock()
	switch op := op.(type) {
	case *host.StorageRead:
		value := bytes.Clone(s.values[op.Key])
		if value == nil {
			value = json.RawMessage("null")
		}
		return &host.StorageValue{Value: value, Revision: s.revision}
	case *host.StorageList:
		keys := []string{}
		for key := range s.values {
			if strings.HasPrefix(key, op.Prefix) {
				keys = append(keys, key)
			}
		}
		sort.Strings(keys)
		return &host.StorageKeys{Keys: keys}
	case *host.StorageWriteIf:
		if op.Expected != nil && *op.Expected != s.revision {
			return &host.StorageConflict{Revision: s.revision}
		}
		if s.values == nil {
			s.values = map[string]json.RawMessage{}
		}
		if len(op.Value) == 0 || bytes.Equal(bytes.TrimSpace(op.Value), []byte("null")) {
			delete(s.values, op.Key)
		} else {
			s.values[op.Key] = bytes.Clone(op.Value)
		}
		s.revision++
		return &host.StorageCommitted{Revision: s.revision}
	}
	return nil
}

// MemoryPort keeps finite and live inputs, captured output, and shared command storage.
// Every request yields before serving, allowing concurrent handlers to interleave.
type MemoryPort struct {
	mu             sync.Mutex
	stdin, live    [][]byte
	stdout, stderr []byte
	storage        *MemoryStorage
}

// NewMemoryPort makes a port, using fresh storage when storage is nil.
func NewMemoryPort(storage *MemoryStorage) *MemoryPort {
	if storage == nil {
		storage = &MemoryStorage{}
	}
	return &MemoryPort{storage: storage}
}

// FeedStdin queues one chunk of finite input.
func (p *MemoryPort) FeedStdin(chunk []byte) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.stdin = append(p.stdin, append([]byte{}, chunk...))
}

// FeedLive queues one interactive input chunk.
func (p *MemoryPort) FeedLive(chunk []byte) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.live = append(p.live, append([]byte{}, chunk...))
}

// Port returns the handler's port over this memory transport.
func (p *MemoryPort) Port() host.RPCPort {
	return host.NewRPCPort(p)
}

// Stdout returns a copy of captured stdout.
func (p *MemoryPort) Stdout() []byte {
	p.mu.Lock()
	defer p.mu.Unlock()
	return bytes.Clone(p.stdout)
}

// Stderr returns a copy of captured stderr.
func (p *MemoryPort) Stderr() []byte {
	p.mu.Lock()
	defer p.mu.Unlock()
	return bytes.Clone(p.stderr)
}

// Request serves one port operation.
func (p *MemoryPort) Request(_ context.Context, request host.PortRequest) (host.PortResponse, error) {
	runtime.Gosched()
	if storage, ok := request.(*host.PortStorage); ok {
		return &host.PortStored{Reply: p.storage.Apply(storage.Op)}, nil
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	switch request := request.(type) {
	case *host.PortStdout:
		p.stdout = append(p.stdout, request.Bytes...)
		return &host.PortWritten{}, nil
	case *host.PortStderr:
		p.stderr = append(p.stderr, request.Bytes...)
		return &host.PortWritten{}, nil
	case *host.PortReadStdin:
		return takeInput(&p.stdin), nil
	case *host.PortReadLiveStdin:
		return takeInput(&p.live), nil
	case *host.PortStorage: // Served outside the port lock above.
	}
	return nil, nil
}

// takeInput consumes a queued port chunk, preserving empty chunks apart from EOF.
func takeInput(queue *[][]byte) host.PortResponse {
	if len(*queue) == 0 {
		return &host.PortInput{}
	}
	data := types.B64Bytes((*queue)[0])
	*queue = (*queue)[1:]
	return &host.PortInput{Bytes: &data}
}

// CountingNumbers assigns each conversation sequence numbers starting at one.
type CountingNumbers struct {
	mu   sync.Mutex
	next map[types.Sequence]uint64
}

// Next returns the next number of the given sequence.
func (n *CountingNumbers) Next(_ context.Context, sequence types.Sequence) (uint64, error) {
	n.mu.Lock()
	defer n.mu.Unlock()
	if n.next == nil {
		n.next = map[types.Sequence]uint64{}
	}
	n.next[sequence]++
	return n.next[sequence], nil
}

// CommandContext is the test context, with locale different from the backend default.
func CommandContext() commandproto.Context {
	return commandproto.Context{
		Conversation: "test-conversation",
		Caller:       &commandproto.AgentCaller{Number: 1},
		Locale: commandproto.CommandLocale{
			TimeZone:  "Asia/Shanghai",
			Languages: []commandproto.LanguageTag{"zh-CN", "en"},
		},
	}
}

// Pages captures every reported page view and exposes watching changes without goroutines.
type Pages struct {
	mu            sync.Mutex
	watching      bool
	changed, told chan struct{}
	views         []host.PageView
}

// NewPages creates a feed with the initial watching state.
func NewPages(watching bool) *Pages {
	return &Pages{watching: watching, changed: make(chan struct{}), told: make(chan struct{})}
}

// Watch shows or hides a page, waking every watcher.
func (p *Pages) Watch(watching bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.watching = watching
	close(p.changed)
	p.changed = make(chan struct{})
}

// Watching returns an atomic snapshot and notification for its next change.
func (p *Pages) Watching() (bool, <-chan struct{}) {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.watching, p.changed
}

// Changed captures a command's current view.
func (p *Pages) Changed(record *host.CommandRecord) {
	view := record.PageView()
	p.mu.Lock()
	defer p.mu.Unlock()
	p.views = append(p.views, view)
	close(p.told)
	p.told = make(chan struct{})
}

// Next waits for the next reported change or the end of ctx.
func (p *Pages) Next(ctx context.Context) (host.PageView, error) {
	for {
		p.mu.Lock()
		if len(p.views) > 0 {
			view := p.views[0]
			p.views = p.views[1:]
			p.mu.Unlock()
			return view, nil
		}
		told := p.told
		p.mu.Unlock()
		select {
		case <-ctx.Done():
			return host.PageView{}, ctx.Err()
		case <-told:
		}
	}
}

// Drain forgets and returns all views not yet taken.
func (p *Pages) Drain() []host.PageView {
	p.mu.Lock()
	defer p.mu.Unlock()
	views := p.views
	p.views = nil
	return views
}
