package usershard

import (
	"errors"
	"math"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/host"
)

// The plugin sees the existing failed-port category, never a new refusal kind.
// Cost: pure conversion, no storage or external services.
func TestExposeLifetimeRangeIsPortFailure(t *testing.T) {
	const maximum = uint64(math.MaxInt64 / int64(time.Second))
	duration, err := exposeLifetime(maximum)
	if err != nil || duration != time.Duration(maximum)*time.Second {
		t.Fatalf("last representable lifetime = %v, %v", duration, err)
	}
	for _, seconds := range []uint64{maximum + 1, math.MaxUint64} {
		_, err := exposeLifetime(seconds)
		var port *host.PortError
		var storage *database.Error
		if !errors.As(err, &port) || port.Kind != host.PortFailed || !errors.As(err, &storage) ||
			storage.Kind != database.TimeRange {
			t.Fatalf("overflow classification = %v", err)
		}
		if port.Message != "a time is out of range: expose lifetime exceeds 9223372036 seconds" {
			t.Fatalf("overflow message = %q", port.Message)
		}
	}
}
