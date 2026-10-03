package cloud

import "time"

// CloudTuning holds the Cloud's times and limits. Tests shorten the times and
// lower the capacity. It is immutable once shared with a Cloud.
//
//nolint:revive // The architecture names this configuration boundary CloudTuning.
type CloudTuning struct {
	// Sweep is how often a running Cloud's maintenance and idle watch look at it.
	Sweep time.Duration
	// CheckpointInterval is how long a running Cloud goes between checkpoints.
	CheckpointInterval time.Duration
	// LifetimeCap is how long a Cloud runs before it is stopped, unless a turn is in flight.
	LifetimeCap time.Duration
	// RunnerConnection is how long a Cloud's runner has to connect after a boot,
	// or to reconnect during a recovery.
	RunnerConnection time.Duration
	// CrashLoopDeaths is how many runtime losses within CrashLoopWindow stop
	// the Cloud's automatic boots.
	CrashLoopDeaths uint32
	// CrashLoopWindow is the window in which runtime losses are counted.
	CrashLoopWindow time.Duration
	// SyncTimeout is how long a Cloud's runner has to flush its filesystems before a save.
	SyncTimeout time.Duration
	// ResetHold is how long a reset, or the lifetime cap, waits for the work it
	// stops to let go.
	ResetHold time.Duration
	// SystemQuota is the largest system filesystem a Cloud may grow to, in bytes.
	SystemQuota uint64
	// HomeQuota is the largest home filesystem a Cloud may grow to, in bytes.
	HomeQuota uint64
	// Capacity is how many Clouds, across every user, may be booting, running,
	// saving or resetting at once.
	Capacity int
}

// DefaultTuning returns the shipped lifecycle policy: a 30-second sweep,
// 15-minute checkpoints, a 24-hour lifetime cap, 60-second runner connection,
// three deaths in ten minutes, a five-second flush, a 30-second reset hold,
// 16 GiB system and 32 GiB home quotas, and capacity for 16 Clouds.
func DefaultTuning() CloudTuning {
	return CloudTuning{Sweep: 30 * time.Second, CheckpointInterval: 15 * time.Minute, LifetimeCap: 24 * time.Hour, RunnerConnection: time.Minute, CrashLoopDeaths: 3, CrashLoopWindow: 10 * time.Minute, SyncTimeout: 5 * time.Second, ResetHold: 30 * time.Second, SystemQuota: 16 << 30, HomeQuota: 32 << 30, Capacity: 16}
}
