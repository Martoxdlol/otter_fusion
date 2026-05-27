import sys
import time

sys.setrecursionlimit(10000)


def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)


def run():
    return fib(35)


run()  # warmup
t0 = time.perf_counter_ns()
ans = run()
dt = time.perf_counter_ns() - t0
print(ans)
print(dt)
