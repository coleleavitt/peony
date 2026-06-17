# Phase-0 baseline — label `p2-buildid`

host: `Linux 7.0.0-08394-g7bd8ef8008b2-dirty x86_64`  cores: 24  date: 2026-06-17T09:13:33-07:00

## hello-c

- inputs: 1
- sha256: `cf27bdf877d11886276428e86aa2c32005c0e9d4aa905b82626448b6098ae1e4` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         7.14ms   57.2%           0B          0
parse+resolve          4.77ms   38.3%           0B          6
reloc-scan              8.0us    0.1%           0B          0
reloc-postproc         22.9us    0.2%           0B          0
layout                 79.1us    0.6%           0B          0
finalize-syms          11.2us    0.1%           0B          0
emit                  157.4us    1.3%           0B          0
other                 285.2us    2.3%  (startup/teardown/untimed)
TOTAL                 12.48ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         6.40ms   54.4%           0B          0
parse+resolve          4.67ms   39.7%           0B          6
reloc-scan              7.9us    0.1%           0B          0
reloc-postproc         17.5us    0.1%           0B          0
layout                 76.7us    0.7%           0B          0
finalize-syms          11.0us    0.1%           0B          0
emit                  153.5us    1.3%           0B          0
other                 425.4us    3.6%  (startup/teardown/untimed)
TOTAL                 11.76ms
───────────────────────────────────────────────────────────
```

## hello-cxx

- inputs: 1
- sha256: `4de128aabff79b13b1ed973aa47ceb34bc33a9e8f8b8cccad78769c74ad0cb77` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         9.25ms   51.8%           0B          0
parse+resolve          7.61ms   42.6%           0B          6
reloc-scan             20.9us    0.1%           0B          0
reloc-postproc         39.4us    0.2%           0B          0
layout                139.4us    0.8%           0B          0
finalize-syms          21.0us    0.1%           0B          0
emit                  220.6us    1.2%           0B          0
other                 549.4us    3.1%  (startup/teardown/untimed)
TOTAL                 17.85ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         8.71ms   50.0%           0B          0
parse+resolve          7.69ms   44.2%           0B          6
reloc-scan             21.5us    0.1%           0B          0
reloc-postproc         37.7us    0.2%           0B          0
layout                132.8us    0.8%           0B          0
finalize-syms          20.8us    0.1%           0B          0
emit                  222.9us    1.3%           0B          0
other                 573.8us    3.3%  (startup/teardown/untimed)
TOTAL                 17.41ms
───────────────────────────────────────────────────────────
```

## ripgrep

- inputs: 419
- sha256: `ff8e621e3454e3f8c67fa050fd730f6987e57675942678efa048c865531ed7e3` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs        17.09ms   10.1%           0B          0
parse+resolve         54.25ms   31.9%           0B        423
reloc-scan             9.47ms    5.6%           0B          0
reloc-postproc        21.89ms   12.9%           0B          0
layout                14.25ms    8.4%           0B          0
finalize-syms          6.67ms    3.9%           0B          0
emit                  41.87ms   24.6%           0B          0
other                  4.39ms    2.6%  (startup/teardown/untimed)
TOTAL                169.89ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs        17.10ms   13.9%           0B          0
parse+resolve         42.39ms   34.5%           0B        423
reloc-scan             3.11ms    2.5%           0B          0
reloc-postproc        20.71ms   16.9%           0B          0
layout                14.35ms   11.7%           0B          0
finalize-syms          6.16ms    5.0%           0B          0
emit                  13.61ms   11.1%           0B          0
other                  5.46ms    4.4%  (startup/teardown/untimed)
TOTAL                122.89ms
───────────────────────────────────────────────────────────
```

## rust-hello

- inputs: 23
- sha256: `08391641374d595f6eca06b5d4049475cb0f7dc710d1946f46b210d8839f06bc` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         7.46ms   22.0%           0B          0
parse+resolve         19.86ms   58.5%           0B         27
reloc-scan             1.33ms    3.9%           0B          0
reloc-postproc         1.10ms    3.2%           0B          0
layout                688.0us    2.0%           0B          0
finalize-syms         226.6us    0.7%           0B          0
emit                   2.22ms    6.5%           0B          0
other                  1.07ms    3.2%  (startup/teardown/untimed)
TOTAL                 33.95ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         7.31ms   21.2%           0B          0
parse+resolve         21.66ms   62.9%           0B         27
reloc-scan             1.06ms    3.1%           0B          0
reloc-postproc        848.8us    2.5%           0B          0
layout                560.6us    1.6%           0B          0
finalize-syms         181.2us    0.5%           0B          0
emit                   1.82ms    5.3%           0B          0
other                 983.5us    2.9%  (startup/teardown/untimed)
TOTAL                 34.42ms
───────────────────────────────────────────────────────────
```

