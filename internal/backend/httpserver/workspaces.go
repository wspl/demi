package httpserver

import (
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) workspaces(w http.ResponseWriter, r *http.Request) error {
	records, err := e.state.Services.Control.Workspaces(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	items := make([]webapiproto.WorkspaceDTO, 0, len(records))
	for _, record := range records {
		items = append(items, record.DTO())
	}
	writeJSON(w, 200, webapiproto.Workspaces{Workspaces: items})
	return nil
}

func (e *Server) createWorkspace(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeCreateWorkspace)
	if err != nil {
		return err
	}
	var record *database.WorkspaceRecord
	switch request := request.(type) {
	case *webapiproto.CreateWorkspaceDevice:
		record, err = e.state.Services.Control.CreateWorkspace(
			r.Context(),
			database.NewWorkspaceID(),
			caller(r).ID,
			request.DeviceID,
			string(request.Path),
			string(request.Name),
		)
	case *webapiproto.CreateWorkspaceCloud:
		shard, failed := e.state.Shards.Of(r.Context(), caller(r).ID)
		if failed != nil {
			return failed
		}
		created, failed := shard.CreateCloudWorkspace(r.Context(), string(request.Name))
		record = &created
		err = failed
	}
	if err != nil {
		return err
	}
	if record == nil {
		return apiFailure(404, "device_not_found", "No such device")
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Workspaces})
	writeJSON(w, 201, webapiproto.WorkspaceAnswer{Workspace: record.DTO()})
	return nil
}

func (e *Server) renameWorkspace(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeRenameWorkspace)
	if err != nil {
		return err
	}
	id, err := webapiproto.ParseWorkspaceID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "workspace_not_found", "No such workspace")
	}
	record, err := e.state.Services.Control.RenameWorkspace(r.Context(), caller(r).ID, id, string(request.Name))
	if err != nil {
		return err
	}
	if record == nil {
		return apiFailure(404, "workspace_not_found", "No such workspace")
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Workspaces})
	writeJSON(w, 200, webapiproto.WorkspaceAnswer{Workspace: record.DTO()})
	return nil
}

func (e *Server) deleteWorkspace(w http.ResponseWriter, r *http.Request) error {
	id, err := webapiproto.ParseWorkspaceID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "workspace_not_found", "No such workspace")
	}
	err = e.state.Services.Control.DeleteWorkspace(r.Context(), caller(r).ID, id)
	var inUse *database.WorkspaceInUseError
	if errors.Is(err, database.ErrWorkspaceNotFound) {
		return apiFailure(404, "workspace_not_found", "No such workspace")
	}
	if errors.As(err, &inUse) {
		return apiFailure(409, "workspace_in_use", inUse.Error())
	}
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Workspaces})
	w.WriteHeader(204)
	return nil
}
