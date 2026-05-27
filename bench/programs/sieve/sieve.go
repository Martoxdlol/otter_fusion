package main

import (
	"fmt"
	"time"
)

func run() int64 {
	n := 2000000
	sieve := make([]bool, n)
	for i := range sieve {
		sieve[i] = true
	}
	var count int64 = 0
	for p := 2; p < n; p++ {
		if sieve[p] {
			count++
			for m := p * 2; m < n; m += p {
				sieve[m] = false
			}
		}
	}
	return count
}

func main() {
	run() // warmup
	t0 := time.Now()
	ans := run()
	dt := time.Since(t0).Nanoseconds()
	fmt.Println(ans)
	fmt.Println(dt)
}
