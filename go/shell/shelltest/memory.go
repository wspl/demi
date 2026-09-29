// Package shelltest provides reusable Host conformance cases and in-memory
// collaborators for command and environment scenarios.
package shelltest

import (
	"context"
	"encoding/json/jsontext"
	"runtime"
	"slices"
	"sort"
	"strings"
	"sync"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/shell"
)

func CommandContext() commandservice.CommandContext {
	return commandservice.CommandContext{Conversation: "test-conversation", Caller: commandservice.AgentCaller{Number: 1}, Locale: commandservice.CommandLocale{TimeZone: "Asia/Shanghai", Languages: []string{"zh-CN", "en"}}}
}

// CountingNumbers starts each sequence at one. Tests may call it from concurrent
// handlers, so this test implementation synchronizes its in-memory store.
type CountingNumbers struct {
	mu   sync.Mutex
	next map[core.Sequence]uint64
}

func (n *CountingNumbers) Next(_ context.Context, sequence core.Sequence) (uint64, error) {
	n.mu.Lock()
	defer n.mu.Unlock()
	if n.next == nil {
		n.next = map[core.Sequence]uint64{}
	}
	n.next[sequence]++
	return n.next[sequence], nil
}

type MemoryStorage struct {
	mu       sync.Mutex
	values   map[string]jsontext.Value
	revision shell.Revision
}

func (s *MemoryStorage) Value(key string) jsontext.Value {
	s.mu.Lock()
	defer s.mu.Unlock()
	return slices.Clone(s.values[key])
}
func (s *MemoryStorage) Apply(op shell.StorageOp) shell.StorageReply {
	s.mu.Lock()
	defer s.mu.Unlock()
	switch op := op.(type) {
	case shell.StorageRead:
		value := slices.Clone(s.values[op.Key])
		if value == nil {
			value = jsontext.Value("null")
		}
		return shell.StorageValue{Value: value, Revision: s.revision}
	case shell.StorageList:
		keys := []string{}
		for key := range s.values {
			if strings.HasPrefix(key, op.Prefix) {
				keys = append(keys, key)
			}
		}
		sort.Strings(keys)
		return shell.StorageKeys{Keys: keys}
	case shell.StorageWriteIf:
		if op.Expected != nil && *op.Expected != s.revision {
			return shell.StorageConflict{Revision: s.revision}
		}
		if s.values == nil {
			s.values = map[string]jsontext.Value{}
		}
		if len(op.Value) == 0 || string(op.Value) == "null" {
			delete(s.values, op.Key)
		} else {
			s.values[op.Key] = slices.Clone(op.Value)
		}
		s.revision++
		return shell.StorageCommitted{Revision: s.revision}
	}
	panic("unreachable storage operation")
}

type MemoryPort struct {
	mu             sync.Mutex
	stdin, live    [][]byte
	stdout, stderr []byte
	Storage        *MemoryStorage
}

func NewMemoryPort(storage *MemoryStorage) *MemoryPort {
	if storage == nil {
		storage = &MemoryStorage{}
	}
	return &MemoryPort{Storage: storage}
}
func (p *MemoryPort) Port(ctx context.Context) shell.RPCPort {
	return shell.RPCPort{Transport: p, Context: ctx}
}
func (p *MemoryPort) FeedStdin(data []byte) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.stdin = append(p.stdin, slices.Clone(data))
}
func (p *MemoryPort) FeedLive(data []byte) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.live = append(p.live, slices.Clone(data))
}
func (p *MemoryPort) Stdout() []byte {
	p.mu.Lock()
	defer p.mu.Unlock()
	return slices.Clone(p.stdout)
}
func (p *MemoryPort) Stderr() []byte {
	p.mu.Lock()
	defer p.mu.Unlock()
	return slices.Clone(p.stderr)
}
func (p *MemoryPort) Request(_ context.Context, request shell.PortRequest) (shell.PortResponse, error) {
	runtime.Gosched() // Real ports yield at their transport; competing handlers can interleave.
	p.mu.Lock()
	defer p.mu.Unlock()
	switch request := request.(type) {
	case shell.PortStdout:
		p.stdout = append(p.stdout, request.Bytes.Bytes()...)
		return shell.PortWritten{}, nil
	case shell.PortStderr:
		p.stderr = append(p.stderr, request.Bytes.Bytes()...)
		return shell.PortWritten{}, nil
	case shell.PortReadStdin, shell.PortReadLiveStdin:
		queue := &p.stdin
		if _, live := request.(shell.PortReadLiveStdin); live {
			queue = &p.live
		}
		if len(*queue) == 0 {
			return shell.PortInput{}, nil
		}
		bytes := core.NewB64Bytes((*queue)[0])
		*queue = (*queue)[1:]
		return shell.PortInput{Bytes: &bytes}, nil
	case shell.PortStorageRequest:
		return shell.PortStorageResponse{Reply: p.Storage.Apply(request.Op)}, nil
	}
	panic("unreachable port request")
}

// Pages retains change snapshots and exposes event notifications. Its methods
// are safe for a scenario's observer and environment to call concurrently.
type Pages struct {
	mu                        sync.Mutex
	watching                  bool
	watchChanged, viewChanged chan struct{}
	views                     []shell.PageView
}

func NewPages(watching bool) *Pages {
	return &Pages{watching: watching, watchChanged: make(chan struct{}), viewChanged: make(chan struct{})}
}
func (p *Pages) Watch(watching bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.watching = watching
	close(p.watchChanged)
	p.watchChanged = make(chan struct{})
}
func (p *Pages) Watching() (bool, <-chan struct{}) {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.watching, p.watchChanged
}
func (p *Pages) Changed(view shell.PageView) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.views = append(p.views, view)
	close(p.viewChanged)
	p.viewChanged = make(chan struct{})
}
func (p *Pages) Next(ctx context.Context) (shell.PageView, error) {
	for {
		p.mu.Lock()
		if len(p.views) > 0 {
			view := p.views[0]
			p.views = p.views[1:]
			p.mu.Unlock()
			return view, nil
		}
		changed := p.viewChanged
		p.mu.Unlock()
		select {
		case <-ctx.Done():
			return shell.PageView{}, ctx.Err()
		case <-changed:
		}
	}
}
func (p *Pages) Drain() []shell.PageView {
	p.mu.Lock()
	defer p.mu.Unlock()
	views := p.views
	p.views = nil
	return views
}
