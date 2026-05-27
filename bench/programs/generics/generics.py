import time


class Wrapper:
    __slots__ = ("value",)

    def __init__(self, value):
        self.value = value

    def get(self):
        return self.value


def run():
    n = 10000000
    p = 1000000007
    acc = 0
    for i in range(n):
        w = Wrapper(i)
        acc = (acc + w.get() * 2) % p
    return acc


run()  # warmup
t0 = time.perf_counter_ns()
ans = run()
dt = time.perf_counter_ns() - t0
print(ans)
print(dt)
