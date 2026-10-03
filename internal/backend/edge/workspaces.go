package edge

import (
	"fmt"
	"net/http"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) workspaces(w http.ResponseWriter, r *http.Request) error {
	records, err := e.state.Services.Control.Workspaces(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	items := make([]webapi.WorkspaceDTO, 0, len(records))
	for _, record := range records {
		items = append(items, record.DTO())
	}
	writeJSON(w, 200, webapi.Workspaces{Workspaces: items})
	return nil
}
func (e *Edge) createWorkspace(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeCreateWorkspace)
	if err != nil {
		return err
	}
	var record *database.WorkspaceRecord
	switch request := request.(type) {
	case *webapi.CreateWorkspaceDevice:
		record, err = e.state.Services.Control.CreateWorkspace(r.Context(), database.NewWorkspaceID(), caller(r).ID, request.DeviceID, string(request.Path), string(request.Name))
	case *webapi.CreateWorkspaceCloud:
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
	writeJSON(w, 201, webapi.WorkspaceAnswer{Workspace: record.DTO()})
	return nil
}
func (e *Edge) renameWorkspace(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeRenameWorkspace)
	if err != nil {
		return err
	}
	id, err := webapi.ParseWorkspaceID(r.PathValue("id"))
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
	writeJSON(w, 200, webapi.WorkspaceAnswer{Workspace: record.DTO()})
	return nil
}
func (e *Edge) deleteWorkspace(w http.ResponseWriter, r *http.Request) error {
	id, err := webapi.ParseWorkspaceID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "workspace_not_found", "No such workspace")
	}
	result, err := e.state.Services.Control.DeleteWorkspace(r.Context(), caller(r).ID, id)
	if err != nil {
		return err
	}
	switch result := result.(type) {
	case *database.WorkspaceDeleted:
		e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Workspaces})
		w.WriteHeader(204)
	case *database.WorkspaceMissing:
		return apiFailure(404, "workspace_not_found", "No such workspace")
	case *database.WorkspaceInUse:
		return apiFailure(409, "workspace_in_use", fmt.Sprintf("%d conversation(s) still target this workspace", result.Count))
	}
	return nil
}
