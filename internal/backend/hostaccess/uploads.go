package hostaccess

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"path"
	"strings"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
	"mvdan.cc/sh/v3/syntax"
)

// RemoteFile is a file a message names on one of the user's devices.
type RemoteFile struct {
	Device string
	// Path is absolute on the device.
	Path string
}

// ReferenceRemoteFiles checks every device before granting any reference,
// attaches non-main devices, and returns references in input order.
func ReferenceRemoteFiles(
	ctx context.Context,
	shard HostShard,
	id webapiproto.ConversationID,
	files []RemoteFile,
) ([]types.UserContentBlock, error) {
	record, err := OwnedConversation(ctx, shard, id)
	if err != nil {
		var access *Error
		if errors.As(err, &access) && access.Kind == AccessStorage {
			return nil, access.Cause
		}
		return nil, ErrDeviceNotAccessible
	}
	devices := make(map[string]database.DeviceRecord)
	ordered := make([]database.DeviceRecord, 0)
	for _, file := range files {
		if _, ok := devices[file.Device]; ok {
			continue
		}
		deviceID, err := webapiproto.ParseDeviceID(file.Device)
		if err != nil {
			return nil, ErrDeviceNotAccessible
		}
		device, found, err := shard.Control().Device(ctx, deviceID)
		if err != nil {
			return nil, err
		}
		if !found || device.User != record.Owner {
			return nil, ErrDeviceNotAccessible
		}
		if !shard.Devices().Online(device.ID) {
			//nolint:staticcheck // ST1005: product text, shown to the user as it is.
			return nil, fmt.Errorf("Referenced device %s is offline", device.Name)
		}
		devices[file.Device] = device
		ordered = append(ordered, device)
	}
	references := make([]types.UserContentBlock, 0, len(files))
	for _, file := range files {
		device := devices[file.Device]
		reference, err := remoteReference(device, file.Path)
		if err != nil {
			return nil, err
		}
		references = append(references, reference)
	}
	if err := attachRemoteDevices(ctx, shard, record, ordered); err != nil {
		return nil, err
	}
	return references, nil
}

// ResolveUpload writes the owner's upload under the Host's attachments home,
// returning message blocks and held media. The caller already holds this Host's
// admission for the frame; this function never enters the file gate again.
func ResolveUpload(
	ctx context.Context,
	shard HostShard,
	id webapiproto.ConversationID,
	admitted *ConversationHost,
	reference, fileName string,
) ([]types.UserContentBlock, store.HeldMedia, error) {
	unavailable := []types.UserContentBlock{store.Unavailable(reference)}
	idUpload, err := webapiproto.ParseAttachmentID(reference)
	if err != nil {
		return unavailable, store.HeldMedia{}, nil
	}
	record, found, err := shard.Control().Attachment(ctx, idUpload)
	if err != nil {
		return nil, store.HeldMedia{}, &Error{Kind: AccessStorage, Cause: err}
	}
	if !found || record.Owner != shard.User() {
		return unavailable, store.HeldMedia{}, nil
	}
	bytes, exists, err := shard.Blobs().Read(ctx, record.SHA256)
	if err != nil {
		return nil, store.HeldMedia{}, &Error{Kind: AccessStorage, Cause: err}
	}
	if !exists {
		return unavailable, store.HeldMedia{}, nil
	}
	home := admitted.Host.Identity().HomeDir
	directory := strings.TrimRight(home, "/") + "/.demi/attachments/" + string(id)
	name := fileName
	for number := 2; ; number++ {
		exists, err := admitted.Host.FS().Exists(ctx, directory+"/"+name)
		if err != nil {
			return nil, store.HeldMedia{}, accessError(err)
		}
		if !exists {
			break
		}
		extension := path.Ext(fileName)
		stem := strings.TrimSuffix(fileName, extension)
		if stem == "" || extension == "" {
			name = fmt.Sprintf("%s-%d", fileName, number)
		} else {
			name = fmt.Sprintf("%s-%d%s", stem, number, extension)
		}
	}
	written := directory + "/" + name
	if err := admitted.Host.FS().
		WriteFile(ctx, written, host.FileContents{Bytes: bytes}, host.WriteOptions{CreateParents: true}); err != nil {
		return nil, store.HeldMedia{}, accessError(err)
	}
	blocks, held, err := store.UploadBlocks(
		ctx,
		store.Upload{Name: name, Path: written, MediaType: record.MediaType, SHA256: record.SHA256, Bytes: bytes},
		shard.Blobs(),
	)
	if err != nil {
		return nil, store.HeldMedia{}, &Error{Kind: AccessStorage, Cause: err}
	}
	return blocks, held, nil
}

// remoteReference names a device file and a shell command that reads its exact path.
func remoteReference(device database.DeviceRecord, path string) (types.UserContentBlock, error) {
	// The runner's shell accepts Bash quoting, including control characters that
	// syntax.Quote deliberately cannot represent in its POSIX mode.
	quotedPath, err := syntax.Quote(path, syntax.LangBash)
	if err != nil {
		return nil, ErrPathUnquotable
	}
	quotedRead, err := syntax.Quote("cat -- "+quotedPath, syntax.LangBash)
	if err != nil {
		return nil, ErrPathUnquotable
	}
	quotedID, err := syntax.Quote(string(device.ID), syntax.LangBash)
	if err != nil {
		return nil, ErrPathUnquotable
	}
	command := "demi host shell --host " + quotedID + " " + quotedRead
	reference := url.URL{Scheme: "file", Path: path}
	reference.RawQuery = "host=" + url.QueryEscape(
		device.Name,
	) + "&deviceId=" + url.QueryEscape(
		string(device.ID),
	) + "&readCommand=" + url.QueryEscape(
		command,
	)
	return &types.UserReference{Reference: reference.String()}, nil
}

func attachRemoteDevices(
	ctx context.Context,
	shard HostShard,
	record database.ConversationRecord,
	ordered []database.DeviceRecord,
) error {
	target, err := ResolveTarget(ctx, shard, record)
	if err != nil {
		return err
	}
	main, hasMain := database.ExecutionDeviceID(target)
	for _, device := range ordered {
		if hasMain && device.ID == main {
			continue
		}
		err := shard.Control().
			ChangeConversation(
				ctx, record.ID,
				&database.RecordAttach{Host: database.AttachedHostRecord{Device: device.ID, Name: device.Name}},
			)
		if errors.Is(err, database.ErrConversationNotFound) || errors.Is(err, database.ErrArchived) {
			return ErrDeviceNotAccessible
		}
		if err != nil {
			return err
		}
	}
	return nil
}
