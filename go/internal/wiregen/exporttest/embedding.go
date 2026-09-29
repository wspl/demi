package exporttest

import "errors"

//demi:wire
//demi:export
//demi:msgpack
type Interval struct {
	Low  int `json:"low" check:"range=1.."`
	High int `json:"high"`
}

func (v Interval) check() error {
	if v.Low > v.High {
		return errors.New("low exceeds high")
	}
	return nil
}

type Intervals = map[string]Interval
type IntervalList []Interval
