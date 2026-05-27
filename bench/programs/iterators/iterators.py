import time


def run():
    n = 10000000
    p = 1000000007
    acc = 0
    # Lazy generator expression: filter + map, summed.
    for i in (x for x in range(n) if x % 3 == 0):
        acc = (acc + (i * i) % p) % p
    return acc


run()  # warmup
t0 = time.perf_counter_ns()
ans = run()
dt = time.perf_counter_ns() - t0
print(ans)
print(dt)
