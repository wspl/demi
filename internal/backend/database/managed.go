package database

import (
	"context"
	"database/sql"
	"fmt"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/webapi"
)

// ManagedOperation returns the device's reset `id`, as it was last written.
func (c *ControlService) ManagedOperation(ctx context.Context, device webapi.DeviceID, id webapi.OperationID) (*ManagedOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*ManagedOperation, error) {
		return queryRecord(ctx, tx, "managed_operations", "SELECT * FROM managed_operations WHERE device_id = ? AND operation_id = ?", operationRow, device, id)
	})
}

// LatestManagedOperation returns the device's reset written last: the one its status shows.
func (c *ControlService) LatestManagedOperation(ctx context.Context, device webapi.DeviceID) (*ManagedOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*ManagedOperation, error) {
		return queryRecord(ctx, tx, "managed_operations", "SELECT * FROM managed_operations WHERE device_id = ? ORDER BY updated_at DESC,rowid DESC LIMIT 1", operationRow, device)
	})
}

// PutManagedOperation writes the operation's phase and error, creating its row on its
// first write.
func (c *ControlService) PutManagedOperation(ctx context.Context, device webapi.DeviceID, operation ManagedOperation) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) error {
		at, err := now.Millisecond()
		if err != nil {
			return err
		}
		return execSQL(ctx, tx, `INSERT INTO managed_operations (device_id,operation_id,base_version,phase,error,updated_at) VALUES (?,?,?,?,?,?) ON CONFLICT (device_id,operation_id) DO UPDATE SET phase=excluded.phase,error=excluded.error,updated_at=excluded.updated_at`, device, operation.ID, operation.BaseVersion, operation.Phase, operation.Error, at)
	})
}

// UnfinishedManagedOperations returns every reset left between its admission and its end, with its
// device: what a backend that stopped in the middle of one finishes
// when it starts.
func (c *ControlService) UnfinishedManagedOperations(ctx context.Context) ([]DeviceOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]DeviceOperation, error) {
		return queryRecords(ctx, tx, "managed_operations", "SELECT * FROM managed_operations WHERE phase NOT IN ('ready','failed') ORDER BY updated_at,rowid", func(r *storedRow) DeviceOperation {
			return DeviceOperation{Device: checked(r, "device_id", webapi.ParseDeviceID), Operation: operationRow(r)}
		})
	})
}

// RotateDeviceToken replaces the token of the user's Cloud device with the one a boot
// minted: the token of an earlier boot opens no connection any more.
func (c *ControlService) RotateDeviceToken(ctx context.Context, device webapi.DeviceID, token TokenHash) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		changed, err := affected(ctx, tx, "UPDATE devices SET token_hash = ? WHERE id = ? AND kind = 'managed'", token.Text(), device)
		if err != nil {
			return err
		}
		if !changed {
			return &Error{Kind: Corrupt, Table: "devices", Column: "id", Reason: fmt.Sprintf("device %s is not a Cloud device", device)}
		}
		return nil
	})
}

// AnnounceCloudReset tells every conversation of the user that its Cloud was reset by
// `operation`: each one's execution context advances once per reset,
// so every node reads the reset in its next context block.
func (c *ControlService) AnnounceCloudReset(ctx context.Context, user webapi.UserID, operation webapi.OperationID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return execSQL(ctx, tx, "UPDATE conversations SET context_version=context_version+1,cloud_reset_id=?2 WHERE user_id=?1 AND (cloud_reset_id IS NULL OR cloud_reset_id<>?2)", user, operation)
	})
}

// AnnouncedCloudReset returns the Cloud reset the conversation was last told of.
func (c *ControlService) AnnouncedCloudReset(ctx context.Context, id webapi.ConversationID) (*webapi.OperationID, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*webapi.OperationID, error) {
		r, err := queryRecord(ctx, tx, "conversations", "SELECT cloud_reset_id FROM conversations WHERE id = ?", func(r *storedRow) *webapi.OperationID {
			return optionalChecked(r, "cloud_reset_id", webapi.ParseOperationID)
		}, id)
		if r == nil {
			return nil, err
		}
		return *r, err
	})
}

// CloudUses returns the user's conversations that are not archived, as the Cloud's
// lifecycle weighs them; `cloud` is the user's Cloud device, once its
// first use made it.
func (c *ControlService) CloudUses(ctx context.Context, user webapi.UserID, cloud *webapi.DeviceID) ([]CloudUseRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]CloudUseRecord, error) {
		return queryRecords(ctx, tx, "conversations", `SELECT c.id,c.target_kind,c.target_device_id,w.device_id AS workspace_device,c.model,EXISTS (SELECT 1 FROM conversation_hosts h WHERE h.conversation_id=c.id AND h.device_id=?2) AS attached FROM conversations c LEFT JOIN workspaces w ON w.id=c.target_workspace_id WHERE c.user_id=?1 AND c.archived=0 ORDER BY c.id`, func(r *storedRow) CloudUseRecord {
			device := r.optionalText("target_device_id")
			if device == nil {
				device = r.optionalText("workspace_device")
			}
			u := CloudUseRecord{ID: checked(r, "id", webapi.ParseConversationID), OnCloud: r.text("target_kind") == "cloud" || (cloud != nil && device != nil && *device == string(*cloud)), Attached: r.boolean("attached")}
			if model := optionalJSON(r, "model", core.DecodeModelSelection); model != nil {
				provider, err := webapi.ParseProviderID(model.ProviderID)
				r.bad("model", err)
				u.Provider = &provider
			}
			return u
		}, user, cloud)
	})
}

func operationRow(r *storedRow) ManagedOperation {
	o := ManagedOperation{ID: checked(r, "operation_id", webapi.ParseOperationID), BaseVersion: checked(r, "base_version", machinewire.ParseBaseVersion), Phase: webapi.ResetPhase(r.text("phase")), Error: r.optionalText("error")}
	r.bad("phase", o.Phase.Validate())
	return o
}
