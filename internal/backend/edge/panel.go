package edge

import (
	"fmt"
	"net/http"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) panel(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	if r.Method == "GET" || r.Method == "HEAD" {
		panel, err := e.state.Services.Control.Panel(r.Context(), record.ID)
		if err != nil {
			return err
		}
		if panel == nil {
			empty := webapi.EmptyWorkPanel()
			panel = &empty
		}
		writeJSON(w, 200, panel)
		return nil
	}
	if record.Archived {
		return apiFailure(409, "conversation_archived", "The conversation is archived")
	}
	panel, err := decodeBody(r, webapi.DecodeWorkPanel)
	if err != nil {
		return err
	}
	document, err := contract.EncodeJSON(panel)
	if err != nil {
		return err
	}
	if len(document) > webapi.PanelBytesMax {
		return apiFailure(413, "too_large", fmt.Sprintf("The work panel is over its %d-byte limit", webapi.PanelBytesMax))
	}
	if err := e.state.Services.Control.SavePanel(r.Context(), record.ID, string(document)); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}
