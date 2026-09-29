package backendtest

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"math"
	"sort"
	"testing"
	"time"

	"github.com/coder/websocket"
)

// RunnerVersion is the runner wire's version, which a runner's hello names.
const RunnerVersion = 24

// A RawRunner is a runner's side of the runner socket (runner.md § Connection
// and identity), spoken by the scenario: MessagePack frames of loose maps, whose
// "type" names the message. Go has no runner wire of its own until the runner
// moves (plan.md G5), so the suite carries the little of MessagePack it needs.
type RawRunner struct {
	t    testing.TB
	conn *websocket.Conn
}

// ConnectRawRunner opens a runner socket to the backend, which the runner
// program does with its hello.
func (b *Backend) ConnectRawRunner() *RawRunner {
	b.t.Helper()
	conn, status, body, err := b.dial(nil, "/api/runner", "")
	if conn == nil {
		b.t.Fatalf("the runner socket refused the upgrade with %d: %v %s", status, err, body)
	}
	b.t.Cleanup(func() {
		_ = conn.CloseNow()
	})
	return &RawRunner{t: b.t, conn: conn}
}

// Send sends a message as a binary frame of MessagePack.
func (r *RawRunner) Send(message map[string]any) {
	r.t.Helper()
	if err := r.TrySend(message); err != nil {
		r.t.Fatal(err)
	}
}

// TrySend sends a message as Send does and answers an error instead of failing
// the test, for a goroutine of the scenario.
func (r *RawRunner) TrySend(message map[string]any) error {
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	if err := r.conn.Write(ctx, websocket.MessageBinary, encodeMessagePack(message)); err != nil {
		return fmt.Errorf("the runner socket does not take a frame: %w", err)
	}
	return nil
}

// SendBinary sends bytes as a binary frame.
func (r *RawRunner) SendBinary(frame []byte) {
	r.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	if err := r.conn.Write(ctx, websocket.MessageBinary, frame); err != nil {
		r.t.Fatalf("the runner socket does not take a frame: %v", err)
	}
}

// SendText sends a text message, which a runner never does.
func (r *RawRunner) SendText(text string) {
	r.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	if err := r.conn.Write(ctx, websocket.MessageText, []byte(text)); err != nil {
		r.t.Fatalf("the runner socket does not take a message: %v", err)
	}
}

// Next reads the next message the backend sends; nil once it closed the socket.
func (r *RawRunner) Next() map[string]any {
	r.t.Helper()
	message, err := r.TryNext()
	if err != nil {
		r.t.Fatal(err)
	}
	return message
}

// TryNext reads as Next does and answers an error instead of failing the test,
// for a goroutine of the scenario.
func (r *RawRunner) TryNext() (map[string]any, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	kind, frame, err := r.conn.Read(ctx)
	for err == nil && kind != websocket.MessageBinary {
		kind, frame, err = r.conn.Read(ctx)
	}
	if err != nil {
		if ctx.Err() != nil {
			return nil, errors.New("the backend does not answer the runner socket within ten seconds")
		}
		return nil, nil
	}
	decoded, err := decodeMessagePack(frame)
	if err != nil {
		return nil, fmt.Errorf("the backend sent a frame that is no MessagePack: %w", err)
	}
	message, ok := decoded.(map[string]any)
	if !ok {
		return nil, fmt.Errorf("the backend sent %v, not a message", decoded)
	}
	return message, nil
}

// Close closes the socket from the runner's side.
func (r *RawRunner) Close() {
	// A socket the backend closed already has nothing to close.
	_ = r.conn.Close(websocket.StatusNormalClosure, "")
}

// RunnerHello is the hello of a runner named "raw" that speaks the protocol,
// with the device token when it has one and the managed flag when it says one.
func RunnerHello(protocol int, token string, managed *bool) map[string]any {
	runner := map[string]any{
		"name": "raw", "platform": "linux", "version": "0",
		"identity": map[string]any{"uid": 1, "gid": 1, "hostname": "raw", "homeDir": "/home/raw"},
	}
	if managed != nil {
		runner["managed"] = *managed
	}
	hello := map[string]any{"type": "hello", "protocol": protocol, "runner": runner}
	if token != "" {
		hello["deviceToken"] = token
	}
	return hello
}

