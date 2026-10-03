package claudecode

import (
	"cmp"
	"context"
	"errors"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/core"
)

// ListModels refreshes models.dev and lists Claude 4.6 and later, flagship first.
func (p *Provider) ListModels(ctx context.Context) (core.ProviderModelList, error) {
	snapshot, err := p.models.Refreshed(ctx)
	if err != nil {
		return core.ProviderModelList{}, err
	}
	list, ok := snapshot.VendorModels("anthropic")
	if !ok {
		return core.ProviderModelList{}, errors.New("models.dev does not list the anthropic vendor")
	}
	models := make([]core.ProviderModel, 0)
	for _, model := range list.Models {
		if !strings.HasPrefix(model.ID, "claude-") {
			continue
		}
		major, minor, ok := modelVersion(model.ID)
		if !ok {
			list.Warnings = append(list.Warnings, "Skipped Claude model with unparseable version: "+model.ID)
			continue
		}
		if major < 4 || major == 4 && minor < 6 {
			continue
		}
		off := false
		model.CanDisableThinking = &off
		models = append(models, model)
	}
	slices.SortFunc(models, func(a, b core.ProviderModel) int {
		if c := cmp.Compare(modelFamily(a.ID), modelFamily(b.ID)); c != 0 {
			return c
		}
		am, an, _ := modelVersion(a.ID)
		bm, bn, _ := modelVersion(b.ID)
		if c := cmp.Compare(bm, am); c != 0 {
			return c
		}
		if c := cmp.Compare(bn, an); c != 0 {
			return c
		}
		return cmp.Compare(a.ID, b.ID)
	})
	list.Models = models
	return list, nil
}

func modelVersion(id string) (uint64, uint64, bool) {
	rest, ok := strings.CutPrefix(id, "claude-")
	if !ok {
		return 0, 0, false
	}
	parts := strings.Split(rest, "-")
	number := func(i int) (uint64, bool) {
		if i >= len(parts) || parts[i] == "" {
			return 0, false
		}
		for _, c := range parts[i] {
			if c < '0' || c > '9' {
				return 0, false
			}
		}
		n, err := strconv.ParseUint(parts[i], 10, 32)
		return n, err == nil
	}
	i := 0
	major, ok := number(i)
	if !ok {
		i++
		major, ok = number(i)
	}
	if !ok {
		return 0, 0, false
	}
	var minor uint64
	if i+1 < len(parts) && len(parts[i+1]) != 8 {
		minor, _ = number(i + 1)
	}
	return major, minor, true
}

func modelFamily(id string) int {
	parts := strings.Split(id, "-")
	if len(parts) > 1 {
		switch parts[1] {
		case "opus":
			return 0
		case "sonnet":
			return 1
		case "haiku":
			return 2
		}
	}
	return 3
}
