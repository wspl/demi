package claudecode

import (
	"cmp"
	"context"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

type claudeVersion struct{ major, minor uint32 }

func version(id string) *claudeVersion {
	rest, ok := strings.CutPrefix(id, "claude-")
	if !ok {
		return nil
	}
	parts := strings.Split(rest, "-")
	number := func(index int) (uint32, bool) {
		if index >= len(parts) || parts[index] == "" {
			return 0, false
		}
		for _, r := range parts[index] {
			if r < '0' || r > '9' {
				return 0, false
			}
		}
		n, err := strconv.ParseUint(parts[index], 10, 32)
		return uint32(n), err == nil
	}
	index := 0
	major, ok := number(index)
	if !ok {
		index = 1
		major, ok = number(index)
	}
	if !ok {
		return nil
	}
	minor := uint32(0)
	if index+1 < len(parts) && len(parts[index+1]) != 8 {
		minor, _ = number(index + 1)
	}
	return &claudeVersion{major: major, minor: minor}
}
func (p *Provider) ListModels(ctx context.Context) (core.ProviderModelList, error) {
	snapshot, err := p.models.Refreshed(ctx)
	if err != nil {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: err.Error()}
	}
	list := snapshot.VendorModels("anthropic")
	if list == nil {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogInvalid, Message: "models.dev does not list the anthropic vendor"}
	}
	models := make([]core.ProviderModel, 0, len(list.Models))
	for _, model := range list.Models {
		if !strings.HasPrefix(model.ID, "claude-") {
			continue
		}
		v := version(model.ID)
		if v == nil {
			list.Warnings = append(list.Warnings, "Skipped Claude model with unparseable version: "+model.ID)
			continue
		}
		if v.major < 4 || v.major == 4 && v.minor < 6 {
			continue
		}
		model.CanDisableThinking = new(false)
		models = append(models, model)
	}
	family := func(id string) int {
		rest := strings.TrimPrefix(id, "claude-")
		family, _, _ := strings.Cut(rest, "-")
		switch family {
		case "opus":
			return 0
		case "sonnet":
			return 1
		case "haiku":
			return 2
		default:
			return 3
		}
	}
	slices.SortFunc(models, func(a, b core.ProviderModel) int {
		if order := cmp.Compare(family(a.ID), family(b.ID)); order != 0 {
			return order
		}
		av, bv := version(a.ID), version(b.ID)
		if order := cmp.Compare(bv.major, av.major); order != 0 {
			return order
		}
		if order := cmp.Compare(bv.minor, av.minor); order != 0 {
			return order
		}
		return strings.Compare(a.ID, b.ID)
	})
	list.Models = models
	return *list, nil
}
