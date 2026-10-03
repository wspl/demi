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
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
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
func ReferenceRemoteFiles(ctx context.Context, shard HostShard, id webapi.ConversationID, files []RemoteFile) ([]core.UserContentBlock, error) {
	record, err := OwnedConversation(ctx, shard, id)
	if err != nil {
		var access *Error
		if errors.As(err, &access) && access.Kind == AccessStorage {
			return nil, &RemoteFileRefusal{Kind: RemoteFileStorage, Cause: access.Cause}
		}
		return nil, &RemoteFileRefusal{Kind: RemoteFileNotAccessible}
	}
	devices := make(map[string]database.DeviceRecord)
	ordered := make([]database.DeviceRecord, 0)
	for _, file := range files {
		if _, ok := devices[file.Device]; ok {
			continue
		}
		deviceID, err := webapi.ParseDeviceID(file.Device)
		if err != nil {
			return nil, &RemoteFileRefusal{Kind: RemoteFileNotAccessible}
		}
		device, err := shard.Control().Device(ctx, deviceID)
		if err != nil {
			return nil, &RemoteFileRefusal{Kind: RemoteFileStorage, Cause: err}
		}
		if device == nil || device.User != record.Owner {
			return nil, &RemoteFileRefusal{Kind: RemoteFileNotAccessible}
		}
		if !shard.Devices().Online(device.ID) {
			return nil, &RemoteFileRefusal{Kind: RemoteFileOffline, Message: device.Name}
		}
		devices[file.Device] = *device
		ordered = append(ordered, *device)
	}
	references := make([]core.UserContentBlock, 0, len(files))
	for _, file := range files {
		device := devices[file.Device]
		reference, err := remoteReference(device, file.Path)
		if err != nil {
			return nil, err
		}
		references = append(references, reference)
	}
	target, err := ResolveTarget(ctx, shard, record)
	if err != nil {
		return nil, &RemoteFileRefusal{Kind: RemoteFileStorage, Cause: err}
	}
	main := database.ExecutionDeviceID(target)
	for _, device := range ordered {
		if main != nil && device.ID == *main {
			continue
		}
		outcome, err := shard.Control().ChangeConversation(ctx, record.ID, &database.RecordAttach{Host: database.AttachedHostRecord{Device: device.ID, Name: device.Name}})
		if err != nil {
			return nil, &RemoteFileRefusal{Kind: RemoteFileStorage, Cause: err}
		}
		if outcome != database.ChangeApplied {
			return nil, &RemoteFileRefusal{Kind: RemoteFileNotAccessible}
		}
	}
	return references, nil
}

// ResolveUpload writes the owner's upload under the Host's attachments home,
// returning message blocks and held media. The caller already holds this Host's
// admission for the frame; this function never enters the file gate again.
func ResolveUpload(ctx context.Context, shard HostShard, id webapi.ConversationID, admitted *ConversationHost, reference, fileName string) ([]core.UserContentBlock, store.HeldMedia, error) {
	unavailable := []core.UserContentBlock{store.Unavailable(reference)}
	idUpload, err := webapi.ParseAttachmentID(reference)
	if err != nil {
		return unavailable, store.HeldMedia{}, nil
	}
	record, err := shard.Control().Attachment(ctx, idUpload)
	if err != nil {
		return nil, store.HeldMedia{}, &Error{Kind: AccessStorage, Cause: err}
	}
	if record == nil || record.Owner != shard.User() {
		return unavailable, store.HeldMedia{}, nil
	}
	bytes, exists, err := shard.Blobs().Read(ctx, record.SHA256)
	if err != nil {
		return nil, store.HeldMedia{}, &Error{Kind: AccessObjects, Cause: err}
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
	if err := admitted.Host.FS().WriteFile(ctx, written, host.FileContents{Bytes: bytes}, host.WriteOptions{CreateParents: true}); err != nil {
		return nil, store.HeldMedia{}, accessError(err)
	}
	blocks, held, err := store.UploadBlocks(ctx, store.Upload{Name: name, Path: written, MediaType: record.MediaType, SHA256: record.SHA256, Bytes: bytes}, shard.Blobs())
	if err != nil {
		return nil, store.HeldMedia{}, &Error{Kind: AccessStore, Cause: err}
	}
	return blocks, held, nil
}

// remoteReference names a device file and a shell command that reads its exact path.
func remoteReference(device database.DeviceRecord, path string) (core.UserContentBlock, error) {
	// The runner's shell accepts Bash quoting, including control characters that
	// syntax.Quote deliberately cannot represent in its POSIX mode.
	quotedPath, err := syntax.Quote(path, syntax.LangBash)
	if err != nil {
		return nil, &RemoteFileRefusal{Kind: RemoteFileUnquotable}
	}
	quotedRead, err := syntax.Quote("cat -- "+quotedPath, syntax.LangBash)
	if err != nil {
		return nil, &RemoteFileRefusal{Kind: RemoteFileUnquotable}
	}
	quotedID, err := syntax.Quote(string(device.ID), syntax.LangBash)
	if err != nil {
		return nil, &RemoteFileRefusal{Kind: RemoteFileUnquotable}
	}
	command := "demi host shell --host " + quotedID + " " + quotedRead
	reference := url.URL{Scheme: "file", Path: path}
	reference.RawQuery = "host=" + url.QueryEscape(device.Name) + "&deviceId=" + url.QueryEscape(string(device.ID)) + "&readCommand=" + url.QueryEscape(command)
	return &core.UserReference{Reference: reference.String()}, nil
}
