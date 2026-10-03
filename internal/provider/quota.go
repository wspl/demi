package provider

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/wspl/demi/internal/core"
)

// ProbeCost describes whether a quota probe spends inference.
type ProbeCost uint8

// Costs a quota source may incur.
const (
	// ProbeFree identifies a quota probe that spends no inference.
	ProbeFree ProbeCost = iota
	// ProbeInference identifies a quota probe that spends inference.
	ProbeInference
)

// Observation is an HTTP response or a CLI line that may report account quota.
//
//sumtype:decl
type Observation interface{ quotaObservation() }

// HTTPObservation holds the headers of a vendor response.
type HTTPObservation struct {
	Status  int
	Headers http.Header
}

// CLIObservation holds a decoded vendor CLI line.
type CLIObservation struct{ Line json.RawMessage }

func (*HTTPObservation) quotaObservation() {}
func (*CLIObservation) quotaObservation()  {}

// ProbeReading is what a vendor usage endpoint reports about an account.
type ProbeReading struct {
	Plan         *core.QuotaPlan
	AccountLabel *string
	Windows      []core.QuotaWindow
}

// ErrQuotaUnsupported means the provider cannot read its usage.
var ErrQuotaUnsupported = errors.New("this provider cannot read its usage")

// ErrQuotaRequiresInference means reading the provider's usage would spend an inference request.
var ErrQuotaRequiresInference = errors.New("reading this provider's usage requires an inference request")

// QuotaSource knows a family's usage endpoint and live quota fields.
type QuotaSource interface {
	// ProbeCost reports the cost of a quota probe; ok is false when the family cannot probe.
	ProbeCost() (cost ProbeCost, ok bool)
	Probe(context.Context) (ProbeReading, error)
	Observe(Observation) []core.QuotaWindow
}

// QuotaSnapshotStore merges each update exactly once, atomically with storing it.
// Returned snapshots are independent values, owned by the caller.
type QuotaSnapshotStore interface {
	Latest() *core.QuotaSnapshot
	Update(func(*core.QuotaSnapshot) core.QuotaSnapshot) *core.QuotaSnapshot
}

// Quota is the quota of the particular account a provider stands for.
type Quota struct {
	source QuotaSource
	store  QuotaSnapshotStore
	clock  core.Clock
}

// NewQuota connects a family source to its account's store.
func NewQuota(source QuotaSource, store QuotaSnapshotStore, clock core.Clock) *Quota {
	return &Quota{source: source, store: store, clock: clock}
}

// Latest reads the kept snapshot without probing.
func (q *Quota) Latest() *core.QuotaSnapshot { return q.store.Latest() }

// ProbeCost reports the probe cost; ok is false when the family cannot probe.
func (q *Quota) ProbeCost() (ProbeCost, bool) { return q.source.ProbeCost() }

// Probe reads a free usage endpoint and merges its plan, label and windows.
func (q *Quota) Probe(ctx context.Context) (*core.QuotaSnapshot, error) {
	cost, ok := q.source.ProbeCost()
	if !ok {
		return nil, ErrQuotaUnsupported
	}
	if cost == ProbeInference {
		return nil, ErrQuotaRequiresInference
	}
	reading, err := q.source.Probe(ctx)
	if err != nil {
		return nil, err
	}
	now := q.clock.Now()
	return q.store.Update(func(previous *core.QuotaSnapshot) core.QuotaSnapshot {
		return mergeQuota(previous, reading, "probe", now)
	}), nil
}

// Observe merges readable live windows while preserving the account's plan and label.
func (q *Quota) Observe(observation Observation) {
	windows := q.source.Observe(observation)
	if windows == nil {
		return
	}
	now := q.clock.Now()
	q.store.Update(func(previous *core.QuotaSnapshot) core.QuotaSnapshot {
		reading := ProbeReading{Windows: windows}
		if previous != nil {
			reading.Plan = previous.Plan
			reading.AccountLabel = previous.AccountLabel
		}
		return mergeQuota(previous, reading, "observation", now)
	})
}

// mergeQuota preserves unnamed windows and replaces named windows in place.
func mergeQuota(
	previous *core.QuotaSnapshot,
	reading ProbeReading,
	source core.SnapshotSource,
	now core.Timestamp,
) core.QuotaSnapshot {
	windows := make([]core.QuotaWindow, 0)
	if previous != nil {
		windows = append(windows, previous.Windows...)
	}
	for _, window := range reading.Windows {
		found := false
		for i := range windows {
			if windows[i].ID == window.ID {
				windows[i] = window
				found = true
				break
			}
		}
		if !found {
			windows = append(windows, window)
		}
	}
	return core.QuotaSnapshot{
		ObservedAt:   now,
		Source:       source,
		Plan:         reading.Plan,
		AccountLabel: reading.AccountLabel,
		Windows:      windows,
	}
}

