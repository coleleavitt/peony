# Phase-0 baseline — label `phase0-baseline`

host: `Linux 7.0.0-08394-g7bd8ef8008b2-dirty x86_64`  cores: 24  date: 2026-06-17T08:50:04-07:00

## hello-c

- inputs: 1
- sha256: `47611685ac6e110485a1a148da382d0ad7987755cbcc86436ed90eb5acc702f3` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         5.84ms   56.6%           0B          0
parse+resolve          3.86ms   37.4%           0B          6
reloc-scan              6.1us    0.1%           0B          0
layout                 49.2us    0.5%           0B          0
emit                   78.9us    0.8%           0B          0
other                 481.8us    4.7%  (startup/teardown/untimed)
TOTAL                 10.31ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs         8.37ms   60.2%           0B          0
parse+resolve          4.76ms   34.2%           0B          6
reloc-scan              7.6us    0.1%           0B          0
layout                 72.7us    0.5%           0B          0
emit                  119.4us    0.9%           0B          0
other                 571.0us    4.1%  (startup/teardown/untimed)
TOTAL                 13.90ms
───────────────────────────────────────────────────────────
```

## hello-cxx

- inputs: 1
- sha256: `3e6219f88bc5865b900104e28913e61252601fe06fbdba49bd6a5473be8b8f54` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         9.22ms   53.7%           0B          0
parse+resolve          7.01ms   40.9%           0B          6
reloc-scan             20.1us    0.1%           0B          0
layout                125.4us    0.7%           0B          0
emit                  194.2us    1.1%           0B          0
other                 593.4us    3.5%  (startup/teardown/untimed)
TOTAL                 17.17ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs         7.75ms   48.0%           0B          0
parse+resolve          7.39ms   45.8%           0B          6
reloc-scan             18.9us    0.1%           0B          0
layout                130.2us    0.8%           0B          0
emit                  189.4us    1.2%           0B          0
other                 660.9us    4.1%  (startup/teardown/untimed)
TOTAL                 16.15ms
───────────────────────────────────────────────────────────
```

## ripgrep

- inputs: 419
- sha256: `bbd111ec2444f8877b096aaa7ac9d5886aff038768a041f3822e89f8f72afa4c` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs        15.90ms    7.7%           0B          0
parse+resolve         56.41ms   27.2%           0B        423
reloc-scan             9.79ms    4.7%           0B          0
layout                14.83ms    7.1%           0B          0
emit                  76.43ms   36.8%           0B          0
other                 34.33ms   16.5%  (startup/teardown/untimed)
TOTAL                207.69ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs        12.98ms    7.5%           0B          0
parse+resolve         42.13ms   24.4%           0B        423
reloc-scan             3.48ms    2.0%           0B          0
layout                14.21ms    8.2%           0B          0
emit                  66.62ms   38.6%           0B          0
other                 33.07ms   19.2%  (startup/teardown/untimed)
TOTAL                172.50ms
───────────────────────────────────────────────────────────
```

## rust-hello

- inputs: 23
- sha256: `52ba36a94f117267a5dc882990d15d3b1e9df2e27a57fea76957b9d91a5b5b53` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         9.54ms   19.0%           0B          0
parse+resolve         22.50ms   44.8%           0B         27
reloc-scan             1.30ms    2.6%           0B          0
layout                649.2us    1.3%           0B          0
emit                  13.78ms   27.5%           0B          0
other                  2.41ms    4.8%  (startup/teardown/untimed)
TOTAL                 50.17ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs         7.66ms   16.9%           0B          0
parse+resolve         20.65ms   45.4%           0B         27
reloc-scan             1.04ms    2.3%           0B          0
layout                559.0us    1.2%           0B          0
emit                  13.23ms   29.1%           0B          0
other                  2.31ms    5.1%  (startup/teardown/untimed)
TOTAL                 45.45ms
───────────────────────────────────────────────────────────
```
