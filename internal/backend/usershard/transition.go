package usershard

import (
	"context"
	"errors"
	"log/slog"
	"reflect"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) applyChange(
	ctx context.Context,
	id webapi.ConversationID,
	change database.ConversationChange,
) error {
	record, err := s.services.Control.Conversation(ctx, id)
	if err != nil {
		return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeStorage, Cause: err}
	}
	if record == nil || record.Owner != s.user {
		return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeNotFound}
	}
	var field database.RecordChange
	if c, ok := change.(*database.ConversationRecordChange); ok {
		field = c.Change
	}
	_, archive := field.(*database.RecordArchived)
	if record.Archived && !archive {
		return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeArchived}
	}
	if target, ok := change.(*database.ConversationTargetChange); ok {
		if reflect.DeepEqual(record.Target, target.Target) {
			return nil
		}
		if err := hostaccess.CheckDestination(ctx, s, *record, target.Target); err != nil {
			return err
		}
	}
	slot := s.conversations.Slot(id)
	if settings, ok := change.(*database.ConversationSettingsChange); ok {
		return s.commitSettingsChange(ctx, id, settings.Change)
	}
	_, detach := field.(*database.RecordDetach)
	ordinaryField := field != nil && !archive && !detach
	if ordinaryField {
		admitted, err := slot.FileGate().Enter(ctx, gates.Demand)
		if err != nil {
			return err
		}
		defer admitted.Release()
		return s.commitRecordField(ctx, *record, field)
	}
	reserved, err := s.reserveConversationChange(id)
	if err != nil {
		return err
	}
	hold, err := hostaccess.HoldForTransition(ctx, s, id, reserved)
	if err != nil {
		return err
	}
	defer hold.Release()
	ctx = context.WithoutCancel(ctx)
	return s.commitReservedChange(ctx, *record, change)
}

func (s *Shard) changeSettings(
	ctx context.Context,
	id webapi.ConversationID,
	change database.SettingsChange,
) error {
	record, err := s.services.Control.Conversation(ctx, id)
	if err != nil {
		return err
	}
	if record == nil {
		return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeNotFound}
	}
	if record.Archived {
		return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeArchived}
	}
	selection, err := s.settingsSelection(ctx, record.Model, change)
	if err != nil {
		return err
	}
	root := hostaccess.RootOf(id)
	prepared, err := s.agent.PrepareSwitch(ctx, root, selection)
	if err != nil {
		if errors.Is(err, server.ErrProviderUnavailable) {
			return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeProviderNotFound}
		}
		return &hostaccess.ChangeRefusal{
			Kind:  hostaccess.ChangeRuntime,
			Cause: err,
		}
	}
	ctx = context.WithoutCancel(ctx)
	committed := hostaccess.Commit(ctx, s, id, &database.RecordModel{Model: selection})
	if prepared != nil {
		if committed == nil {
			return s.agent.SwitchModel(ctx, root, *prepared)
		}
		return errors.Join(committed, prepared.Discard(ctx))
	}
	return committed
}

func (s *Shard) settingsSelection(
	ctx context.Context,
	current *core.ModelSelection,
	change database.SettingsChange,
) (core.ModelSelection, error) {
	var selection core.ModelSelection
	var entry webapi.ProviderID
	var model string
	if change.Model != nil {
		entry = change.Model.ProviderID
		model = change.Model.ModelID
	} else {
		if current == nil {
			return selection, &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeModelNotSelected}
		}
		selection = *current
		var err error
		entry, err = webapi.ParseProviderID(current.ProviderID)
		if err != nil {
			return selection, &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeProviderNotFound}
		}
		model = current.Model.ID
	}
	listed, err := s.listedModel(ctx, entry, model)
	if err != nil {
		return selection, err
	}
	var effort, tier *string
	if change.ThinkingEffort != nil {
		effort = *change.ThinkingEffort
	}
	if change.ServiceTierID != nil {
		tier = *change.ServiceTierID
	}
	if change.Model != nil || change.ThinkingEffort != nil {
		selection.Thinking, err = listed.ThinkingFor(effort)
		if err != nil {
			return selection, &hostaccess.ChangeRefusal{
				Kind:  hostaccess.ChangeSettingUnavailable,
				Cause: err,
			}
		}
	}
	if change.Model != nil || change.ServiceTierID != nil {
		selection.ServiceTierID, err = listed.TierFor(tier)
		if err != nil {
			return selection, &hostaccess.ChangeRefusal{
				Kind:  hostaccess.ChangeSettingUnavailable,
				Cause: err,
			}
		}
	}
	if change.Model != nil {
		selection = listed.Selection(string(entry), selection.Thinking, selection.ServiceTierID)
	}
	return selection, nil
}