// encodeMessagePack encodes nil, booleans, integers, floats, strings, lists and
// maps with string keys, each in its smallest form, as the runner wire's encoder
// does.
func encodeMessagePack(value any) []byte {
	var out []byte
	switch value := value.(type) {
	case nil:
		out = append(out, 0xc0)
	case bool:
		if value {
			out = append(out, 0xc3)
		} else {
			out = append(out, 0xc2)
		}
	case int:
		out = appendInteger(out, int64(value))
	case uint32:
		out = appendInteger(out, int64(value))
	case int64:
		out = appendInteger(out, value)
	case float64:
		out = append(out, 0xcb)
		out = binary.BigEndian.AppendUint64(out, math.Float64bits(value))
	case string:
		out = appendLength(out, len(value), 0xa0, 31, 0xd9, 0xda, 0xdb)
		out = append(out, value...)
	case []any:
		out = appendLength(out, len(value), 0x90, 15, 0, 0xdc, 0xdd)
		for _, item := range value {
			out = append(out, encodeMessagePack(item)...)
		}
	case map[string]any:
		out = appendLength(out, len(value), 0x80, 15, 0, 0xde, 0xdf)
		keys := make([]string, 0, len(value))
		for key := range value {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		for _, key := range keys {
			out = append(out, encodeMessagePack(key)...)
			out = append(out, encodeMessagePack(value[key])...)
		}
	default:
		panic(fmt.Sprintf("no MessagePack of %T", value))
	}
	return out
}

// appendLength writes the header of a string, list or map of length n: a fixed
// form up to fixMax, else the 8-bit form (when there is one), the 16-bit or the
// 32-bit.
func appendLength(out []byte, n int, fix byte, fixMax int, eight, sixteen, thirtyTwo byte) []byte {
	switch {
	case n <= fixMax:
		return append(out, fix|byte(n))
	case eight != 0 && n < 1<<8:
		return append(out, eight, byte(n))
	case n < 1<<16:
		return binary.BigEndian.AppendUint16(append(out, sixteen), uint16(n))
	default:
		return binary.BigEndian.AppendUint32(append(out, thirtyTwo), uint32(n))
	}
}

func appendInteger(out []byte, value int64) []byte {
	switch {
	case value >= 0 && value < 128:
		return append(out, byte(value))
	case value < 0 && value >= -32:
		return append(out, byte(value))
	case value >= 0 && value < 1<<8:
		return append(out, 0xcc, byte(value))
	case value >= 0 && value < 1<<16:
		return binary.BigEndian.AppendUint16(append(out, 0xcd), uint16(value))
	case value >= 0 && value < 1<<32:
		return binary.BigEndian.AppendUint32(append(out, 0xce), uint32(value))
	case value >= 0:
		return binary.BigEndian.AppendUint64(append(out, 0xcf), uint64(value))
	default:
		return binary.BigEndian.AppendUint64(append(out, 0xd3), uint64(value))
	}
}

// decodeMessagePack decodes one MessagePack document into nil, booleans,
// float64 (every number), strings, byte slices, lists and maps with string keys;
// bytes after it are an error.
func decodeMessagePack(data []byte) (any, error) {
	value, rest, err := decodeMessagePackValue(data)
	if err != nil {
		return nil, err
	}
	if len(rest) != 0 {
		return nil, errors.New("trailing MessagePack data")
	}
	return value, nil
}

var errShort = errors.New("MessagePack ends early")

func decodeMessagePackValue(data []byte) (any, []byte, error) {
	if len(data) == 0 {
		return nil, nil, errShort
	}
	tag, data := data[0], data[1:]
	take := func(n int) ([]byte, []byte, error) {
		if len(data) < n {
			return nil, nil, errShort
		}
		return data[:n], data[n:], nil
	}
	unsigned := func(n int) (uint64, []byte, error) {
		bytes, rest, err := take(n)
		if err != nil {
			return 0, nil, err
		}
		var value uint64
		for _, b := range bytes {
			value = value<<8 | uint64(b)
		}
		return value, rest, nil
	}
	collection := func(n int, entries int, isMap bool, rest []byte) (any, []byte, error) {
		if isMap {
			result := make(map[string]any, entries)
			for range entries {
				var key, value any
				var err error
				if key, rest, err = decodeMessagePackValue(rest); err != nil {
					return nil, nil, err
				}
				name, ok := key.(string)
				if !ok {
					return nil, nil, fmt.Errorf("a map key is %T", key)
				}
				if value, rest, err = decodeMessagePackValue(rest); err != nil {
					return nil, nil, err
				}
				result[name] = value
			}
			return result, rest, nil
		}
		list := make([]any, 0, entries)
		for range entries {
			var value any
			var err error
			if value, rest, err = decodeMessagePackValue(rest); err != nil {
				return nil, nil, err
			}
			list = append(list, value)
		}
		return list, rest, nil
	}
	text := func(n int) (any, []byte, error) {
		bytes, rest, err := take(n)
		if err != nil {
			return nil, nil, err
		}
		return string(bytes), rest, nil
	}
	switch {
	case tag < 0x80:
		return float64(tag), data, nil
	case tag >= 0xe0:
		return float64(int8(tag)), data, nil
	case tag&0xf0 == 0x80:
		return collection(0, int(tag&0x0f), true, data)
	case tag&0xf0 == 0x90:
		return collection(0, int(tag&0x0f), false, data)
	case tag&0xe0 == 0xa0:
		return text(int(tag & 0x1f))
	}
	switch tag {
	case 0xc0:
		return nil, data, nil
	case 0xc2:
		return false, data, nil
	case 0xc3:
		return true, data, nil
	case 0xcc, 0xcd, 0xce, 0xcf:
		value, rest, err := unsigned(1 << (tag - 0xcc))
		return float64(value), rest, err
	case 0xd0, 0xd1, 0xd2, 0xd3:
		size := 1 << (tag - 0xd0)
		value, rest, err := unsigned(size)
		shift := 64 - 8*size
		return float64(int64(value<<shift) >> shift), rest, err
	case 0xca:
		value, rest, err := unsigned(4)
		return float64(math.Float32frombits(uint32(value))), rest, err
	case 0xcb:
		value, rest, err := unsigned(8)
		return math.Float64frombits(value), rest, err
	case 0xd9, 0xda, 0xdb:
		length, rest, err := unsigned(1 << (tag - 0xd9))
		if err != nil {
			return nil, nil, err
		}
		data = rest
		return text(int(length))
	case 0xc4, 0xc5, 0xc6:
		length, rest, err := unsigned(1 << (tag - 0xc4))
		if err != nil {
			return nil, nil, err
		}
		bytes, rest, err := func() ([]byte, []byte, error) {
			if uint64(len(rest)) < length {
				return nil, nil, errShort
			}
			return rest[:length], rest[length:], nil
		}()
		return append([]byte(nil), bytes...), rest, err
	case 0xdc, 0xdd, 0xde, 0xdf:
		size := 2
		if tag&1 == 1 {
			size = 4
		}
		entries, rest, err := unsigned(size)
		if err != nil {
			return nil, nil, err
		}
		return collection(0, int(entries), tag >= 0xde, rest)
	}
	return nil, nil, fmt.Errorf("MessagePack type %#x is not supported", tag)
}
