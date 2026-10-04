package plugins

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"slices"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

func (r registered) ownsKind(kind string) bool {
	return r.manifest.Page != nil && slices.Contains(r.manifest.Page.PanelKinds, kind)
}

func (r *Registry) kindOwner(kind string) (int, bool) {
	for i, p := range r.plugins {
		if p.ownsKind(kind) {
			return i, true
		}
	}
	return 0, false
}

func unknownKind(kind string) error {
	return &plugin.PortRefusalPanel{
		Code:    webapi.ErrorCodeUnknownPanelKind,
		Message: (&database.PanelKindError{Kind: kind}).Error(),
	}
}

// ChangePanel applies a page's change and notifies the tab's plugin without
// waiting for the plugin's Host work.
func (u *User) ChangePanel(
	ctx context.Context,
	conversation webapi.ConversationID,
	change database.PanelChange,
) (uint64, error) {
	enabled, err := u.enabledSet(ctx)
	if err != nil {
		return 0, err
	}
	if create, ok := change.(database.PanelCreate); ok {
		owner, found := u.registry.kindOwner(create.Tab.Kind)
		if !found || !enabled[owner] {
			return 0, unknownKind(create.Tab.Kind)
		}
	}
	revision, effect, changed, err := u.applyPanel(ctx, conversation, change)
	if err != nil {
		return 0, err
	}
	if !changed || effect.Kind != database.PanelCreated && effect.Kind != database.PanelRemoved {
		return revision, nil
	}
	owner, found := u.registry.kindOwner(effect.Tab.Kind)
	if found && enabled[owner] {
		changed := plugin.PanelTabCreated
		if effect.Kind == database.PanelRemoved {
			changed = plugin.PanelTabRemoved
		}
		u.notify(
			owner,
			&plugin.RequestPanelTab{User: u.id, Conversation: conversation, Change: changed, Tab: effect.Tab},
			&conversation,
		)
	}
	return revision, nil
}

func (u *User) applyPanel(
	ctx context.Context,
	conversation webapi.ConversationID,
	change database.PanelChange,
) (uint64, database.PanelEffect, bool, error) {
	revision, effect, changed, err := u.control.ChangePanel(ctx, conversation, change)
	if err != nil {
		return 0, database.PanelEffect{}, false, panelRefusal(err)
	}
	u.markPanelChanged(conversation, changed)
	return revision, effect, changed, nil
}

func panelRefusal(err error) error {
	var code webapi.ErrorCode
	var kind *database.PanelKindError
	switch {
	case errors.As(err, &kind):
		return unknownKind(kind.Kind)
	case errors.Is(err, database.ErrPanelFull):
		code = webapi.ErrorCodePanelFull
	case errors.Is(err, database.ErrPanelTooLarge):
		code = webapi.ErrorCodeTooLarge
	case errors.Is(err, database.ErrArchived):
		return &plugin.PortRefusalPanel{
			Code:    webapi.ErrorCodeConversationArchived,
			Message: "The conversation is archived",
		}
	case errors.Is(err, sql.ErrNoRows):
		return &plugin.PortRefusalPanel{
			Code:    webapi.ErrorCodeConversationNotFound,
			Message: "No conversation of that id",
		}
	default:
		return err
	}
	return &plugin.PortRefusalPanel{Code: code, Message: err.Error()}
}

// notify admits background work before spawning it. Close cancels and joins it.
func (u *User) notify(index int, request plugin.Request, conversation *webapi.ConversationID) {
	u.notifications.Lock()
	if u.notificationCtx.Err() != nil {
		u.notifications.Unlock()
		return
	}
	u.notificationCalls.Add(1)
	u.notifications.Unlock()
	if conversation != nil {
		id := *conversation
		conversation = &id
	}
	go func() {
		defer u.notificationCalls.Done()
		if _, err := u.request(
			u.notificationCtx,
			index,
			request,
			conversation,
			nil,
		); err != nil &&
			!errors.Is(err, context.Canceled) {
			slog.Warn(
				"a plugin could not do its part of a change",
				"plugin",
				u.registry.plugins[index].manifest.ID,
				"error",
				err,
			)
		}
	}()
}

func (p requestPort) panelTabs(ctx context.Context) (plugin.PortAnswer, error) {
	conversation, err := p.conversationID()
	if err != nil {
		return nil, err
	}
	panel, err := p.user.control.Panel(ctx, conversation)
	if err != nil {
		return nil, storageError(err)
	}
	panel.Tabs = slices.DeleteFunc(
		panel.Tabs,
		func(tab webapi.PanelTab) bool { return !p.user.registry.plugins[p.index].ownsKind(tab.Kind) },
	)
	return &plugin.PortAnswerPanel{Panel: panel}, nil
}

func (p requestPort) panelChange(ctx context.Context, change database.PanelChange) (plugin.PortAnswer, error) {
	conversation, err := p.conversationID()
	if err != nil {
		return nil, err
	}
	var kinds []string
	if page := p.user.registry.plugins[p.index].manifest.Page; page != nil {
		kinds = page.PanelKinds
	}
	revision, _, changed, err := p.user.control.ChangePanelOfKinds(ctx, conversation, change, kinds)
	if err != nil {
		return nil, panelRefusal(err)
	}
	p.user.markPanelChanged(conversation, changed)
	return &plugin.PortAnswerPanelRevision{Revision: revision}, nil
}

func (p requestPort) answerPanel(ctx context.Context, m plugin.PortMessage) (plugin.PortAnswer, error) {
	switch m := any(m).(type) {
	case *plugin.PortMessagePanelTabs:
		return p.panelTabs(ctx)
	case *plugin.PortMessageCreatePanelTab:
		return p.panelChange(ctx, database.PanelCreate{Tab: m.Tab})
	case *plugin.PortMessageUpdatePanelTab:
		return p.panelChange(ctx, database.PanelUpdate{ID: m.ID, Data: m.Data})
	case *plugin.PortMessageRemovePanelTab:
		return p.panelChange(ctx, database.PanelRemove{ID: m.ID})
	}
	return nil, fmt.Errorf("unexpected panel message %T", m)
}

func (u *User) markPanelChanged(conversation webapi.ConversationID, changed bool) {
	if changed {
		u.marks.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: conversation})
	}
}
