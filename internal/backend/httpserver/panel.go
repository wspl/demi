package httpserver

import (
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) panel(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	if r.Method == "GET" || r.Method == "HEAD" {
		panel, err := e.state.Services.Control.Panel(r.Context(), record.ID)
		if err != nil {
			return err
		}
		writeJSON(w, 200, panel)
		return nil
	}
	if record.Archived {
		return apiFailure(409, "conversation_archived", "The conversation is archived")
	}
	change, err := panelChange(r)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	revision, err := shard.Plugins().ChangePanel(r.Context(), record.ID, change)
	if err != nil {
		return panelError(err)
	}
	writeJSON(w, 200, webapiproto.PanelRevision{Revision: revision})
	return nil
}

func panelChange(r *http.Request) (database.PanelChange, error) {
	id := r.PathValue("tab")
	switch r.Method {
	case "PATCH":
		body, err := decodeBody(r, webapiproto.DecodeUpdatePanelTab)
		return database.PanelUpdate{ID: id, Data: body.Data}, err
	case "DELETE":
		return database.PanelRemove{ID: id}, nil
	default:
		if id != "" {
			body, err := decodeBody(r, webapiproto.DecodeMovePanelTab)
			return database.PanelMove{ID: id, Index: body.Index}, err
		}
		body, err := decodeBody(r, webapiproto.DecodeCreatePanelTab)
		return database.PanelCreate{Tab: body}, err
	}
}

func panelError(err error) error {
	var refusal *plugin.PortRefusalPanel
	if !errors.As(err, &refusal) {
		return err
	}
	status := 409
	switch refusal.Code {
	case webapiproto.ErrorCodeUnknownPanelKind:
		status = 400
	case webapiproto.ErrorCodeTooLarge:
		status = 413
	case webapiproto.ErrorCodeConversationNotFound:
		status = 404
	}
	return apiFailure(status, refusal.Code, refusal.Message)
}