func (s *Shard) listedModel(
	ctx context.Context,
	id webapi.ProviderID,
	model string,
) (core.ProviderModel, error) {
	entry, err := s.services.Vault.Visible(ctx, s.user, id)
	if err != nil {
		return core.ProviderModel{}, err
	}
	if entry == nil {
		return core.ProviderModel{}, &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeProviderNotFound}
	}
	built, buildErr := s.services.Assembly.ProviderFor(ctx, *entry)
	catalog := s.services.Assembly.EntryCatalog(ctx, *entry, built, buildErr, false)
	for _, listed := range catalog.Models {
		if listed.ID == model {
			return listed, nil
		}
	}
	return core.ProviderModel{}, &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeModelNotFound}
}

func (s *Shard) stopTitle(id webapi.ConversationID) {
	s.mu.Lock()
	title := s.titles[id]
	delete(s.titles, id)
	s.mu.Unlock()
	if title != nil {
		title.cancel()
	}
}

func (s *Shard) applyPatch(
	ctx context.Context,
	id webapi.ConversationID,
	patch webapi.ConversationPatch,
) (*webapi.ConversationUpdate, error) {
	record, err := s.services.Control.Conversation(ctx, id)
	if err != nil || record == nil {
		return nil, err
	}
	if record.Owner != s.user {
		return nil, nil
	}
	changes := patchChanges(patch)
	results := make([]webapi.FieldResult, 0)
	for _, change := range changes {
		err := s.transition(ctx, id, change.change)
		for _, field := range change.fields {
			if err == nil {
				results = append(results, &webapi.FieldResultApplied{Field: field})
				continue
			}
			refusal := &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeStorage, Cause: err}
			errors.As(err, &refusal)
			if refusal.Kind == hostaccess.ChangeStorage {
				slog.ErrorContext(ctx, "a conversation change failed", "field", field, "error", err)
			}
			code, status := refusal.Code()
			results = append(
				results,
				&webapi.FieldResultFailed{
					Field:      field,
					Code:       code,
					Message:    refusal.Error(),
					HTTPStatus: uint16(status),
				},
			)
		}
	}
	record, err = s.services.Control.Conversation(ctx, id)
	if err != nil || record == nil {
		return nil, err
	}
	summary, err := s.ConversationSummary(ctx, *record)
	if err != nil {
		return nil, err
	}
	return &webapi.ConversationUpdate{Conversation: summary, Results: results}, nil
}

func (s *Shard) transition(
	ctx context.Context,
	id webapi.ConversationID,
	change database.ConversationChange,
) error {
	if err := s.applyChange(ctx, id, change); err != nil {
		var refused *hostaccess.ChangeRefusal
		if errors.As(err, &refused) || errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return err
		}
		return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeStorage, Cause: err}
	}
	s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
	if c, ok := change.(*database.ConversationRecordChange); ok {
		switch c.Change.(type) {
		case *database.RecordPinned, *database.RecordArchived:
			s.Mark(pagesync.Part{Kind: pagesync.ConversationOrder})
		case *database.RecordAttach,
			*database.RecordDetach,
			*database.RecordModel,
			*database.RecordRename,
			*database.RecordTitle:
		}
	}
	return nil
}

// commitRecordField checks an attached host before committing an ordinary field.
func (s *Shard) commitRecordField(
	ctx context.Context,
	record database.ConversationRecord,
	field database.RecordChange,
) error {
	if attach, ok := field.(*database.RecordAttach); ok {
		target, err := hostaccess.ResolveTarget(ctx, s, record)
		if err != nil {
			return err
		}
		device := database.ExecutionDeviceID(target)
		if device != nil && *device == attach.Host.Device {
			return &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeHostIsMain}
		}
	}
	return hostaccess.Commit(context.WithoutCancel(ctx), s, record.ID, field)
}

