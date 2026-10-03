package edge

import (
	"context"
	"errors"
	"fmt"
	"net/http"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) devices(w http.ResponseWriter, r *http.Request) error {
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	all, err := shard.DeviceList(r.Context())
	if err != nil {
		return err
	}
	list := make([]webapi.DeviceDTO, 0, len(all))
	for _, device := range all {
		if device.Kind == webapi.DeviceKindUser {
			list = append(list, device)
		}
	}
	writeJSON(w, 200, webapi.Devices{Devices: list})
	return nil
}

func invalidCode() *apiError {
	return apiFailure(404, "invalid_code", "Unknown or expired pairing code")
}

func (e *Edge) claim(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeClaim)
	if err != nil {
		return err
	}
	if !e.state.Services.Claims.Attempt(caller(r).ID) {
		return apiFailure(429, "rate_limited", "Too many claim attempts")
	}
	code, ok := runners.ParseClaimCode(request.Code)
	if !ok {
		return invalidCode()
	}
	pending := e.state.Services.Claims.Take(code)
	if pending == nil {
		return invalidCode()
	}
	defer pending.Release()
	token := runners.NewDeviceToken()
	device, err := e.state.Services.Control.CreateDevice(
		r.Context(),
		caller(r).ID,
		pending.Runner.Name,
		pending.Runner.Platform,
		database.HashToken(token.Expose()),
	)
	if err != nil {
		return err
	}
	answer := pending.Grant(device, token)
	defer answer.Release()
	bound, err := answer.Wait(r.Context())
	if err != nil || bound == nil {
		if failed := e.state.Services.Control.DeleteDevice(
			context.WithoutCancel(r.Context()),
			device.ID,
		); failed != nil {
			return failed
		}
		return invalidCode()
	}
	writeJSON(w, 201, webapi.ClaimedDevice{Device: *bound})
	return nil
}

func (e *Edge) ownedDevice(r *http.Request, paired bool) (*database.DeviceRecord, error) {
	missing := apiFailure(404, "device_not_found", "No such device")
	id, err := webapi.ParseDeviceID(r.PathValue("id"))
	if err != nil {
		return nil, missing
	}
	device, err := e.state.Services.Control.Device(r.Context(), id)
	if err != nil {
		return nil, err
	}
	if device == nil || device.User != caller(r).ID || paired && device.Kind != webapi.DeviceKindUser {
		return nil, missing
	}
	return device, nil
}

func (e *Edge) revoke(w http.ResponseWriter, r *http.Request) error {
	device, err := e.ownedDevice(r, true)
	if err != nil {
		return err
	}
	count, err := e.state.Services.Control.WorkspacesOnDevice(r.Context(), device.ID)
	if err != nil {
		return err
	}
	if count > 0 {
		return apiFailure(409, "device_in_use", fmt.Sprintf("%d workspace(s) still point at this device", count))
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	if err := shard.RevokeDevice(r.Context(), device.ID); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}

func deviceFSError(err error) error {
	var failure *host.Error
	if !errors.As(err, &failure) {
		return err
	}
	if failure.Kind == host.Offline {
		return apiFailure(409, "device_offline", "The device's runner is not connected")
	}
	status := 400
	if failure.Code == "ENOENT" {
		status = 404
	}
	return apiFailure(status, "fs_error", failure.Message)
}

func (e *Edge) deviceDirectory(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeDeviceDirectoryQuery)
	if err != nil {
		return err
	}
	device, err := e.ownedDevice(r, true)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	h := shard.Devices().DeviceAccess(device.ID)
	if h == nil {
		return apiFailure(409, "device_offline", "The device's runner is not connected")
	}
	home, known := shard.Devices().Home(device.ID)
	var homeValue *string
	if known {
		homeValue = &home
	}
	path := home
	if query.Path != nil {
		path = string(*query.Path)
	} else if !known {
		return apiFailure(400, "invalid_query", "Missing path query parameter")
	}
	entries, err := runners.BrowseDirectory(r.Context(), h.FS(), path)
	if err != nil {
		return deviceFSError(err)
	}
	writeJSON(w, 200, webapi.Directory{Path: path, Home: homeValue, Entries: entries})
	return nil
}

func (e *Edge) deviceMkdir(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeCreateDeviceDirectory)
	if err != nil {
		return err
	}
	device, err := e.ownedDevice(r, true)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	h := shard.Devices().DeviceAccess(device.ID)
	if h == nil {
		return apiFailure(409, "device_offline", "The device's runner is not connected")
	}
	if err := h.FS().Mkdir(r.Context(), string(request.Path), host.MkdirOptions{Recursive: true}); err != nil {
		return deviceFSError(err)
	}
	writeJSON(w, 201, webapi.CreatedDirectory{Path: string(request.Path)})
	return nil
}

func (e *Edge) deviceLog(w http.ResponseWriter, r *http.Request) error {
	query, err := webapi.DecodeDeviceLogValues(r.URL.Query())
	if err != nil {
		return apiFailure(400, "invalid_query", err.Error())
	}
	device, err := e.ownedDevice(r, false)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	h := shard.Devices().DeviceAccess(device.ID)
	if h == nil {
		return apiFailure(409, "device_offline", "The device's runner is not connected")
	}
	var source *string
	if query.Source != nil {
		s := string(*query.Source)
		source = &s
	}
	page, err := h.ReadLog(r.Context(), query.Since, uint64(query.Limit), source)
	if err != nil {
		var failed *host.Error
		if errors.As(err, &failed) && failed.Kind == host.Offline {
			return apiFailure(409, "device_offline", "The device's runner is not connected")
		}
		return apiFailure(500, "log_unreadable", err.Error())
	}
	lines := make([]webapi.DeviceLogLine, 0, len(page.Lines))
	for _, line := range page.Lines {
		at, err := core.TimestampFromMillisecond(int64(line.At))
		if err != nil {
			return apiFailure(500, "log_unreadable", err.Error())
		}
		lines = append(
			lines,
			webapi.DeviceLogLine{At: at, Source: line.Source, ConversationID: line.ConversationID, Text: line.Text},
		)
	}
	writeJSON(w, 200, webapi.DeviceLog{Lines: lines, Next: page.Next})
	return nil
}
