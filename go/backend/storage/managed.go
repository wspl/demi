package storage

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/machinesproto"
	"github.com/wspl/demi/go/webapi"
)

type ManagedOperation struct {
	ID          webapi.OperationID
	BaseVersion machinesproto.BaseVersion
	Phase       webapi.ResetPhase
	Error       *string
}
type DeviceOperation struct {
	Device    webapi.DeviceID
	Operation ManagedOperation
}
type CloudUseRecord struct {
	ID       webapi.ConversationID
	OnCloud  bool
	Provider *webapi.ProviderID
	Attached bool
}

const operationColumns = "operation_id,base_version,phase,error"

func scanManaged(row scanner, withDevice bool) (*DeviceOperation, error) {
	var r DeviceOperation
	var id, base, phase, device string
	dest := []any{&id, &base, &phase, &r.Operation.Error}
	if withDevice {
		dest = append([]any{&device}, dest...)
	}
	err := row.Scan(dest...)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.Operation.ID, err = webapi.ParseOperationID(id)
	if err != nil {
		return nil, corrupt("managed_operations", "operation_id", err)
	}
	r.Operation.BaseVersion, err = machinesproto.ParseBaseVersion(base)
	if err != nil {
		return nil, corrupt("managed_operations", "base_version", err)
	}
	r.Operation.Phase = webapi.ResetPhase(phase)
	if err = webapi.ValidateResetPhase(r.Operation.Phase); err != nil {
		return nil, corrupt("managed_operations", "phase", err)
	}
	if withDevice {
		r.Device, err = webapi.ParseDeviceID(device)
		if err != nil {
			return nil, corrupt("managed_operations", "device_id", err)
		}
	}
	return &r, nil
}
func (c *Control) ManagedOperation(ctx context.Context, device webapi.DeviceID, id webapi.OperationID) (*ManagedOperation, error) {
	r, err := scanManaged(c.db.QueryRowContext(ctx, "SELECT "+operationColumns+" FROM managed_operations WHERE device_id=? AND operation_id=?", device.String(), id.String()), false)
	if err != nil || r == nil {
		return nil, err
	}
	return &r.Operation, nil
}
func (c *Control) LatestManagedOperation(ctx context.Context, device webapi.DeviceID) (*ManagedOperation, error) {
	r, err := scanManaged(c.db.QueryRowContext(ctx, "SELECT "+operationColumns+" FROM managed_operations WHERE device_id=? ORDER BY updated_at DESC,rowid DESC LIMIT 1", device.String()), false)
	if err != nil || r == nil {
		return nil, err
	}
	return &r.Operation, nil
}
func (c *Control) PutManagedOperation(ctx context.Context, device webapi.DeviceID, operation ManagedOperation) error {
	_, err := c.db.ExecContext(ctx, "INSERT INTO managed_operations(device_id,operation_id,base_version,phase,error,updated_at) VALUES (?,?,?,?,?,?) ON CONFLICT(device_id,operation_id) DO UPDATE SET phase=excluded.phase,error=excluded.error,updated_at=excluded.updated_at", device.String(), operation.ID.String(), string(operation.BaseVersion), string(operation.Phase), operation.Error, c.clock.Now().Millisecond())
	return sqliteError(err)
}
func (c *Control) UnfinishedManagedOperations(ctx context.Context) ([]DeviceOperation, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT device_id,"+operationColumns+" FROM managed_operations WHERE phase NOT IN ('ready','failed') ORDER BY updated_at,rowid")
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	operations := []DeviceOperation{}
	for rows.Next() {
		r, err := scanManaged(rows, true)
		if err != nil {
			return nil, err
		}
		operations = append(operations, *r)
	}
	return operations, sqliteError(rows.Err())
}
func (c *Control) RotateDeviceToken(ctx context.Context, device webapi.DeviceID, token TokenHash) error {
	result, err := c.db.ExecContext(ctx, "UPDATE devices SET token_hash=? WHERE id=? AND kind='managed'", token.Text(), device.String())
	if err != nil {
		return sqliteError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return sqliteError(err)
	}
	if count == 0 {
		return &CorruptError{"devices", "id", fmt.Sprintf("device %s is not a Cloud device", device.String())}
	}
	return nil
}
func (c *Control) AnnounceCloudReset(ctx context.Context, user webapi.UserID, operation webapi.OperationID) error {
	_, err := c.db.ExecContext(ctx, "UPDATE conversations SET context_version=context_version+1,cloud_reset_id=?2 WHERE user_id=?1 AND (cloud_reset_id IS NULL OR cloud_reset_id<>?2)", user.String(), operation.String())
	return sqliteError(err)
}
func (c *Control) AnnouncedCloudReset(ctx context.Context, id webapi.ConversationID) (*webapi.OperationID, error) {
	var text *string
	err := c.db.QueryRowContext(ctx, "SELECT cloud_reset_id FROM conversations WHERE id=?", id.String()).Scan(&text)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	if text == nil {
		return nil, nil
	}
	operation, err := webapi.ParseOperationID(*text)
	return &operation, corrupt("conversations", "cloud_reset_id", err)
}
func (c *Control) CloudUses(ctx context.Context, user webapi.UserID, cloud *webapi.DeviceID) ([]CloudUseRecord, error) {
	var device *string
	if cloud != nil {
		text := cloud.String()
		device = &text
	}
	rows, err := c.db.QueryContext(ctx, `SELECT c.id,c.target_kind,c.target_device_id,w.device_id,c.model,
 EXISTS(SELECT 1 FROM conversation_hosts h WHERE h.conversation_id=c.id AND h.device_id=?2)
 FROM conversations c LEFT JOIN workspaces w ON w.id=c.target_workspace_id WHERE c.user_id=?1 AND c.archived=0 ORDER BY c.id`, user.String(), device)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	uses := []CloudUseRecord{}
	for rows.Next() {
		var r CloudUseRecord
		var id, kind string
		var target, workspace, model *string
		if err = rows.Scan(&id, &kind, &target, &workspace, &model, &r.Attached); err != nil {
			return nil, sqliteError(err)
		}
		r.ID, err = webapi.ParseConversationID(id)
		if err != nil {
			return nil, corrupt("conversations", "id", err)
		}
		if target == nil {
			target = workspace
		}
		r.OnCloud = kind == "cloud" || (device != nil && target != nil && *device == *target)
		if model != nil {
			selection, err := core.Decode[core.ModelSelection]([]byte(*model))
			if err != nil {
				return nil, corrupt("conversations", "model", err)
			}
			provider, err := webapi.ParseProviderID(selection.ProviderID)
			if err != nil {
				return nil, corrupt("conversations", "model", err)
			}
			r.Provider = &provider
		}
		uses = append(uses, r)
	}
	return uses, sqliteError(rows.Err())
}
