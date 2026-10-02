package session

import "time"

// Config controls how a session saves, retries and compacts. Use DefaultConfig
// for the production policy; zero durations and a nil threshold are explicit.
type Config struct {
	// PersistInterval is how long after the first unsaved change a save starts.
	PersistInterval time.Duration
	Retry           RetryPolicy
	Compaction      CompactionConfig
}

// DefaultConfig returns the one-second save interval and default retry and
// compaction policies.
func DefaultConfig() Config {
	return Config{PersistInterval: time.Second, Retry: DefaultRetryPolicy(), Compaction: DefaultCompactionConfig()}
}

// RetryPolicy controls when and how long a failed request is retried.
type RetryPolicy struct {
	// MaxAttempts counts attempts per request, the first included.
	MaxAttempts uint32
	// BaseDelay caps the random wait before the second attempt; it doubles
	// with each retry.
	BaseDelay time.Duration
	// MaxDelay is the backoff ceiling. A vendor wait beyond it makes the
	// failure terminal at once.
	MaxDelay time.Duration
}

// DefaultRetryPolicy returns four attempts, a one-second base delay and a
// thirty-second ceiling.
func DefaultRetryPolicy() RetryPolicy {
	return RetryPolicy{MaxAttempts: 4, BaseDelay: time.Second, MaxDelay: 30 * time.Second}
}

// CompactionConfig controls when a session compacts.
type CompactionConfig struct {
	// ThresholdPercent is the share of a model's context window and each
	// vendor request limit at which history is compacted. Nil disables
	// automatic compaction, as in a session copy.
	ThresholdPercent *uint8
}

// DefaultCompactionConfig returns the eighty-percent threshold.
func DefaultCompactionConfig() CompactionConfig {
	return CompactionConfig{ThresholdPercent: new(uint8(80))}
}