// MemorySnapshots is an in-memory account snapshot store. Zero is ready to use.
type MemorySnapshots struct {
	mu     sync.Mutex
	latest *core.QuotaSnapshot
}

// Latest returns an independent snapshot without IO.
func (s *MemorySnapshots) Latest() *core.QuotaSnapshot {
	s.mu.Lock()
	defer s.mu.Unlock()
	return cloneSnapshot(s.latest)
}

// Update atomically calls next once and stores the result. The callback must
// neither block nor call back into the store.
func (s *MemorySnapshots) Update(next func(*core.QuotaSnapshot) core.QuotaSnapshot) *core.QuotaSnapshot {
	s.mu.Lock()
	defer s.mu.Unlock()
	value := next(cloneSnapshot(s.latest))
	s.latest = cloneSnapshot(&value)
	return cloneSnapshot(s.latest)
}

// cloneSnapshot prevents caller mutations from changing a kept account snapshot.
func cloneSnapshot(s *core.QuotaSnapshot) *core.QuotaSnapshot {
	if s == nil {
		return nil
	}
	value := *s
	if s.Plan != nil {
		p := *s.Plan
		value.Plan = &p
	}
	if s.AccountLabel != nil {
		p := *s.AccountLabel
		value.AccountLabel = &p
	}
	value.Windows = append([]core.QuotaWindow{}, s.Windows...)
	for i := range value.Windows {
		w := &value.Windows[i]
		if w.UsedPercent != nil {
			p := *w.UsedPercent
			w.UsedPercent = &p
		}
		if w.Used != nil {
			p := *w.Used
			w.Used = &p
		}
		if w.Limit != nil {
			p := *w.Limit
			w.Limit = &p
		}
		if w.Unit != nil {
			p := *w.Unit
			w.Unit = &p
		}
		if w.ResetsAt != nil {
			p := *w.ResetsAt
			w.ResetsAt = &p
		}
		if w.Severity != nil {
			p := *w.Severity
			w.Severity = &p
		}
		if w.Scope != nil {
			p := *w.Scope
			if p.Label != nil {
				label := *p.Label
				p.Label = &label
			}
			w.Scope = &p
		}
	}
	return &value
}

// ClampUsedPercent clips a finite percentage to the range zero through one hundred.
func ClampUsedPercent(value float64) *float64 {
	if math.IsNaN(value) || math.IsInf(value, 0) {
		return nil
	}
	value = max(0, min(100, value))
	return &value
}

// UsedPercentFromRatio reads a share only when its finite limit is positive.
func UsedPercentFromRatio(used, limit float64) *float64 {
	if math.IsNaN(used) || math.IsInf(used, 0) || math.IsNaN(limit) || math.IsInf(limit, 0) || limit <= 0 {
		return nil
	}
	return ClampUsedPercent(used / limit * 100)
}

// Severity reads critical from 95 percent and warning from 80 percent.
func Severity(used *float64) *core.QuotaSeverity {
	if used == nil {
		return nil
	}
	result := core.QuotaSeverity("normal")
	if *used >= 95 {
		result = "critical"
	} else if *used >= 80 {
		result = "warning"
	}
	return &result
}

// UnixSeconds reads a reset time, flooring fractional seconds to milliseconds.
func UnixSeconds(seconds float64) *core.Timestamp {
	ms := math.Floor(seconds * 1000)
	if math.IsNaN(ms) || math.IsInf(ms, 0) || math.Abs(ms) >= 9e15 {
		return nil
	}
	result, err := core.TimestampFromMillisecond(int64(ms))
	if err != nil {
		return nil
	}
	return &result
}

// RFC3339 reads a vendor's reset time at millisecond precision.
func RFC3339(text string) *core.Timestamp {
	parsed, err := time.Parse(time.RFC3339Nano, strings.TrimSpace(text))
	if err != nil {
		return nil
	}
	result, err := core.TimestampFromTime(parsed)
	if err != nil {
		return nil
	}
	return &result
}

// HeaderNumber reads a trimmed, finite vendor header number.
func HeaderNumber(headers http.Header, name string) *float64 {
	value, err := strconv.ParseFloat(strings.TrimSpace(headers.Get(name)), 64)
	if err != nil || math.IsNaN(value) || math.IsInf(value, 0) {
		return nil
	}
	return &value
}
