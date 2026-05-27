package main

import (
	"fmt"
	"time"
)

// Go has no lazy iterator adapters in std (pre-range-over-func idiom), so the
// filter/map/sum is a plain loop — which is how this is normally written in Go.
func run() int64 {
	var n, p int64 = 10000000, 1000000007
	var acc int64 = 0
	for i := int64(0); i < n; i++ {
		if i%3 == 0 {
			acc = (acc + (i*i)%p) % p
		}
	}
	return acc
}

func main() {
	run() // warmup
	t0 := time.Now()
	ans := run()
	dt := time.Since(t0).Nanoseconds()
	fmt.Println(ans)
	fmt.Println(dt)
}
