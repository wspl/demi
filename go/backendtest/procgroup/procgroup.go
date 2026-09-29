// Package procgroup starts and ends the suite's process groups. Linux also
// kills children when their launcher dies and supports adopting orphans.
// Other Unix systems support explicit group cleanup only. Windows refuses
// to start processes because this harness requires Unix process groups.
package procgroup
