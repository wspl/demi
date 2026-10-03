package plugintest

import (
	"cmp"
	"fmt"
	"math"
	"slices"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// LiveExposes removes expired exposes and returns the rest in expiry order.
func (d *TestDemi) LiveExposes() []plugin.ExposeRecord {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.liveExposesLocked()
}

// liveExposesLocked applies the test clock while mu is held.
func (d *TestDemi) liveExposesLocked() []plugin.ExposeRecord {
	d.exposes = slices.DeleteFunc(d.exposes, func(e plugin.ExposeRecord) bool { return e.ExpiresAt <= d.Now })
	slices.SortStableFunc(
		d.exposes,
		func(a, b plugin.ExposeRecord) int { return cmp.Compare(a.ExpiresAt, b.ExpiresAt) },
	)
	return append([]plugin.ExposeRecord{}, d.exposes...)
}

// EndExposesOn ends every expose of the device, as a Cloud stop does.
func (d *TestDemi) EndExposesOn(device webapi.DeviceID) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.exposes = slices.DeleteFunc(d.exposes, func(e plugin.ExposeRecord) bool { return e.Device == device })
}

// createExposeLocked answers a creation request using the test clock and Hosts.
func (d *TestDemi) createExposeLocked(m *plugin.PortMessageCreateExpose) (plugin.PortAnswer, error) {
	if !d.ExposesAvailable {
		return exposeRefused(plugin.ExposeRefusalUnavailable, "no expose domain"), nil
	}
	address, err := webapi.ParseExposeAddress(m.Address)
	if err != nil {
		return exposeRefused(plugin.ExposeRefusalInvalidAddress, err.Error()), nil
	}
	index := slices.IndexFunc(d.Hosts, func(h plugin.ConversationHost) bool { return h.Device == m.Device })
	if index < 0 {
		return exposeRefused(plugin.ExposeRefusalDeviceNotFound, "No such device"), nil
	}
	h := d.Hosts[index]
	if !h.Online {
		return exposeRefused(plugin.ExposeRefusalDeviceOffline, "offline"), nil
	}
	expiry, err := d.exposeExpiryLocked(m.Lifetime)
	if err != nil {
		return nil, err
	}
	d.exposesMade++
	id, err := testExposeID(d.exposesMade)
	if err != nil {
		return nil, err
	}
	record := plugin.ExposeRecord{
		ID: id, Device: m.Device, DeviceName: h.Name, Address: address,
		URL: "http://" + string(id) + ".expose.localhost:3271/", CreatedAt: d.Now, ExpiresAt: expiry,
	}
	d.exposes = append(d.exposes, record)
	return &plugin.PortAnswerExpose{Expose: record}, nil
}

// exposeExpiryLocked computes an expiry from the test clock while mu is held.
func (d *TestDemi) exposeExpiryLocked(seconds uint64) (core.Timestamp, error) {
	now, err := d.Now.Millisecond()
	if err != nil {
		return "", err
	}
	if seconds > math.MaxInt64/1000 {
		return "", fmt.Errorf("a test lifetime fits in milliseconds")
	}
	millis := int64(seconds) * 1000
	if now > math.MaxInt64-millis {
		return "", fmt.Errorf("a test expiry fits in milliseconds")
	}
	return core.TimestampFromMillisecond(now + millis)
}

// exposeRefused puts an expose refusal on the plugin wire.
func exposeRefused(reason plugin.ExposeRefusal, message string) plugin.PortAnswer {
	return refused(&plugin.PortRefusalExpose{Reason: reason, Message: message})
}

// testExposeID draws the nth test expose id as 26 base32 characters.
func testExposeID(n uint64) (webapi.ExposeID, error) {
	const alphabet = "abcdefghijklmnopqrstuvwxyz234567"
	text := []byte("aaaaaaaaaaaaaaaaaaaaaaaaaa")
	for i := len(text) - 1; i >= 0; i-- {
		text[i] = alphabet[n%32]
		n /= 32
	}
	return webapi.ParseExposeID(string(text))
}
