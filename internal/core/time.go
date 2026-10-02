package core

import (
	"fmt"
	"time"
)

const timestampLayout = "2006-01-02T15:04:05.000Z"

// UnixEpoch is the origin of millisecond timestamps.
const UnixEpoch Timestamp = "1970-01-01T00:00:00.000Z"

// Clock supplies record timestamps. Elapsed durations use Go's monotonic clock.
type Clock interface{ Now() Timestamp }

// SystemClock reads wall time at millisecond precision.
type SystemClock struct{}

// Now reads the current wall time, truncated to milliseconds.
func (SystemClock) Now() Timestamp {
	return Timestamp(time.Now().UTC().Format(timestampLayout))
}

// TimestampFromMillisecond constructs a canonical timestamp in the supported calendar.
func TimestampFromMillisecond(ms int64) (Timestamp, error) {
	value, err := TimestampFromTime(time.UnixMilli(ms))
	if err != nil {
		return "", fmt.Errorf("timestamp from milliseconds %d: %w", ms, err)
	}
	return value, nil
}

// TimestampFromTime truncates a Go time to canonical UTC milliseconds.
func TimestampFromTime(t time.Time) (Timestamp, error) {
	value := Timestamp(t.UTC().Format(timestampLayout))
	if err := value.Validate(); err != nil {
		return "", fmt.Errorf("timestamp from time: %w", err)
	}
	return value, nil
}

// ParseTimestamp validates the canonical spelling at a text boundary.
func ParseTimestamp(text string) (Timestamp, error) {
	value := Timestamp(text)
	if err := value.Validate(); err != nil {
		return "", fmt.Errorf("timestamp: %w", err)
	}
	return value, nil
}

// Time returns the timestamp as a Go time, refusing invalid constructed values.
func (t Timestamp) Time() (time.Time, error) {
	if err := t.Validate(); err != nil {
		return time.Time{}, fmt.Errorf("timestamp: %w", err)
	}
	value, err := time.Parse(timestampLayout, string(t))
	if err != nil {
		return time.Time{}, fmt.Errorf("timestamp: %w", err)
	}
	return value, nil
}

// Millisecond returns the whole milliseconds since the Unix epoch.
func (t Timestamp) Millisecond() (int64, error) {
	value, err := t.Time()
	if err != nil {
		return 0, err
	}
	return value.UnixMilli(), nil
}
