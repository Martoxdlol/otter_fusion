import time


class Num:
    __slots__ = ("v",)

    def __init__(self, v):
        self.v = v


class Add:
    __slots__ = ("l", "r")

    def __init__(self, l, r):
        self.l = l
        self.r = r


class Mul:
    __slots__ = ("l", "r")

    def __init__(self, l, r):
        self.l = l
        self.r = r


def evaluate(e):
    if type(e) is Num:
        return e.v
    if type(e) is Add:
        return evaluate(e.l) + evaluate(e.r)
    return evaluate(e.l) * evaluate(e.r)


def run():
    count = 2000000
    p = 1000000007
    acc = 0
    for i in range(count):
        expr = Add(Num(i % 1000), Mul(Num(i % 7), Num(3)))
        acc = (acc + evaluate(expr)) % p
    return acc


run()  # warmup
t0 = time.perf_counter_ns()
ans = run()
dt = time.perf_counter_ns() - t0
print(ans)
print(dt)
