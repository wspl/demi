// Package cdp owns bounded Chrome transport, session routing, debugging commands
// and the low-level operations shared by browser tabs and page actions.
//
// Connections and subscriptions have explicit owners. Close cancels and joins
// their work. Callers own tab admission, tab state and conversation lifetimes.
package cdp
