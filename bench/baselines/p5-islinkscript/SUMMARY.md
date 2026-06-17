# Phase-0 baseline — label `p5-islinkscript`

host: `Linux 7.0.0-08394-g7bd8ef8008b2-dirty x86_64`  cores: 24  date: 2026-06-17T09:56:37-07:00

## hello-c

- inputs: 1
- sha256: `cf27bdf877d11886276428e86aa2c32005c0e9d4aa905b82626448b6098ae1e4` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         1.17ms   22.2%           0B          0
parse+resolve          3.66ms   69.5%           0B          6
reloc-scan              4.4us    0.1%           0B          0
reloc-postproc         11.2us    0.2%           0B          0
layout                 42.8us    0.8%           0B          0
finalize-syms           6.0us    0.1%           0B          0
emit                   78.2us    1.5%           0B          0
other                 293.9us    5.6%  (startup/teardown/untimed)
TOTAL                  5.27ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         1.20ms   19.1%           0B          0
parse+resolve          4.58ms   72.9%           0B          6
reloc-scan              5.0us    0.1%           0B          0
reloc-postproc         13.1us    0.2%           0B          0
layout                 52.3us    0.8%           0B          0
finalize-syms           7.4us    0.1%           0B          0
emit                   82.8us    1.3%           0B          0
other                 340.1us    5.4%  (startup/teardown/untimed)
TOTAL                  6.28ms
───────────────────────────────────────────────────────────
```

## hello-cxx

- inputs: 1
- sha256: `4de128aabff79b13b1ed973aa47ceb34bc33a9e8f8b8cccad78769c74ad0cb77` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         1.11ms   14.2%           0B          0
parse+resolve          5.79ms   74.2%           0B          6
reloc-scan             11.2us    0.1%           0B          0
reloc-postproc         20.7us    0.3%           0B          0
layout                 66.4us    0.9%           0B          0
finalize-syms           9.1us    0.1%           0B          0
emit                  109.1us    1.4%           0B          0
other                 687.5us    8.8%  (startup/teardown/untimed)
TOTAL                  7.80ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         1.38ms   16.7%           0B          0
parse+resolve          5.88ms   71.2%           0B          6
reloc-scan             11.1us    0.1%           0B          0
reloc-postproc         22.8us    0.3%           0B          0
layout                 63.0us    0.8%           0B          0
finalize-syms           9.2us    0.1%           0B          0
emit                  119.3us    1.4%           0B          0
other                 779.6us    9.4%  (startup/teardown/untimed)
TOTAL                  8.26ms
───────────────────────────────────────────────────────────
```

## ripgrep

- inputs: 419
- sha256: `ff8e621e3454e3f8c67fa050fd730f6987e57675942678efa048c865531ed7e3` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         2.14ms    2.3%           0B          0
parse+resolve         29.26ms   31.8%           0B        423
reloc-scan             5.85ms    6.4%           0B          0
reloc-postproc        14.30ms   15.6%           0B          0
layout                 8.57ms    9.3%           0B          0
finalize-syms          5.62ms    6.1%           0B          0
emit                  23.47ms   25.5%           0B          0
other                  2.71ms    2.9%  (startup/teardown/untimed)
TOTAL                 91.91ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         2.21ms    2.6%           0B          0
parse+resolve         37.82ms   44.5%           0B        423
reloc-scan             3.21ms    3.8%           0B          0
reloc-postproc        14.15ms   16.7%           0B          0
layout                 8.33ms    9.8%           0B          0
finalize-syms          4.20ms    4.9%           0B          0
emit                  11.88ms   14.0%           0B          0
other                  3.20ms    3.8%  (startup/teardown/untimed)
TOTAL                 84.99ms
───────────────────────────────────────────────────────────
```

## rust-hello

- inputs: 23
- sha256: `08391641374d595f6eca06b5d4049475cb0f7dc710d1946f46b210d8839f06bc` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         1.58ms    9.5%           0B          0
parse+resolve         11.88ms   71.3%           0B         27
reloc-scan            545.0us    3.3%           0B          0
reloc-postproc        582.5us    3.5%           0B          0
layout                381.9us    2.3%           0B          0
finalize-syms         101.0us    0.6%           0B          0
emit                  899.9us    5.4%           0B          0
other                 692.0us    4.2%  (startup/teardown/untimed)
TOTAL                 16.66ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs         1.30ms    9.5%           0B          0
parse+resolve          9.70ms   71.2%           0B         27
reloc-scan            486.2us    3.6%           0B          0
reloc-postproc        413.8us    3.0%           0B          0
layout                270.0us    2.0%           0B          0
finalize-syms          78.8us    0.6%           0B          0
emit                  826.9us    6.1%           0B          0
other                 555.3us    4.1%  (startup/teardown/untimed)
TOTAL                 13.62ms
───────────────────────────────────────────────────────────
```

