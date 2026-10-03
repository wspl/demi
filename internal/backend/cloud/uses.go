package cloud

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"time"

	"github.com/wspl/demi/internal/webapi"
)

type useRole uint8

const (
	targetRole useRole = iota
	providerRole
	attachedRole
)

type cloudUse struct {
	id   webapi.ConversationID
	role useRole
}

// cloudUses derives the strongest Cloud role of every unarchived conversation.
func cloudUses(ctx context.Context, s CloudShard) ([]cloudUse, error) {
	control := cloudRecords(s)
	device, err := control.ManagedDevice(ctx, s.User())
	if err != nil {
		return nil, err
	}
	var id *webapi.DeviceID
	if device != nil {
		id = &device.ID
	}
	conversations, err := control.CloudUses(ctx, s.User(), id)
	if err != nil {
		return nil, err
	}
	known := make(map[webapi.ProviderID]bool)
	var uses []cloudUse
	for _, conversation := range conversations {
		process := false
		if !conversation.OnCloud && conversation.Provider != nil {
			provider := *conversation.Provider
			var ok bool
			process, ok = known[provider]
			if !ok {
				process = providerRunsProcess(ctx, s, provider)
				known[provider] = process
			}
		}
		var role useRole
		switch {
		case conversation.OnCloud:
			role = targetRole
		case process:
			role = providerRole
		case conversation.Attached:
			role = attachedRole
		default:
			continue
		}
		uses = append(uses, cloudUse{id: conversation.ID, role: role})
	}
	return uses, nil
}

// releaseHolds releases every conversation acquired by one Cloud transition.
func releaseHolds(held []ConversationHold) {
	for _, h := range held {
		h.Release()
	}
}

// holdIdle reserves all required conversations now, or releases the partial set.
func holdIdle(s CloudShard, uses []cloudUse) ([]ConversationHold, bool) {
	var held []ConversationHold
	for _, use := range uses {
		if use.role == attachedRole {
			continue
		}
		hold := s.HoldForIdle(use.id)
		if hold == nil {
			releaseHolds(held)
			return nil, false
		}
		held = append(held, hold)
	}
	return held, true
}

// holdReset interrupts and holds conversations that cannot work without Cloud.
func holdReset(ctx context.Context, s CloudShard, uses []cloudUse, timeout time.Duration) ([]ConversationHold, error) {
	var held []ConversationHold
	for _, use := range uses {
		if use.role == attachedRole {
			continue
		}
		hold, err := s.HoldForReset(ctx, use.id, use.role == targetRole, timeout)
		if err != nil {
			releaseHolds(held)
			if errors.Is(err, ErrNotLetGo) {
				//nolint:staticcheck // Product text, shown to the user as it is.
				return nil, failed(fmt.Errorf("The conversation %s did not stop for the reset", use.id))
			}
			return nil, failed(err)
		}
		held = append(held, hold)
	}
	return held, nil
}

// providerRunsProcess reads and builds a provider for Cloud demand classification.
func providerRunsProcess(ctx context.Context, s CloudShard, provider webapi.ProviderID) bool {
	process := false
	entry, err := s.Vault().Visible(ctx, s.User(), provider)
	if err != nil {
		slog.Warn("a conversation's provider could not be read", "provider", provider, "error", err)
	} else if entry != nil {
		process, err = s.Assembly().RunsAProcess(ctx, *entry)
		if err != nil {
			slog.Warn("a conversation's provider could not be built", "provider", provider, "error", err)
			process = false
		}
	}
	return process
}
