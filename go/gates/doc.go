// Package gates holds the named gates that serialize work across waits
// (docs/architecture/concurrency.md, Locks). An [ActivityGate] admits leases
// and quiets for reservations, a [SerialGate] admits one holder at a time, and a
// [KeyedSerialGate] keeps a serial gate for each key while anyone uses it.
//
// Go has no destructors, so every lease, reservation and permit is released by
// its Release method, or by the defer of the code that took it; releasing twice
// is the same as releasing once. A wait takes a [context.Context], and one that
// is cancelled gives up its place in the queue. The gates admit in arrival
// order, and a wait that is given up never admits anyone.
package gates
