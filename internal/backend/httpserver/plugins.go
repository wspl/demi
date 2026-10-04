package httpserver

import (
	"encoding/json"
	"net/http"

	"github.com/wspl/demi/internal/backend/pluginhost"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) switchPlugin(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodePluginSwitch)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	if err := shard.SwitchPlugin(r.Context(), r.PathValue("plugin"), request.Enabled); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}

func (e *Server) pageCall(w http.ResponseWriter, r *http.Request) error {
	data, err := readJSONBody(r)
	if err != nil {
		return err
	}
	if len(data) == 0 {
		data = []byte("{}")
	}
	params, err := contract.Decode[json.RawMessage](data)
	if err != nil {
		return invalidBody(err)
	}
	call := pluginhost.PageCall{Plugin: r.PathValue("plugin"), Method: r.PathValue("method"), Params: params}
	if r.PathValue("id") != "" {
		record, err := e.owned(r)
		if err != nil {
			return err
		}
		call.Conversation = &record.ID
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	result, err := shard.Plugins().PageCall(r.Context(), call)
	if err != nil {
		return err
	}
	writeJSON(w, 200, result)
	return nil
}

func (e *Server) pluginState(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	result, err := shard.Plugins().ConversationState(r.Context(), r.PathValue("plugin"), record.ID)
	if err != nil {
		return err
	}
	writeJSON(w, 200, result)
	return nil
}

func (e *Server) reload(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	if err := shard.ReloadConversation(r.Context(), record.ID); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}
