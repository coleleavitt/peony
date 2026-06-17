# Phase-0 baseline — label `p3-zerocopy`

host: `Linux 7.0.0-08394-g7bd8ef8008b2-dirty x86_64`  cores: 24  date: 2026-06-17T09:42:06-07:00

## hello-c

- inputs: 1
- sha256: `cf27bdf877d11886276428e86aa2c32005c0e9d4aa905b82626448b6098ae1e4` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         7.13ms   53.0%           0B          0
parse+resolve          5.61ms   41.7%           0B          6
reloc-scan             10.2us    0.1%           0B          0
reloc-postproc         26.0us    0.2%           0B          0
layout                 85.4us    0.6%           0B          0
finalize-syms          11.7us    0.1%           0B          0
emit                  152.9us    1.1%           0B          0
other                 415.5us    3.1%  (startup/teardown/untimed)
TOTAL                 13.44ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         7.13ms   56.9%           0B          0
parse+resolve          4.68ms   37.4%           0B          6
reloc-scan              9.2us    0.1%           0B          0
reloc-postproc         24.6us    0.2%           0B          0
layout                 85.2us    0.7%           0B          0
finalize-syms          11.8us    0.1%           0B          0
emit                  150.2us    1.2%           0B          0
other                 431.1us    3.4%  (startup/teardown/untimed)
TOTAL                 12.52ms
───────────────────────────────────────────────────────────
```

## hello-cxx

- inputs: 1
- sha256: `4de128aabff79b13b1ed973aa47ceb34bc33a9e8f8b8cccad78769c74ad0cb77` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs        71.02ms   87.7%           0B          0
parse+resolve          8.96ms   11.1%           0B          6
reloc-scan             25.0us    0.0%           0B          0
reloc-postproc         45.8us    0.1%           0B          0
layout                155.4us    0.2%           0B          0
finalize-syms          23.4us    0.0%           0B          0
emit                  246.7us    0.3%           0B          0
other                 515.2us    0.6%  (startup/teardown/untimed)
TOTAL                 81.00ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         9.51ms   46.4%           0B          0
parse+resolve         10.01ms   48.8%           0B          6
reloc-scan             23.0us    0.1%           0B          0
reloc-postproc         43.4us    0.2%           0B          0
layout                150.9us    0.7%           0B          0
finalize-syms          23.5us    0.1%           0B          0
emit                  254.5us    1.2%           0B          0
other                 498.8us    2.4%  (startup/teardown/untimed)
TOTAL                 20.51ms
───────────────────────────────────────────────────────────
```

## ripgrep

- inputs: 419
- sha256: `ff8e621e3454e3f8c67fa050fd730f6987e57675942678efa048c865531ed7e3` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs        15.85ms    9.6%           0B          0
parse+resolve         45.11ms   27.3%           0B        423
reloc-scan             9.82ms    5.9%           0B          0
reloc-postproc        24.09ms   14.6%           0B          0
layout                16.07ms    9.7%           0B          0
finalize-syms          8.31ms    5.0%           0B          0
emit                  41.71ms   25.2%           0B          0
other                  4.21ms    2.6%  (startup/teardown/untimed)
TOTAL                165.18ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs        13.74ms    8.5%           0B          0
parse+resolve         73.94ms   45.5%           0B        423
reloc-scan             3.70ms    2.3%           0B          0
reloc-postproc        26.06ms   16.0%           0B          0
layout                16.47ms   10.1%           0B          0
finalize-syms          8.12ms    5.0%           0B          0
emit                  15.09ms    9.3%           0B          0
other                  5.33ms    3.3%  (startup/teardown/untimed)
TOTAL                162.45ms
───────────────────────────────────────────────────────────
```

## rust-hello

- inputs: 23
- sha256: `08391641374d595f6eca06b5d4049475cb0f7dc710d1946f46b210d8839f06bc` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         9.00ms   26.6%           0B          0
parse+resolve         17.17ms   50.8%           0B         27
reloc-scan             1.36ms    4.0%           0B          0
reloc-postproc         1.33ms    3.9%           0B          0
layout                776.2us    2.3%           0B          0
finalize-syms         246.4us    0.7%           0B          0
emit                   2.53ms    7.5%           0B          0
other                  1.40ms    4.2%  (startup/teardown/untimed)
TOTAL                 33.82ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         8.62ms   28.5%           0B          0
parse+resolve         15.39ms   50.8%           0B         27
reloc-scan             1.18ms    3.9%           0B          0
reloc-postproc        990.3us    3.3%           0B          0
layout                672.7us    2.2%           0B          0
finalize-syms         214.6us    0.7%           0B          0
emit                   2.20ms    7.3%           0B          0
other                  1.01ms    3.3%  (startup/teardown/untimed)
TOTAL                 30.28ms
───────────────────────────────────────────────────────────
```

