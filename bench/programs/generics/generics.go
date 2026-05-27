package main

import (
	"fmt"
	"time"
)

type Wrapper[T any] struct {
	value T
}

func (w Wrapper[T]) get() T { return w.value }

func run() int64 {
	var n, p int64 = 10000000, 1000000007
	var acc int64 = 0
	for i := int64(0); i < n; i++ {
		w := Wrapper[int64]{value: i}
		acc = (acc + w.get()*2) % p
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
