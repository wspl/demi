package invalid

// +demi:root
// +demi:union tag=ok
type Broken interface{ broken() }

// +demi:variant true
type Yes struct{}

func (*Yes) broken() {}

// +demi:variant no
type No struct{}

func (*No) broken() {}
