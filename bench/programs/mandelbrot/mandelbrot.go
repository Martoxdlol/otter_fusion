package main

import (
	"fmt"
	"time"
)

func run() int64 {
	var w, h, maxiter int64 = 800, 800, 255
	var checksum int64 = 0

	// Go may contract `a*b + c` into a single FMA, which rounds differently
	// from the other languages' separately-rounded mul-then-add (and the SSA
	// pass re-fuses even across statements). A `float64(...)` conversion on the
	// product is the only reliable barrier: it forces the multiply to round to
	// double before the add, matching everyone else's checksum.
	for py := int64(0); py < h; py++ {
		for px := int64(0); px < w; px++ {
			cre := -2.0 + float64(float64(px)*(2.5/float64(w)))
			cim := -1.25 + float64(float64(py)*(2.5/float64(h)))
			var zr, zi, zr2, zi2 float64 = 0, 0, 0, 0
			var i int64 = 0
			for zr2+zi2 <= 4.0 {
				if i >= maxiter {
					break
				}
				zi = float64(2.0*zr*zi) + cim
				zr = zr2 - zi2 + cre
				zr2 = zr * zr
				zi2 = zi * zi
				i++
			}
			checksum += i
		}
	}
	return checksum
}

func main() {
	run() // warmup
	t0 := time.Now()
	ans := run()
	dt := time.Since(t0).Nanoseconds()
	fmt.Println(ans)
	fmt.Println(dt)
}
