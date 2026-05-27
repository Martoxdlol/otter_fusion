package main

import (
	"fmt"
	"time"
)

func fib(n int64) int64 {
	if n < 2 {
		return n
	}
	return fib(n-1) + fib(n-2)
}

func run() int64 {
	return fib(35)
}

func main() {
	run() // warmup
	t0 := time.Now()
	ans := run()
	dt := time.Since(t0).Nanoseconds()
	fmt.Println(ans)
	fmt.Println(dt)
}