// commitReservedChange applies a change while the conversation transition hold is owned.
func (s *Shard) commitReservedChange(
	ctx context.Context,
	record database.ConversationRecord,
	change database.ConversationChange,
) error {
	var err error
	id := record.ID
	switch c := change.(type) {
	case *database.ConversationTargetChange:
		err = hostaccess.SwitchTarget(ctx, s, record, c.Target)
		if err == nil {
			s.stopIdle(id)
			s.TrackIdle(id)
		}
	case *database.ConversationRecordChange:
		switch c := c.Change.(type) {
		case *database.RecordArchived:
			err = s.commitArchive(ctx, record, c)
		case *database.RecordDetach:
			err = hostaccess.Detach(ctx, s, record, c.Device)
		case *database.RecordAttach,
			*database.RecordModel,
			*database.RecordPinned,
			*database.RecordRename,
			*database.RecordTitle:
			err = hostaccess.Commit(ctx, s, id, c)
		}
	case *database.ConversationSettingsChange:
		// Settings returned before reserving the idle tree.
	}
	return err
}

// commitArchive stops title and idle work after a successful archive commit.
func (s *Shard) commitArchive(
	ctx context.Context,
	record database.ConversationRecord,
	change *database.RecordArchived,
) error {
	if !change.Archived {
		return hostaccess.Commit(ctx, s, record.ID, change)
	}
	err := hostaccess.Archive(ctx, s, record)
	if err == nil {
		s.stopTitle(record.ID)
		s.stopIdle(record.ID)
	}
	return err
}

type modification struct {
	fields []webapi.PatchField
	change database.ConversationChange
}

// patchChanges preserves the independent patch fields in archive-first order.
func patchChanges(patch webapi.ConversationPatch) []modification {
	var changes []modification
	if patch.Archived != nil {
		changes = append(
			changes,
			modification{
				[]webapi.PatchField{webapi.PatchFieldArchived},
				&database.ConversationRecordChange{
					Change: &database.RecordArchived{Archived: *patch.Archived},
				},
			},
		)
	}
	if patch.Title != nil {
		changes = append(
			changes,
			modification{
				[]webapi.PatchField{webapi.PatchFieldTitle},
				&database.ConversationRecordChange{Change: &database.RecordTitle{Title: string(*patch.Title)}},
			},
		)
	}
	if patch.Pinned != nil {
		changes = append(
			changes,
			modification{
				[]webapi.PatchField{webapi.PatchFieldPinned},
				&database.ConversationRecordChange{Change: &database.RecordPinned{Pinned: *patch.Pinned}},
			},
		)
	}
	changes = appendPatchSettings(changes, patch)
	if patch.Target != nil {
		changes = append(
			changes,
			modification{
				[]webapi.PatchField{webapi.PatchFieldTarget},
				&database.ConversationTargetChange{Target: *patch.Target},
			},
		)
	}
	return changes
}

// commitSettingsChange owns file and settings admission while changing the selected model.
func (s *Shard) commitSettingsChange(
	ctx context.Context,
	id webapi.ConversationID,
	change database.SettingsChange,
) error {
	slot := s.conversations.Slot(id)
	admitted, err := slot.FileGate().Enter(ctx, gates.Demand)
	if err != nil {
		return err
	}
	defer admitted.Release()
	turn, err := slot.Settings().Acquire(ctx)
	if err != nil {
		return err
	}
	defer turn.Release()
	return s.changeSettings(ctx, id, change)
}

// appendPatchSettings groups model, thinking effort and tier into one settings change.
func appendPatchSettings(changes []modification, patch webapi.ConversationPatch) []modification {
	var settings []webapi.PatchField
	if patch.Model != nil {
		settings = append(settings, webapi.PatchFieldModel)
	}
	if patch.ThinkingEffort != nil {
		settings = append(settings, webapi.PatchFieldThinkingEffort)
	}
	if patch.ServiceTierID != nil {
		settings = append(settings, webapi.PatchFieldServiceTierID)
	}
	if len(settings) > 0 {
		changes = append(
			changes,
			modification{
				settings,
				&database.ConversationSettingsChange{
					Change: database.SettingsChange{
						Model:          patch.Model,
						ThinkingEffort: patch.ThinkingEffort,
						ServiceTierID:  patch.ServiceTierID,
					},
				},
			},
		)
	}
	return changes
}

// reserveConversationChange reserves a quiescent live turn or refuses an in-flight change.
func (s *Shard) reserveConversationChange(id webapi.ConversationID) (*gates.Reservation, error) {
	var reserved *gates.Reservation
	if tree := s.agent.Tree(hostaccess.RootOf(id)); tree != nil {
		reserved = tree.Admission().TryReserve()
		if reserved == nil {
			return nil, &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeTurnInFlight}
		}
		if !tree.IsQuiescent() {
			reserved.Release()
			return nil, &hostaccess.ChangeRefusal{Kind: hostaccess.ChangeTurnInFlight}
		}
	}
	return reserved, nil
}
