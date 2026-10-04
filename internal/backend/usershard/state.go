package usershard

import (
	"context"
	"errors"
	"sync"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapiproto"
)

func (s *Shard) productState(ctx context.Context, user webapiproto.UserDTO) (webapiproto.ProductState, error) {
	state := webapiproto.ProductState{User: user, Mode: s.services.Mode}
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

func (s *Shard) workspaceDTOs(ctx context.Context) ([]webapiproto.WorkspaceDTO, error) {
	records, err := s.Control().Workspaces(ctx, s.user)
	if err != nil {
		return nil, err
	}
	result := make([]webapiproto.WorkspaceDTO, 0, len(records))
	for _, record := range records {
		result = append(result, record.DTO())
	}
	return result, nil
}

func (s *Shard) readPart(
	ctx context.Context,
	part pagesync.Part,
	user webapiproto.UserDTO,
) (webapiproto.SyncEvent, error) {
	switch part.Kind {
	case pagesync.Conversation:
		return s.conversationEvent(ctx, part.ConversationID)
	case pagesync.ConversationOrder:
		ids, err := s.Control().ConversationOrder(ctx, s.user)
		return &webapiproto.SyncEventConversationOrder{IDs: ids}, err
	case pagesync.Preferences:
		preferences, err := s.Control().Preferences(ctx, s.user)
		return &webapiproto.SyncEventPreferences{Preferences: preferences}, err
	case pagesync.User:
		account, found, err := s.Control().Account(ctx, s.user)
		if err != nil || !found {
			return nil, err
		}
		return &webapiproto.SyncEventUser{User: account.User}, nil
	case pagesync.Workspaces:
		workspaces, err := s.workspaceDTOs(ctx)
		return &webapiproto.SyncEventWorkspaces{Workspaces: workspaces}, err
	case pagesync.Devices:
		devices, err := s.DeviceList(ctx)
		return &webapiproto.SyncEventDevices{Devices: devices}, err
	case pagesync.Plugins:
		plugins, err := s.plugins.Entries(ctx)
		return &webapiproto.SyncEventPlugins{Plugins: plugins}, err
	case pagesync.Plugin:
		state, err := s.plugins.PageState(ctx, part.PluginID)
		if err != nil || state == nil {
			return nil, err
		}
		return &webapiproto.SyncEventPlugin{Plugin: part.PluginID, State: state}, nil
	case pagesync.Providers:
		providers, err := s.providerStates(ctx, user)
		return &webapiproto.SyncEventProviders{Providers: providers}, err
	case pagesync.Cloud:
		status, err := cloud.Status(ctx, s)
		return &webapiproto.SyncEventCloud{Cloud: status}, err
	}
	return nil, nil
}

func (s *Shard) providerStates(ctx context.Context, user webapiproto.UserDTO) ([]webapiproto.ProviderState, error) {
	owner, err := s.services.Vault.OwnerFor(ctx, user.ID)
	if err != nil {
		return nil, err
	}
	entries, err := s.services.Vault.Entries(ctx, owner)
	if err != nil {
		return nil, err
	}
	disclose := s.services.Vault.Configures(user)
	result := make([]webapiproto.ProviderState, len(entries))
	var workers sync.WaitGroup
	for i, entry := range entries {
		workers.Add(1)
		go func() {
			defer workers.Done()
			details, err := s.services.Assembly.Details(ctx, entry, disclose)
			var reading webapiproto.ProviderReading = &webapiproto.ProviderReadingRead{ProviderDetails: details}
			if err != nil {
				reading = &webapiproto.ProviderReadingFailed{Message: err.Error()}
			}
			result[i] = webapiproto.ProviderState{ProviderDTO: entry.DTO(), Details: reading}
		}()
	}
	workers.Wait()
	return result, nil
}

// conversationEvent presents a changed conversation only when it belongs to this user.
func (s *Shard) conversationEvent(ctx context.Context, id webapiproto.ConversationID) (webapiproto.SyncEvent, error) {
	record, found, err := s.Control().Conversation(ctx, id)
	if err != nil || !found {
		return nil, err
	}
	if record.Owner != s.user {
		return nil, nil
	}
	summary, err := s.ConversationSummary(ctx, record)
	return &webapiproto.SyncEventConversation{Conversation: summary}, err
}
