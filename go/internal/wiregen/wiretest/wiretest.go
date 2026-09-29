// Package wiretest declares wire types that between them use every shape and
// every rule of the wire contracts, so that the generator's decoders and checks
// can be run in tests. Nothing else imports it.
package wiretest

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/jsontext"
	"errors"
	"regexp"
	"strings"
)

// Limit is a constant that a rule names.
const Limit = 3

// lowercase is the pattern of the field that has one.
var lowercase = regexp.MustCompile(`^[a-z]+$`)

// A Mode is a closed set of strings under its own name.
type Mode string

// Rules has one field for each rule.
//
//demi:wire
type Rules struct {
	Chars    string            `json:"chars" check:"chars=2..4"`
	AtLeast  string            `json:"atLeast" check:"chars=2.."`
	AtMost   string            `json:"atMost" check:"chars=..Limit"`
	Bytes    string            `json:"bytes" check:"bytes=1..3"`
	Items    []string          `json:"items" check:"items=1..2"`
	Number   int8              `json:"number" check:"range=1..10"`
	Big      uint64            `json:"big" check:"range=..Limit"`
	Version  uint64            `json:"version" check:"eq=7"`
	Word     string            `json:"word" check:"eq=hello"`
	Kind     string            `json:"kind" check:"oneof=a|b"`
	Mode     Mode              `json:"mode" check:"oneof=fast|slow"`
	Token    string            `json:"token" check:"pattern=lowercase"`
	Plain    string            `json:"plain" check:"nonul"`
	Unique   []string          `json:"unique" check:"unique"`
	Checked  string            `json:"checked" check:"func=noSpaces"`
	Each     []string          `json:"each" check:"each(chars=1..2)"`
	Table    map[string]string `json:"table" check:"keys(chars=1..3),each(nonul)"`
	Modes    map[Mode]int      `json:"modes" check:"keys(oneof=fast|slow)"`
	Optional *string           `json:"optional,omitzero" check:"chars=1..2"`
	Object   jsontext.Value    `json:"object" check:"func=isObject"`
}

// noSpaces is the rule of a field that holds no space.
func noSpaces(value string) error {
	if strings.Contains(value, " ") {
		return errors.New("contains a space")
	}
	return nil
}

// isObject is the rule of a field that holds a JSON object.
func isObject(value jsontext.Value) error {
	if value.Kind() != '{' {
		return errors.New("must be an object")
	}
	return nil
}

// Shapes has one field for each shape of a field's type.
//
//demi:wire
type Shapes struct {
	Text     string          `json:"text"`
	Flag     bool            `json:"flag"`
	Small    uint8           `json:"small"`
	Signed   int64           `json:"signed"`
	Native   int             `json:"native"`
	Names    []string        `json:"names"`
	Nested   [][]int         `json:"nested"`
	Leaf     Leaf            `json:"leaf"`
	Leaves   []Leaf          `json:"leaves"`
	ByName   map[string]Leaf `json:"byName"`
	Maybe    *Leaf           `json:"maybe,omitzero"`
	MaybeNum *int            `json:"maybeNum,omitzero"`
	Shape    Shape           `json:"shape"`
	Shapes   []Shape         `json:"shapes"`
	Loose    Loose           `json:"loose"`
	Span     Span            `json:"span"`
}

// A Leaf is the value of a nested field.
//
//demi:wire
type Leaf struct {
	Name string `json:"name" check:"chars=1.."`
}

// A Shape is a circle or a square, told apart by its kind.
//
//demi:union tag=kind
type Shape interface {
	shape()
}

// A Circle has a radius.
//
//demi:variant circle
type Circle struct {
	Radius uint8 `json:"radius" check:"range=1.."`
}

// A Square has a side, and may have a label.
//
//demi:variant square
type Square struct {
	Side  uint8   `json:"side"`
	Label *string `json:"label,omitzero" check:"chars=1.."`
}

func (Circle) shape() {}
func (Square) shape() {}

// Loose is a union told apart by its members: a number or a name.
//
//demi:union untagged
type Loose interface {
	loose()
}

// A Numbered has a number.
//
//demi:variant
type Numbered struct {
	Number uint8 `json:"number"`
}

// A Named has a name.
//
//demi:variant
type Named struct {
	Name string `json:"name" check:"chars=1.."`
}

func (Numbered) loose() {}
func (Named) loose()    {}

// Vague is a union told apart by its members that cannot tell its variants
// apart: an empty object fits both.
//
//demi:union untagged
type Vague interface {
	vague()
}

// A Yes may hold a flag.
//
//demi:variant
type Yes struct {
	OK *bool `json:"ok,omitzero"`
}

// A No may hold a flag.
//
//demi:variant
type No struct {
	OK *bool `json:"ok,omitzero"`
}

func (Yes) vague() {}
func (No) vague()  {}

// A Span is a range of numbers with a rule across its fields.
//
//demi:wire
type Span struct {
	Low  int `json:"low"`
	High int `json:"high"`
}

func (s Span) check() error {
	if s.Low > s.High {
		return errors.New("low is above high")
	}
	return nil
}

// An Empty has no field.
//
//demi:wire
type Empty struct{}

// A Level is a closed set of strings: the constants of its type.
//
//demi:enum
//demi:describe How loud.
type Level string

// The values of a [Level].
const (
	LevelLow  Level = "low"
	LevelHigh Level = "high"
)

// A Code is a string with the rules of its type: three lowercase letters.
//
//demi:value
//demi:check pattern=lowercase,chars=3..3
type Code string

// A Header is what a [Message] starts with.
//
//demi:wire
type Header struct {
	ID string `json:"id" check:"chars=1.."`
}

// A Message embeds a struct: its members are the Header's and its own, as
// serde's flatten makes them.
//
//demi:wire
type Message struct {
	Header
	Body string `json:"body"`
}

// A Tolerant ignores the members it does not have.
//
//demi:wire open
type Tolerant struct {
	Kept string `json:"kept"`
}

// A Reading is a temperature or a label: a JSON number or a JSON string.
//
//demi:union untagged
//demi:describe A temperature or a label.
type Reading interface {
	reading()
}

// A Celsius is a temperature.
//
//demi:variant
type Celsius float64

// A Label is a name.
//
//demi:variant
type Label string

func (Celsius) reading() {}
func (Label) reading()   {}

// Extras has one field for each thing the wire has beyond the rest: a float, a
// closed set, a value with rules, a member that may be null, a union of
// scalars, an embedded struct and an open one.
//
//demi:wire
type Extras struct {
	Ratio    float64  `json:"ratio" check:"range=0.5..4"`
	Level    Level    `json:"level"`
	Levels   []Level  `json:"levels"`
	Code     Code     `json:"code"`
	Note     *string  `json:"note" check:"nullable,chars=1..3"`
	Reading  Reading  `json:"reading"`
	Message  Message  `json:"message"`
	Tolerant Tolerant `json:"tolerant"`
}
