package usershard

import (
	"context"
	"errors"
	"sync"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) productState(ctx context.Context, user webapi.UserDTO) (webapi.ProductState, error) {
	state := webapi.ProductState{User: user, Mode: s.services.Mode}
	var err error
	state.Preferences, err = s.Control().Preferences(ctx, s.user)
	if err != nil {
		return state, err
	}
	state.Providers, err = s.providerStates(ctx, user)
	if err != nil {
		return state, err
	}
	state.Workspaces, err = s.workspaceDTOs(ctx)
	if err != nil {
		return state, err
	}
	state.Devices, err = s.DeviceList(ctx)
	if err != nil {
		return state, err
	}
	state.Conversations, err = s.ConversationSummaries(ctx, false)
	if err != nil {
		return state, err
	}
	archived, err := s.ConversationSummaries(ctx, true)
	if err != nil {
		return state, err
	}
	state.Conversations = append(state.Conversations, archived...)
	state.Cloud, err = cloud.Status(ctx, s)
	if err != nil {
		return state, err
	}
	state.Plugins, err = s.plugins.Entries(ctx)
	if err != nil {
		return state, err
	}
	state.PluginStates, err = s.plugins.PageStates(ctx)
	if err != nil {
		return state, err
	}
	url, ok := s.services.PublicURL.URL()
	if !ok {
		return state, errors.New("the backend listens before it serves a request")
	}
	state.PublicURL = url.String()
	return state, nil
}
func (s *Shard) workspaceDTOs(ctx context.Context) ([]webapi.WorkspaceDTO, error) {
	records, err := s.Control().Workspaces(ctx, s.user)
	if err != nil {
		return nil, err
	}
	result := make([]webapi.WorkspaceDTO, 0, len(records))
	for _, record := range records {
		result = append(result, record.DTO())
	}
	return result, nil
}
func (s *Shard) readPart(ctx context.Context, part pagesync.Part, user webapi.UserDTO) (webapi.SyncEvent, error) {
	switch part.Kind {
	case pagesync.Conversation:
		record, err := s.Control().Conversation(ctx, part.ConversationID)
		if err != nil || record == nil {
			return nil, err
		}
		if record.Owner != s.user {
			return nil, nil
		}
		summary, err := s.ConversationSummary(ctx, *record)
		return &webapi.SyncEventConversation{Conversation: summary}, err
	case pagesync.ConversationOrder:
		ids, err := s.Control().ConversationOrder(ctx, s.user)
		return &webapi.SyncEventConversationOrder{IDs: ids}, err
	case pagesync.Preferences:
		preferences, err := s.Control().Preferences(ctx, s.user)
		return &webapi.SyncEventPreferences{Preferences: preferences}, err
	case pagesync.User:
		account, err := s.Control().Account(ctx, s.user)
		if err != nil || account == nil {
			return nil, err
		}
		return &webapi.SyncEventUser{User: account.User}, nil
	case pagesync.Workspaces:
		workspaces, err := s.workspaceDTOs(ctx)
		return &webapi.SyncEventWorkspaces{Workspaces: workspaces}, err
	case pagesync.Devices:
		devices, err := s.DeviceList(ctx)
		return &webapi.SyncEventDevices{Devices: devices}, err
	case pagesync.Plugins:
		plugins, err := s.plugins.Entries(ctx)
		return &webapi.SyncEventPlugins{Plugins: plugins}, err
	case pagesync.Plugin:
		state, err := s.plugins.PageState(ctx, part.PluginID)
		if err != nil || state == nil {
			return nil, err
		}
		return &webapi.SyncEventPlugin{Plugin: part.PluginID, State: state}, nil
	case pagesync.Providers:
		providers, err := s.providerStates(ctx, user)
		return &webapi.SyncEventProviders{Providers: providers}, err
	case pagesync.Cloud:
		status, err := cloud.Status(ctx, s)
		return &webapi.SyncEventCloud{Cloud: status}, err
	}
	return nil, nil
}
func (s *Shard) providerStates(ctx context.Context, user webapi.UserDTO) ([]webapi.ProviderState, error) {
	owner, err := s.services.Vault.OwnerFor(ctx, user.ID)
	if err != nil {
		return nil, err
	}
	entries, err := s.services.Vault.Entries(ctx, owner)
	if err != nil {
		return nil, err
	}
	disclose := s.services.Vault.Configures(user)
	result := make([]webapi.ProviderState, len(entries))
	var workers sync.WaitGroup
	for i, entry := range entries {
		workers.Add(1)
		go func() {
			defer workers.Done()
			details, err := s.services.Assembly.Details(ctx, entry, disclose)
			var reading webapi.ProviderReading = &webapi.ProviderReadingRead{ProviderDetails: details}
			if err != nil {
				reading = &webapi.ProviderReadingFailed{Message: err.Error()}
			}
			result[i] = webapi.ProviderState{ProviderDTO: entry.DTO(), Details: reading}
		}()
	}
	workers.Wait()
	return result, nil
}
