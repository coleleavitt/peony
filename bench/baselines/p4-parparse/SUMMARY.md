# Phase-0 baseline — label `p4-parparse`

host: `Linux 7.0.0-08394-g7bd8ef8008b2-dirty x86_64`  cores: 24  date: 2026-06-17T09:49:47-07:00

## hello-c

- inputs: 1
- sha256: `cf27bdf877d11886276428e86aa2c32005c0e9d4aa905b82626448b6098ae1e4` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         3.72ms   52.8%           0B          0
parse+resolve          2.67ms   38.0%           0B          6
reloc-scan              6.2us    0.1%           0B          0
reloc-postproc         16.7us    0.2%           0B          0
layout                 65.9us    0.9%           0B          0
finalize-syms           8.2us    0.1%           0B          0
emit                  108.6us    1.5%           0B          0
other                 448.2us    6.4%  (startup/teardown/untimed)
TOTAL                  7.05ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         3.78ms   55.3%           0B          0
parse+resolve          2.37ms   34.7%           0B          6
reloc-scan              4.1us    0.1%           0B          0
reloc-postproc         10.5us    0.2%           0B          0
layout                 44.1us    0.6%           0B          0
finalize-syms           5.3us    0.1%           0B          0
emit                   74.6us    1.1%           0B          0
other                 542.5us    8.0%  (startup/teardown/untimed)
TOTAL                  6.82ms
───────────────────────────────────────────────────────────
```

## hello-cxx

- inputs: 1
- sha256: `4de128aabff79b13b1ed973aa47ceb34bc33a9e8f8b8cccad78769c74ad0cb77` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         4.31ms   46.5%           0B          0
parse+resolve          4.60ms   49.6%           0B          6
reloc-scan              9.9us    0.1%           0B          0
reloc-postproc         21.3us    0.2%           0B          0
layout                 65.4us    0.7%           0B          0
finalize-syms           8.5us    0.1%           0B          0
emit                   99.1us    1.1%           0B          0
other                 164.2us    1.8%  (startup/teardown/untimed)
TOTAL                  9.28ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         4.76ms   46.2%           0B          0
parse+resolve          4.97ms   48.3%           0B          6
reloc-scan             12.9us    0.1%           0B          0
reloc-postproc         25.6us    0.2%           0B          0
layout                 72.6us    0.7%           0B          0
finalize-syms          10.1us    0.1%           0B          0
emit                  118.2us    1.1%           0B          0
other                 323.3us    3.1%  (startup/teardown/untimed)
TOTAL                 10.29ms
───────────────────────────────────────────────────────────
```

## ripgrep

- inputs: 419
- sha256: `ff8e621e3454e3f8c67fa050fd730f6987e57675942678efa048c865531ed7e3` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         8.24ms    9.7%           0B          0
parse+resolve         26.67ms   31.5%           0B        423
reloc-scan             5.99ms    7.1%           0B          0
reloc-postproc        11.95ms   14.1%           0B          0
layout                 7.04ms    8.3%           0B          0
finalize-syms          3.47ms    4.1%           0B          0
emit                  18.54ms   21.9%           0B          0
other                  2.79ms    3.3%  (startup/teardown/untimed)
TOTAL                 84.69ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         8.92ms   11.9%           0B          0
parse+resolve         24.16ms   32.3%           0B        423
reloc-scan             2.13ms    2.8%           0B          0
reloc-postproc        13.56ms   18.1%           0B          0
layout                 9.80ms   13.1%           0B          0
finalize-syms          4.33ms    5.8%           0B          0
emit                   8.98ms   12.0%           0B          0
other                  2.95ms    3.9%  (startup/teardown/untimed)
TOTAL                 74.82ms
───────────────────────────────────────────────────────────
```

## rust-hello

- inputs: 23
- sha256: `08391641374d595f6eca06b5d4049475cb0f7dc710d1946f46b210d8839f06bc` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         4.04ms   26.7%           0B          0
parse+resolve          8.21ms   54.4%           0B         27
reloc-scan            500.9us    3.3%           0B          0
reloc-postproc        450.0us    3.0%           0B          0
layout                290.2us    1.9%           0B          0
finalize-syms          86.6us    0.6%           0B          0
emit                  914.5us    6.1%           0B          0
other                 609.7us    4.0%  (startup/teardown/untimed)
TOTAL                 15.10ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         4.15ms   26.8%           0B          0
parse+resolve          8.46ms   54.7%           0B         27
reloc-scan            452.4us    2.9%           0B          0
reloc-postproc        431.8us    2.8%           0B          0
layout                290.0us    1.9%           0B          0
finalize-syms          84.1us    0.5%           0B          0
emit                  878.7us    5.7%           0B          0
other                 728.0us    4.7%  (startup/teardown/untimed)
TOTAL                 15.48ms
───────────────────────────────────────────────────────────
```

