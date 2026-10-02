package hostaccess

//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// RemoteFile is a file a message names on one of the user's devices.
type RemoteFile struct {
	Device string
	// Path is absolute on the device.
	Path string
}

// ReferenceRemoteFiles checks every device before granting any reference,
// attaches non-main devices, and returns references in input order.
func ReferenceRemoteFiles(ctx context.Context, shard HostShard, id webapi.ConversationID, files []RemoteFile) ([]core.UserContentBlock, error) {
	panic("not written: b-hostaccess")
}

// ResolveUpload writes the owner's upload under the Host's attachments home,
// returning message blocks and held media. The caller already holds this Host's
// admission for the frame; this function never enters the file gate again.
func ResolveUpload(ctx context.Context, shard HostShard, id webapi.ConversationID, admitted *ConversationHost, reference, fileName string) ([]core.UserContentBlock, store.HeldMedia, error) {
	panic("not written: b-hostaccess")
}
