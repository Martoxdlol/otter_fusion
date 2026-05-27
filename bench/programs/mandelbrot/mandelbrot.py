import time


def run():
    w = 800
    h = 800
    maxiter = 255
    checksum = 0
    for py in range(h):
        for px in range(w):
            cre = -2.0 + px * (2.5 / w)
            cim = -1.25 + py * (2.5 / h)
            zr = 0.0
            zi = 0.0
            zr2 = 0.0
            zi2 = 0.0
            i = 0
            while zr2 + zi2 <= 4.0:
                if i >= maxiter:
                    break
                zi = 2.0 * zr * zi + cim
                zr = zr2 - zi2 + cre
                zr2 = zr * zr
                zi2 = zi * zi
                i += 1
            checksum += i
    return checksum


run()  # warmup
t0 = time.perf_counter_ns()
ans = run()
dt = time.perf_counter_ns() - t0
print(ans)
print(dt)
