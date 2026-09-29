package core

import (
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/wspl/demi/go/internal/wire"
)

// Timestamp stores whole milliseconds since the Unix epoch.
//
//demi:opaque string format=date-time
//wiregen:browser inline {"type":"string"}
type Timestamp struct{ millisecond int64 }

var UnixEpoch = Timestamp{}

// TimestampFromMillisecond accepts the range supported by jiff::Timestamp.
func TimestampFromMillisecond(ms int64) (Timestamp, error) {
	if ms < -377705023201000 || ms > 253402207200999 {
		return Timestamp{}, errors.New("outside the supported range of times")
	}
	return Timestamp{millisecond: ms}, nil
}

// TruncateTimestamp removes precision finer than milliseconds, toward zero.
func TruncateTimestamp(t time.Time) Timestamp {
	ns := int64(t.Nanosecond())
	ms := t.Unix()*1000 + ns/1000000
	if t.Unix() < 0 && ns%1000000 != 0 {
		ms++
	}
	return Timestamp{millisecond: ms}
}

// timestampText constrains time.Parse's deliberately permissive RFC 3339
// parser. Expanded negative years are how jiff writes its supported BCE dates.
var timestampText = regexp.MustCompile(`^([0-9]{4}|[+-][0-9]{6})-([0-9]{2}-[0-9]{2})[Tt ]([0-9]{2}:[0-9]{2}:[0-9]{2})([.,][0-9]{1,9})?([Zz]|[+-][0-9]{2}:[0-9]{2})$`)

func ParseTimestamp(text string) (Timestamp, error) {
	parts := timestampText.FindStringSubmatch(text)
	if parts == nil {
		return Timestamp{}, errors.New("not an RFC 3339 time")
	}
	year, err := strconv.Atoi(parts[1])
	if err != nil || year < -9999 || year > 9999 || parts[1] == "-000000" {
		return Timestamp{}, errors.New("not an RFC 3339 time")
	}
	zone := strings.ToUpper(parts[5])
	if zone != "Z" {
		hour, _ := strconv.Atoi(zone[1:3]) // The pattern establishes decimal digits.
		minute, _ := strconv.Atoi(zone[4:6])
		if hour > 23 || minute > 59 {
			return Timestamp{}, errors.New("not an RFC 3339 time")
		}
	}
	// Map BCE years into the same Gregorian 400-year cycle for time.Parse,
	// whose year grammar only accepts four unsigned digits.
	parseYear := year
	if year < 0 {
		parseYear = 2000 + (year%400+400)%400
	}
	clock := parts[3]
	// Jiff follows Temporal in constraining a leap second to second 59.
	if clock[6:] == "60" {
		clock = clock[:6] + "59"
	}
	normalized := fmt.Sprintf("%04d-%sT%s%s%s", parseYear, parts[2], clock, parts[4], zone)
	t, err := time.Parse(time.RFC3339Nano, normalized)
	if err != nil {
		return Timestamp{}, errors.New("not an RFC 3339 time")
	}
	if t.Nanosecond()%1000000 != 0 {
		return Timestamp{}, errors.New("a time is whole milliseconds")
	}
	if year != parseYear {
		t = time.Date(year, t.Month(), t.Day(), t.Hour(), t.Minute(), t.Second(), t.Nanosecond(), t.Location())
	}
	return TimestampFromMillisecond(t.UnixMilli())
}
func (t Timestamp) Millisecond() int64 { return t.millisecond }
func (t Timestamp) Time() time.Time    { return time.UnixMilli(t.millisecond).UTC() }
func (t Timestamp) String() string {
	value := t.Time()
	if value.Year() < 0 {
		return fmt.Sprintf("-%06d%s", -value.Year(), value.Format("-01-02T15:04:05.000Z"))
	}
	return value.Format("2006-01-02T15:04:05.000Z")
}
func (t Timestamp) validate() error { _, err := TimestampFromMillisecond(t.millisecond); return err }
func (t Timestamp) MarshalJSONTo(enc *jsontext.Encoder) error {
	if err := t.validate(); err != nil {
		return err
	}
	return enc.WriteToken(jsontext.String(t.String()))
}
func (t *Timestamp) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	parsed, err := ParseTimestamp(text)
	if err != nil {
		return &InvalidError{Rule: err.Error()}
	}
	*t = parsed
	return nil
}

// Clock supplies the wall time records and answers carry.
type Clock interface{ Now() Timestamp }
type SystemClock struct{}

func (SystemClock) Now() Timestamp { return TruncateTimestamp(time.Now()) }
