package main

import (
	"fmt"
	"time"
)

// Sum-type via interface + type switch (Go's idiom for discriminated unions).
type Expr interface{ isExpr() }

type Num struct{ v int64 }
type Add struct{ l, r Expr }
type Mul struct{ l, r Expr }

func (Num) isExpr() {}
func (Add) isExpr() {}
func (Mul) isExpr() {}

func evaluate(e Expr) int64 {
	switch x := e.(type) {
	case Num:
		return x.v
	case Add:
		return evaluate(x.l) + evaluate(x.r)
	case Mul:
		return evaluate(x.l) * evaluate(x.r)
	}
	return 0
}

func run() int64 {
	var count, p int64 = 2000000, 1000000007
	var acc int64 = 0
	for i := int64(0); i < count; i++ {
		var expr Expr = Add{
			l: Num{v: i % 1000},
			r: Mul{l: Num{v: i % 7}, r: Num{v: 3}},
		}
		acc = (acc + evaluate(expr)) % p
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
