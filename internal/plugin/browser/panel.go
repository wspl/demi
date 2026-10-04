package browser

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapiproto"
)

const panelKind = "browser"

// What a `browser` tab keeps.
// +demi:root
// +demi:tolerant
type tabData struct {
	URL string `json:"url"`
	// +demi:nullable
	Tab *string `json:"tab,omitempty"`
	// +demi:nullable
	Closed  *bool            `json:"closed,omitempty"`
	Failure *json.RawMessage `json:"failure,omitempty"`
}

// enter serializes one conversation's browser work without holding a mutex over IO.
func (i *instance) enter(ctx context.Context, conversation webapiproto.ConversationID) (func(), error) {
	i.mu.Lock()
	if i.turns == nil {
		i.turns = map[webapiproto.ConversationID]chan struct{}{}
	}
	turn := i.turns[conversation]
	if turn == nil {
		turn = make(chan struct{}, 1)
		i.turns[conversation] = turn
	}
	i.mu.Unlock()
	select {
	case turn <- struct{}{}:
		return func() {
			<-turn
		}, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func (d tabData) live() (string, bool) {
	if d.Tab == nil || d.Closed != nil && *d.Closed {
		return "", false
	}
	return *d.Tab, true
}

func dataOf(ctx context.Context, port plugin.Port, id string) (tabData, bool, error) {
	panel, err := port.PanelTabs(ctx)
	if err != nil {
		return tabData{}, false, plugin.RequestError(err)
	}
	for _, tab := range panel.Tabs {
		if tab.ID == id {
			data, err := decodeTabData(tab.Data)
			return data, err == nil, nil
		}
	}
	return tabData{}, false, nil
}

func (i *instance) bind(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	port plugin.Port,
	id string,
) error {
	release, err := i.enter(ctx, conversation)
	if err != nil {
		return err
	}
	defer release()
	data, found, err := dataOf(ctx, port, id)
	if err != nil || !found {
		return err
	}
	if _, live := data.live(); live && (data.Failure == nil || string(*data.Failure) == "null") {
		return nil
	}
	opened, err := openTab(ctx, data.URL, port)
	if err != nil {
		failure := panelFailure(err)
		patch, encodeErr := contract.EncodeObject([]contract.Field{{Name: "failure", Value: failure}})
		if encodeErr != nil {
			return encodeErr
		}
		_, err = port.UpdatePanelTab(ctx, id, patch)
		return err
	}
	patch, err := contract.EncodeObject(
		[]contract.Field{{Name: "tab", Value: opened.ID}, {Name: "closed", Value: nil}, {Name: "failure", Value: nil}},
	)
	if err != nil {
		return err
	}
	if _, err := port.UpdatePanelTab(ctx, id, patch); err != nil {
		return plugin.RequestError(err)
	}
	now, found, err := dataOf(ctx, port, id)
	if err != nil {
		return err
	}
	if !found {
		return closeTab(ctx, string(opened.ID), port)
	}
	if now.URL != data.URL {
		return navigate(ctx, port, string(opened.ID), now.URL)
	}
	return nil
}

func (i *instance) removed(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	port plugin.Port,
	tab webapiproto.PanelTab,
) error {
	release, err := i.enter(ctx, conversation)
	if err != nil {
		return err
	}
	defer release()
	data, err := decodeTabData(tab.Data)
	if err != nil {
		return nil
	}
	if id, live := data.live(); live {
		return closeTab(ctx, id, port)
	}
	return nil
}

func (i *instance) syncTabs(ctx context.Context, conversation webapiproto.ConversationID, port plugin.Port) error {
	release, err := i.enter(ctx, conversation)
	if err != nil {
		return err
	}
	defer release()
	listed, err := listTabs(ctx, port)
	// An unreachable Host changes no saved tab; retry after it is reachable.
	if stopped(err) {
		return nil
	}
	if err != nil {
		return err
	}
	panel, err := port.PanelTabs(ctx)
	if err != nil {
		return plugin.RequestError(err)
	}
	return reconcile(ctx, port, listed, panel.Tabs)
}

func reconcile(
	ctx context.Context,
	port plugin.Port,
	listed []browserproto.BrowserTab,
	panel []webapiproto.PanelTab,
) error {
	shown := map[string]bool{}
	present := map[string]bool{}
	for _, tab := range panel {
		if tab.Kind != panelKind {
			continue
		}
		data, err := decodeTabData(tab.Data)
		if err == nil && data.Tab != nil {
			shown[*data.Tab] = true
		}
	}
	for _, tab := range listed {
		present[string(tab.ID)] = true
		if _, user := tab.CreatedBy.(*browserproto.BrowserCreatedByUser); user || shown[string(tab.ID)] {
			continue
		}
		data, err := contract.EncodeObject(
			[]contract.Field{{Name: "url", Value: tab.URL}, {Name: "tab", Value: tab.ID}},
		)
		if err != nil {
			return err
		}
		if _, err := port.CreatePanelTab(
			ctx,
			webapiproto.CreatePanelTab{ID: "browser-" + string(tab.ID), Kind: panelKind, Data: data},
		); err != nil {
			return plugin.RequestError(err)
		}
	}
	for _, tab := range panel {
		if tab.Kind != panelKind {
			continue
		}
		data, err := decodeTabData(tab.Data)
		if err != nil {
			continue
		}
		if id, live := data.live(); live && !present[id] {
			if _, err := port.UpdatePanelTab(ctx, tab.ID, json.RawMessage(`{"closed":true}`)); err != nil {
				return plugin.RequestError(err)
			}
		}
	}
	return nil
}

// Why a browser tab could not be opened.
// +demi:root
type tabFailure struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

func panelFailure(err error) tabFailure {
	code, message := "failed", err.Error()
	var refusal *plugin.ErrorRefused
	var host *plugin.PortRefusalHost
	switch {
	case errors.As(err, &refusal):
		code, message = refusal.Reason, refusal.Message
	case errors.As(err, &host):
		code, message = string(host.Code), host.Message
	}
	return tabFailure{Code: code, Message: message}
}
