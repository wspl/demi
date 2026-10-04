package httpserver

import (
	"net/http"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) hosts(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	listed, err := e.state.Services.Control.AttachedHostListing(r.Context(), record.ID)
	if err != nil {
		return err
	}
	hosts := make([]webapiproto.AttachedHost, 0, len(listed))
	for _, item := range listed {
		hosts = append(
			hosts,
			webapiproto.AttachedHost{
				DeviceID:   item.Host.Device,
				Name:       item.Host.Name,
				Cwd:        item.Host.CWD,
				AttachedAt: item.At,
				Online:     shard.Devices().Online(item.Host.Device),
			},
		)
	}
	writeJSON(w, 200, webapiproto.AttachedHosts{Hosts: hosts})
	return nil
}

func (e *Server) changeHost(w http.ResponseWriter, r *http.Request) error {
	id, err := webapiproto.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "conversation_not_found", "No such conversation")
	}
	var change database.RecordChange
	switch r.Method {
	case "POST":
		request, err := decodeBody(r, webapiproto.DecodeAttachHost)
		if err != nil {
			return err
		}
		device, found, err := e.state.Services.Control.Device(r.Context(), request.DeviceID)
		if err != nil {
			return err
		}
		if !found || device.User != caller(r).ID {
			return apiFailure(404, "device_not_found", "No such device")
		}
		change = &database.RecordAttach{Host: database.AttachedHostRecord{Device: device.ID, Name: device.Name}}
	case "PATCH":
		request, err := decodeBody(r, webapiproto.DecodeRenameHost)
		if err != nil {
			return err
		}
		device, err := webapiproto.ParseDeviceID(r.PathValue("device"))
		if err != nil {
			return apiFailure(404, "host_not_attached", "No such attached host")
		}
		change = &database.RecordRename{Device: device, Name: string(request.Name)}
	case "DELETE":
		device, err := webapiproto.ParseDeviceID(r.PathValue("device"))
		if err != nil {
			w.WriteHeader(204)
			return nil
		}
		change = &database.RecordDetach{Device: device}
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	if err := shard.Transition(r.Context(), id, &database.ConversationRecordChange{Change: change}); err != nil {
		return err
	}
	if r.Method == "DELETE" {
		w.WriteHeader(204)
		return nil
	}
	// The list is read after the transition; only an attach changes its status.
	if r.Method == "POST" {
		return e.hosts(&statusResponse{ResponseWriter: w, status: 201}, r)
	}
	return e.hosts(w, r)
}

type statusResponse struct {
	http.ResponseWriter
	status int
}

// WriteHeader substitutes the successful status selected by the host change.
func (r *statusResponse) WriteHeader(status int) {
	if status == 200 {
		status = r.status
	}
	r.ResponseWriter.WriteHeader(status)
}
