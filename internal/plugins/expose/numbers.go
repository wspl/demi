package expose

import (
	"cmp"
	"context"
	"errors"
	"fmt"
	"maps"
	"slices"

	"github.com/wspl/demi/internal/plugin"
)

type numberedExpose struct {
	number uint64
	expose plugin.ExposeRecord
}

// numbered assigns persistent, never-reused numbers, oldest first, and preserves list order.
func numbered(ctx context.Context, port plugin.Port, exposes []plugin.ExposeRecord) ([]numberedExpose, error) {
	for {
		stored, err := port.Value(ctx, "numbers")
		if err != nil {
			return nil, err
		}
		var revision *uint64
		read := numbers{Next: 1, Exposes: map[string]uint64{}}
		if stored != nil {
			revision = &stored.Revision
			read, err = decodeNumbers(stored.Value)
			if err != nil {
				return nil, fmt.Errorf("the expose numbers do not read: %w", err)
			}
		}
		next := numbers{Next: read.Next, Exposes: maps.Clone(read.Exposes)}
		for id := range next.Exposes {
			if !slices.ContainsFunc(exposes, func(e plugin.ExposeRecord) bool { return string(e.ID) == id }) {
				delete(next.Exposes, id)
			}
		}
		pending := make([]plugin.ExposeRecord, 0, len(exposes))
		for _, e := range exposes {
			if _, exists := next.Exposes[string(e.ID)]; !exists {
				pending = append(pending, e)
			}
		}
		slices.SortStableFunc(pending, func(a, b plugin.ExposeRecord) int {
			if order := cmp.Compare(a.CreatedAt, b.CreatedAt); order != 0 {
				return order
			}
			return cmp.Compare(a.ID, b.ID)
		})
		for _, e := range pending {
			next.Exposes[string(e.ID)] = next.Next
			next.Next++
		}
		if next.Next != read.Next || !maps.Equal(next.Exposes, read.Exposes) {
			value, err := next.MarshalJSON()
			if err != nil {
				return nil, err
			}
			_, err = port.WriteValue(ctx, "numbers", value, revision)
			var conflict *plugin.PortRefusalConflict
			if errors.As(err, &conflict) {
				continue
			}
			if err != nil {
				return nil, err
			}
		}
		result := make([]numberedExpose, 0, len(exposes))
		for _, e := range exposes {
			result = append(result, numberedExpose{number: next.Exposes[string(e.ID)], expose: e})
		}
		return result, nil
	}
}
