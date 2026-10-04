package expose

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapiproto"
)

// How long an expose lives from its creation or its last renewal, in
// seconds: one hour (`expose.md` § Lifetime).
const lifetime uint64 = 60 * 60

func state(ctx context.Context, port plugin.Port) (json.RawMessage, error) {
	listed, err := port.Exposes(ctx)
	if err != nil {
		return nil, err
	}
	entries, err := numbered(ctx, port, listed.Exposes)
	if err != nil {
		return nil, err
	}
	result := ExposeState{Available: listed.Available, Exposes: make([]ExposeEntry, 0, len(entries))}
	for _, e := range entries {
		result.Exposes = append(
			result.Exposes,
			ExposeEntry{
				ID:         e.expose.ID,
				Number:     e.number,
				DeviceID:   e.expose.Device,
				DeviceName: e.expose.DeviceName,
				Address:    e.expose.Address,
				URL:        e.expose.URL,
				ExpiresAt:  e.expose.ExpiresAt,
			},
		)
	}
	return result.MarshalJSON()
}

func pageCall(ctx context.Context, method string, params json.RawMessage, port plugin.Port) (json.RawMessage, error) {
	args, err := DecodeExposeCall(params)
	if err != nil {
		return nil, &plugin.ErrorUsage{Message: err.Error()}
	}
	notFound := &plugin.ErrorRefused{Reason: "expose_not_found", Message: "No expose " + args.Expose}
	id, err := webapiproto.ParseExposeID(args.Expose)
	if err != nil {
		return nil, notFound
	}
	switch method {
	case "renew":
		_, err = port.RenewExpose(ctx, id, lifetime)
	case "remove":
		err = port.RemoveExpose(ctx, id)
	default:
		return nil, &plugin.ErrorFailed{Message: fmt.Sprintf("the expose plugin has no method %s", method)}
	}
	var refused *plugin.PortRefusalExpose
	if errors.As(err, &refused) && refused.Reason == plugin.ExposeRefusalNotFound {
		return nil, notFound
	}
	if err != nil {
		return nil, plugin.RequestError(err)
	}
	return json.RawMessage("null"), nil
}
