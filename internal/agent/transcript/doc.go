// Package transcript owns a session's ordered blocks, patch journal, identities,
// replay, estimates, history cuts, and retirement of expired tool media.
// The session serializes access to a Log. Returned block snapshots must not be
// mutated by callers. This package performs no storage or provider calls.
package transcript
