package browserop

import "slices"

// clone detaches a browser locator from its original optional field storage.
func (q BrowserQueryMatch) clone() BrowserQueryMatch {
	if q.Ref != nil {
		q.Ref = new(*q.Ref)
	}
	if q.Role != nil {
		q.Role = new(*q.Role)
	}
	if q.Name != nil {
		q.Name = new(*q.Name)
	}
	if q.NamePattern != nil {
		q.NamePattern = new(*q.NamePattern)
	}
	if q.TextPattern != nil {
		q.TextPattern = new(*q.TextPattern)
	}
	if q.Label != nil {
		q.Label = new(*q.Label)
	}
	if q.Placeholder != nil {
		q.Placeholder = new(*q.Placeholder)
	}
	if q.TextMatch != nil {
		q.TextMatch = new(*q.TextMatch)
	}
	if q.TestID != nil {
		q.TestID = new(*q.TestID)
	}
	if q.CSS != nil {
		q.CSS = new(*q.CSS)
	}
	if q.Exact != nil {
		q.Exact = new(*q.Exact)
	}
	return q
}

// clone detaches the locator and scope of a browser element target.
func (t BrowserTarget) clone() BrowserTarget {
	t.BrowserQueryMatch = t.BrowserQueryMatch.clone()
	if t.Frame != nil {
		t.Frame = new(slices.Clone(*t.Frame))
	}
	if t.Nth != nil {
		t.Nth = new(*t.Nth)
	}
	if t.Within != nil {
		t.Within = new(*t.Within)
	}
	return t
}

// clone detaches a query tree, including nested locators and branch slices.
func (q BrowserQuery) clone() BrowserQuery {
	if q.Match != nil {
		q.Match = new(q.Match.clone())
	}
	if q.Within != nil {
		q.Within = new(q.Within.clone())
	}
	if q.Frame != nil {
		q.Frame = new(q.Frame.clone())
	}
	if q.Has != nil {
		q.Has = new(q.Has.clone())
	}
	if q.HasNot != nil {
		q.HasNot = new(q.HasNot.clone())
	}
	if q.And != nil {
		children := slices.Clone(*q.And)
		for i := range children {
			children[i] = children[i].clone()
		}
		q.And = &children
	}
	if q.Or != nil {
		children := slices.Clone(*q.Or)
		for i := range children {
			children[i] = children[i].clone()
		}
		q.Or = &children
	}
	if q.HasText != nil {
		q.HasText = new(*q.HasText)
	}
	if q.HasNotText != nil {
		q.HasNotText = new(*q.HasNotText)
	}
	if q.Visible != nil {
		q.Visible = new(*q.Visible)
	}
	if q.Nth != nil {
		q.Nth = new(*q.Nth)
	}
	return q
}
