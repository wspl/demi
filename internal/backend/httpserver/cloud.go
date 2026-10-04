package httpserver

import (
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) cloudStatus(w http.ResponseWriter, r *http.Request) error {
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	status, err := cloud.Status(r.Context(), shard.CloudShard())
	if err != nil {
		return err
	}
	writeJSON(w, 200, status)
	return nil
}

func (e *Server) resetCloud(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeCloudReset)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	operation, err := cloud.Reset(r.Context(), shard.CloudShard(), request.OperationID)
	if err != nil {
		var refusal *cloud.Error
		if errors.As(err, &refusal) && refusal.Kind == cloud.AtCapacity {
			return apiFailure(409, "cloud_capacity", err.Error())
		}
		return err
	}
	writeJSON(w, 202, webapiproto.CloudResetAnswer{Operation: cloud.OperationDTO(operation)})
	return nil
}
