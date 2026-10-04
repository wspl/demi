package plugin

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/webapi"
)

// PanelTabs returns the conversation's tabs of this plugin's kinds.
func (p Port) PanelTabs(ctx context.Context) (webapi.WorkPanel, error) {
	answer, err := p.ask(ctx, &PortMessagePanelTabs{})
	if err != nil {
		return webapi.WorkPanel{}, err
	}
	if a, ok := answer.(*PortAnswerPanel); ok {
		return a.Panel, nil
	}
	return webapi.WorkPanel{}, unexpected("panel_tabs", answer)
}

// CreatePanelTab creates a tab and returns the panel revision.
func (p Port) CreatePanelTab(ctx context.Context, tab webapi.CreatePanelTab) (uint64, error) {
	return p.panelChange(ctx, "create_panel_tab", &PortMessageCreatePanelTab{Tab: tab})
}

// UpdatePanelTab changes data fields and returns the panel revision.
func (p Port) UpdatePanelTab(ctx context.Context, id string, data json.RawMessage) (uint64, error) {
	return p.panelChange(ctx, "update_panel_tab", &PortMessageUpdatePanelTab{ID: id, Data: data})
}

// RemovePanelTab removes a tab and returns the panel revision.
func (p Port) RemovePanelTab(ctx context.Context, id string) (uint64, error) {
	return p.panelChange(ctx, "remove_panel_tab", &PortMessageRemovePanelTab{ID: id})
}

func (p Port) panelChange(ctx context.Context, asked string, message PortMessage) (uint64, error) {
	answer, err := p.ask(ctx, message)
	if err != nil {
		return 0, err
	}
	if a, ok := answer.(*PortAnswerPanelRevision); ok {
		return a.Revision, nil
	}
	return 0, unexpected(asked, answer)
}

func validatePanelUpdate(message PortMessageUpdatePanelTab) error {
	return (webapi.UpdatePanelTab{Data: message.Data}).Validate()
}
