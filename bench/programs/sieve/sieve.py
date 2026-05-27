import time


def run():
    n = 2000000
    sieve = bytearray([1]) * n
    count = 0
    for p in range(2, n):
        if sieve[p]:
            count += 1
            for m in range(p * 2, n, p):
                sieve[m] = 0
    return count


run()  # warmup
t0 = time.perf_counter_ns()
ans = run()
dt = time.perf_counter_ns() - t0
print(ans)
print(dt)
