import time


class Circle:
    __slots__ = ("r",)

    def __init__(self, r):
        self.r = r

    def area(self):
        return self.r * self.r * 355 // 113


class Square:
    __slots__ = ("s",)

    def __init__(self, s):
        self.s = s

    def area(self):
        return self.s * self.s


def run():
    k, rounds, p = 2000, 5000, 1000000007
    shapes = []
    for i in range(k):
        v = i % 100 + 1
        shapes.append(Circle(v) if i % 2 == 0 else Square(v))
    acc = 0
    for _ in range(rounds):
        for sh in shapes:
            acc = (acc + sh.area()) % p
    return acc


run()  # warmup
t0 = time.perf_counter_ns()
ans = run()
dt = time.perf_counter_ns() - t0
print(ans)
print(dt)
