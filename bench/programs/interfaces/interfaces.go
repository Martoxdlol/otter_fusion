package main

import (
	"fmt"
	"time"
)

type Shape interface {
	area() int64
}

type Circle struct{ r int64 }
type Square struct{ s int64 }

func (c Circle) area() int64 { return c.r * c.r * 355 / 113 }
func (s Square) area() int64 { return s.s * s.s }

func run() int64 {
	var k, rounds, p int64 = 2000, 5000, 1000000007

	shapes := make([]Shape, 0, k)
	for i := int64(0); i < k; i++ {
		v := i%100 + 1
		if i%2 == 0 {
			shapes = append(shapes, Circle{r: v})
		} else {
			shapes = append(shapes, Square{s: v})
		}
	}

	var acc int64 = 0
	for r := int64(0); r < rounds; r++ {
		for _, sh := range shapes {
			acc = (acc + sh.area()) % p
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
