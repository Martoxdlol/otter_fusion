# Benchmark results

langs: rust, go, node, python, otter | runs: 5 | warmup: 1

### fib

| lang | compute | end-to-end | startup≈ | vs fastest |
|------|--------:|-----------:|---------:|-----------:|
| rust | 15.4 ms | 32.5 ms | 17.1 ms | 1.00× |
| go | 20.1 ms | 43.9 ms | 23.8 ms | 1.30× |
| otter | 31.2 ms | 65.0 ms | 33.9 ms | 2.02× |
| node | 50.7 ms | 120.3 ms | 69.5 ms | 3.30× |
| python | 647.9 ms | 1312.3 ms | 664.3 ms | 42.09× |

### generics

| lang | compute | end-to-end | startup≈ | vs fastest |
|------|--------:|-----------:|---------:|-----------:|
| rust | 25.6 ms | 53.5 ms | 27.9 ms | 1.00× |
| go | 25.8 ms | 30.6 ms | 4.8 ms | 1.01× |
| node | 29.7 ms | 78.7 ms | 49.0 ms | 1.16× |
| otter | 145.8 ms | 303.7 ms | 157.9 ms | 5.69× |
| python | 856.6 ms | 1733.8 ms | 877.1 ms | 33.46× |

### interfaces

| lang | compute | end-to-end | startup≈ | vs fastest |
|------|--------:|-----------:|---------:|-----------:|
| rust | 27.0 ms | 56.3 ms | 29.2 ms | 1.00× |
| node | 31.3 ms | 84.4 ms | 53.1 ms | 1.16× |
| go | 40.5 ms | 83.9 ms | 43.4 ms | 1.50× |
| otter | 166.1 ms | 336.2 ms | 170.1 ms | 6.15× |
| python | 522.0 ms | 1077.5 ms | 555.6 ms | 19.31× |

### iterators

| lang | compute | end-to-end | startup≈ | vs fastest |
|------|--------:|-----------:|---------:|-----------:|
| rust | 9.3 ms | 20.8 ms | 11.5 ms | 1.00× |
| go | 9.4 ms | 14.8 ms | 5.4 ms | 1.01× |
| node | 185.9 ms | 388.1 ms | 202.3 ms | 19.92× |
| python | 348.3 ms | 710.6 ms | 362.3 ms | 37.33× |
| otter | 460.3 ms | 942.0 ms | 481.6 ms | 49.34× |

### mandelbrot

| lang | compute | end-to-end | startup≈ | vs fastest |
|------|--------:|-----------:|---------:|-----------:|
| rust | 83.4 ms | 169.3 ms | 85.9 ms | 1.00× |
| go | 83.6 ms | 171.3 ms | 87.7 ms | 1.00× |
| node | 86.5 ms | 194.4 ms | 107.9 ms | 1.04× |
| otter | 88.8 ms | 180.9 ms | 92.1 ms | 1.06× |
| python | 2891.5 ms | 5825.6 ms | 2934.0 ms | 34.69× |

### sieve

| lang | compute | end-to-end | startup≈ | vs fastest |
|------|--------:|-----------:|---------:|-----------:|
| rust | 3.9 ms | 9.9 ms | 6.0 ms | 1.00× |
| go | 4.6 ms | 12.0 ms | 7.4 ms | 1.16× |
| node | 5.7 ms | 32.1 ms | 26.4 ms | 1.45× |
| otter | 101.9 ms | 210.4 ms | 108.5 ms | 26.02× |
| python | 116.0 ms | 247.2 ms | 131.2 ms | 29.64× |

### unions

| lang | compute | end-to-end | startup≈ | vs fastest |
|------|--------:|-----------:|---------:|-----------:|
| go | 11.9 ms | 26.5 ms | 14.7 ms | 1.00× |
| node | 38.3 ms | 97.6 ms | 59.3 ms | 3.22× |
| rust | 226.6 ms | 458.2 ms | 231.6 ms | 19.07× |
| otter | 704.1 ms | 1451.7 ms | 747.7 ms | 59.25× |
| python | 789.6 ms | 1608.4 ms | 818.8 ms | 66.45× |
