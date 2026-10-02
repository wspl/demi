package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// WorkspaceRecord is a `workspaces` row.
type WorkspaceRecord struct {
	ID        webapi.WorkspaceID
	User      webapi.UserID
	Device    webapi.DeviceID
	Path      string
	Name      string
	CreatedAt core.Timestamp
}
