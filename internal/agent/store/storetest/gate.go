package storetest

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import "context"

// StoreGate holds store calls as a slow database would. The acquiring test
// must release it during cleanup; canceled calls leave the gate promptly.
type StoreGate struct{}

// Waiting reports how many calls are waiting.
func (g *StoreGate) Waiting() int { panic("not written: a-store") }

// Wait waits for at least count calls to be held, without polling or sleeping.
func (g *StoreGate) Wait(ctx context.Context, count int) error { panic("not written: a-store") }

// Release lets current and future calls through; it is idempotent.
func (g *StoreGate) Release() { panic("not written: a-store") }
