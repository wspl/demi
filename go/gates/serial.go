package gates

import (
	"context"
	"sync"

	"golang.org/x/sync/semaphore"
)

// A SerialGate admits one holder at a time, in arrival order. A permit is a
// value that can be handed to the goroutine that does the work it admits.
type SerialGate struct {
	turn *semaphore.Weighted
}

// NewSerialGate returns a gate that nobody holds.
func NewSerialGate() *SerialGate {
	return &SerialGate{turn: semaphore.NewWeighted(1)}
}

// Acquire holds the gate once every earlier caller has released it. It returns
// the context's error, and holds nothing, when ctx ends first.
func (g *SerialGate) Acquire(ctx context.Context) (*SerialPermit, error) {
	if err := g.turn.Acquire(ctx, 1); err != nil {
		return nil, err
	}
	return &SerialPermit{gate: g}, nil
}

// TryAcquire holds the gate now, or none while someone holds or waits for it.
func (g *SerialGate) TryAcquire() (*SerialPermit, bool) {
	if !g.turn.TryAcquire(1) {
		return nil, false
	}
	return &SerialPermit{gate: g}, true
}

// A SerialPermit is the turn a [SerialGate] gave; releasing it gives the next
// caller its turn.
type SerialPermit struct {
	gate *SerialGate
	once sync.Once
}

// Release gives up the turn.
func (p *SerialPermit) Release() {
	p.once.Do(func() { p.gate.turn.Release(1) })
}

// A KeyedSerialGate is a serial gate for each key, for example a conversation:
// callers of one key take turns, and callers of different keys do not wait for
// each other. A key's gate exists only while some caller holds or waits for it.
type KeyedSerialGate[K comparable] struct {
	// mu guards gates and each entry's users. Each section only looks up,
	// inserts or removes an entry and never waits.
	mu    sync.Mutex
	gates map[K]*keyedTurn
}

// A keyedTurn is one key's gate, and how many callers hold or wait for it.
type keyedTurn struct {
	turn  *semaphore.Weighted
	users int
}

// NewKeyedSerialGate returns a gate with no key in use.
func NewKeyedSerialGate[K comparable]() *KeyedSerialGate[K] {
	return &KeyedSerialGate[K]{gates: map[K]*keyedTurn{}}
}

// Acquire holds key's turn once every earlier caller of the same key has
// released it. It returns the context's error, and holds nothing, when ctx
// ends first.
func (g *KeyedSerialGate[K]) Acquire(ctx context.Context, key K) (*KeyedPermit[K], error) {
	g.mu.Lock()
	entry := g.gates[key]
	if entry == nil {
		entry = &keyedTurn{turn: semaphore.NewWeighted(1)}
		g.gates[key] = entry
	}
	entry.users++
	g.mu.Unlock()
	if err := entry.turn.Acquire(ctx, 1); err != nil {
		g.leave(key, entry)
		return nil, err
	}
	return &KeyedPermit[K]{gate: g, key: key, entry: entry}, nil
}

// leave ends one caller's use of a key, and forgets the key when nobody else
// uses it.
func (g *KeyedSerialGate[K]) leave(key K, entry *keyedTurn) {
	g.mu.Lock()
	defer g.mu.Unlock()
	entry.users--
	if entry.users == 0 {
		delete(g.gates, key)
	}
}

// Len returns how many keys have a holder or a waiter.
func (g *KeyedSerialGate[K]) Len() int {
	g.mu.Lock()
	defer g.mu.Unlock()
	return len(g.gates)
}

// A KeyedPermit is one key's turn; releasing it gives the next caller of the
// key its turn, and forgets the key when nobody holds or waits for it.
type KeyedPermit[K comparable] struct {
	gate  *KeyedSerialGate[K]
	key   K
	entry *keyedTurn
	once  sync.Once
}

// Release gives up the turn.
func (p *KeyedPermit[K]) Release() {
	p.once.Do(func() {
		p.entry.turn.Release(1)
		p.gate.leave(p.key, p.entry)
	})
}
