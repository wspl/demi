package provider

import (
	"context"
	"encoding/json/jsontext"
	"errors"
	"math"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/wspl/demi/go/core"
)

type ProbeCost uint8

const (
	ProbeFree ProbeCost = iota + 1
	ProbeInference
)

type Observation struct {
	Status  uint16
	Headers http.Header
	CLILine jsontext.Value
}
type ProbeReading struct {
	Plan         *core.QuotaPlan
	AccountLabel *string
	Windows      []core.QuotaWindow
}
type QuotaSource interface {
	ProbeCost() *ProbeCost
	Probe(context.Context) (ProbeReading, error)
	Observe(Observation) ([]core.QuotaWindow, bool)
}

var (
	ErrQuotaUnsupported       = errors.New("this provider cannot read its usage")
	ErrQuotaRequiresInference = errors.New("reading this provider's usage requires an inference request")
)

// QuotaSnapshotStore calls the merge exactly once, atomically with publication.
// Snapshots passed to or returned from this interface are immutable.
type QuotaSnapshotStore interface {
	Latest() *core.QuotaSnapshot
	Update(func(*core.QuotaSnapshot) core.QuotaSnapshot) *core.QuotaSnapshot
}
type MemorySnapshots struct {
	mu     sync.Mutex
	latest *core.QuotaSnapshot
}

func (s *MemorySnapshots) Latest() *core.QuotaSnapshot {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.latest
}
func (s *MemorySnapshots) Update(next func(*core.QuotaSnapshot) core.QuotaSnapshot) *core.QuotaSnapshot {
	s.mu.Lock()
	defer s.mu.Unlock()
	value := next(s.latest)
	s.latest = &value
	return s.latest
}

type ProviderQuota struct {
	source QuotaSource
	store  QuotaSnapshotStore
	clock  core.Clock
}

func NewProviderQuota(source QuotaSource, store QuotaSnapshotStore, clock core.Clock) *ProviderQuota {
	return &ProviderQuota{source: source, store: store, clock: clock}
}
func (q *ProviderQuota) Latest() *core.QuotaSnapshot { return q.store.Latest() }
func (q *ProviderQuota) ProbeCost() *ProbeCost       { return q.source.ProbeCost() }
func (q *ProviderQuota) Probe(ctx context.Context) (*core.QuotaSnapshot, error) {
	cost := q.ProbeCost()
	if cost == nil {
		return nil, ErrQuotaUnsupported
	}
	if *cost == ProbeInference {
		return nil, ErrQuotaRequiresInference
	}
	reading, err := q.source.Probe(ctx)
	if err != nil {
		return nil, err
	}
	now := q.clock.Now()
	return q.store.Update(func(previous *core.QuotaSnapshot) core.QuotaSnapshot {
		return mergeQuota(previous, reading, core.SnapshotSourceProbe, now)
	}), nil
}
func (q *ProviderQuota) Observe(observation Observation) {
	windows, ok := q.source.Observe(observation)
	if !ok {
		return
	}
	now := q.clock.Now()
	q.store.Update(func(previous *core.QuotaSnapshot) core.QuotaSnapshot {
		reading := ProbeReading{Windows: windows}
		if previous != nil {
			reading.Plan = previous.Plan
			reading.AccountLabel = previous.AccountLabel
		}
		return mergeQuota(previous, reading, core.SnapshotSourceObservation, now)
	})
}

// mergeQuota replaces named windows in place and appends newly observed windows.
func mergeQuota(previous *core.QuotaSnapshot, reading ProbeReading, source core.SnapshotSource, now core.Timestamp) core.QuotaSnapshot {
	merged := []core.QuotaWindow{}
	if previous != nil {
		merged = append(merged, previous.Windows...)
	}
	for _, window := range reading.Windows {
		index := -1
		for i, kept := range merged {
			if kept.ID == window.ID {
				index = i
				break
			}
		}
		if index < 0 {
			merged = append(merged, window)
		} else {
			merged[index] = window
		}
	}
	return core.QuotaSnapshot{ObservedAt: now, Source: source, Plan: reading.Plan, AccountLabel: reading.AccountLabel, Windows: merged}
}
func ClampUsedPercent(value float64) *float64 {
	if math.IsNaN(value) || math.IsInf(value, 0) {
		return nil
	}
	value = max(0, min(100, value))
	return &value
}
func UsedPercentFromRatio(used, limit float64) *float64 {
	if math.IsNaN(used) || math.IsInf(used, 0) || math.IsNaN(limit) || math.IsInf(limit, 0) || limit <= 0 {
		return nil
	}
	return ClampUsedPercent(used / limit * 100)
}
func Severity(used *float64) *core.QuotaSeverity {
	if used == nil {
		return nil
	}
	value := core.QuotaSeverityNormal
	if *used >= 95 {
		value = core.QuotaSeverityCritical
	} else if *used >= 80 {
		value = core.QuotaSeverityWarning
	}
	return &value
}
func UnixSeconds(seconds float64) *core.Timestamp {
	ms := math.Floor(seconds * 1000)
	if math.IsNaN(ms) || math.IsInf(ms, 0) || math.Abs(ms) >= 9e15 {
		return nil
	}
	stamp, err := core.TimestampFromMillisecond(int64(ms))
	if err != nil {
		return nil
	}
	return &stamp
}
func RFC3339(text string) *core.Timestamp {
	parsed, err := time.Parse(time.RFC3339Nano, strings.TrimSpace(text))
	if err != nil {
		return nil
	}
	stamp := core.TruncateTimestamp(parsed)
	return &stamp
}
func HeaderNumber(headers http.Header, name string) *float64 {
	value, err := strconv.ParseFloat(strings.TrimSpace(headers.Get(name)), 64)
	if err != nil || math.IsNaN(value) || math.IsInf(value, 0) {
		return nil
	}
	return &value
}
