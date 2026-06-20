# Phase-0 baseline — label `final-epoch`

host: `Linux 7.0.0-08394-g7bd8ef8008b2-dirty x86_64`  cores: 24  date: 2026-06-17T10:08:02-07:00

## hello-c

- inputs: 1
- sha256: `cf27bdf877d11886276428e86aa2c32005c0e9d4aa905b82626448b6098ae1e4` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         1.29ms   24.3%           0B          0
parse+resolve          3.62ms   68.1%           0B          6
reloc-scan              3.1us    0.1%           0B          0
reloc-postproc          8.9us    0.2%           0B          0
layout                 30.9us    0.6%           0B          0
finalize-syms           4.5us    0.1%           0B          0
emit                   57.7us    1.1%           0B          0
other                 295.3us    5.6%  (startup/teardown/untimed)
TOTAL                  5.31ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs        998.9us   21.1%           0B          0
parse+resolve          3.29ms   69.6%           0B          6
reloc-scan              2.9us    0.1%           0B          0
reloc-postproc          8.3us    0.2%           0B          0
layout                 33.9us    0.7%           0B          0
finalize-syms           4.7us    0.1%           0B          0
emit                   57.2us    1.2%           0B          0
other                 333.9us    7.1%  (startup/teardown/untimed)
TOTAL                  4.73ms
───────────────────────────────────────────────────────────
```

## hello-cxx

- inputs: 1
- sha256: `4de128aabff79b13b1ed973aa47ceb34bc33a9e8f8b8cccad78769c74ad0cb77` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         1.25ms   13.4%           0B          0
parse+resolve          7.56ms   81.3%           0B          6
reloc-scan             12.5us    0.1%           0B          0
reloc-postproc         26.9us    0.3%           0B          0
layout                 80.7us    0.9%           0B          0
finalize-syms          11.0us    0.1%           0B          0
emit                  105.1us    1.1%           0B          0
other                 250.1us    2.7%  (startup/teardown/untimed)
TOTAL                  9.30ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs         1.18ms   13.0%           0B          0
parse+resolve          7.57ms   83.3%           0B          6
reloc-scan             12.1us    0.1%           0B          0
reloc-postproc         27.2us    0.3%           0B          0
layout                 76.3us    0.8%           0B          0
finalize-syms          10.4us    0.1%           0B          0
emit                  107.7us    1.2%           0B          0
other                 104.6us    1.2%  (startup/teardown/untimed)
TOTAL                  9.10ms
───────────────────────────────────────────────────────────
```

## ripgrep

- inputs: 419
- sha256: `ff8e621e3454e3f8c67fa050fd730f6987e57675942678efa048c865531ed7e3` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         2.00ms    2.5%           0B          0
parse+resolve         27.39ms   34.0%           0B        423
reloc-scan             5.69ms    7.1%           0B          0
reloc-postproc        12.72ms   15.8%           0B          0
layout                 7.12ms    8.8%           0B          0
finalize-syms          3.51ms    4.4%           0B          0
emit                  19.33ms   24.0%           0B          0
other                  2.74ms    3.4%  (startup/teardown/untimed)
TOTAL                 80.51ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs         1.89ms    3.3%           0B          0
parse+resolve         20.74ms   36.0%           0B        423
reloc-scan             2.15ms    3.7%           0B          0
reloc-postproc        10.73ms   18.6%           0B          0
layout                 7.49ms   13.0%           0B          0
finalize-syms          3.20ms    5.6%           0B          0
emit                   7.70ms   13.4%           0B          0
other                  3.66ms    6.4%  (startup/teardown/untimed)
TOTAL                 57.57ms
───────────────────────────────────────────────────────────
```

## rust-hello

- inputs: 23
- sha256: `08391641374d595f6eca06b5d4049475cb0f7dc710d1946f46b210d8839f06bc` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs         1.10ms    8.8%           0B          0
parse+resolve          8.95ms   71.9%           0B         27
reloc-scan            455.2us    3.7%           0B          0
reloc-postproc        389.6us    3.1%           0B          0
layout                249.8us    2.0%           0B          0
finalize-syms          77.9us    0.6%           0B          0
emit                  802.9us    6.5%           0B          0
other                 421.9us    3.4%  (startup/teardown/untimed)
TOTAL                 12.45ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=auto (0=auto, host cores=24)
```
phase                    wall       %         bytes      items
resolve-inputs         1.29ms    8.5%           0B          0
parse+resolve         11.40ms   74.8%           0B         27
reloc-scan            477.6us    3.1%           0B          0
reloc-postproc        392.9us    2.6%           0B          0
layout                257.4us    1.7%           0B          0
finalize-syms          79.9us    0.5%           0B          0
emit                  814.6us    5.3%           0B          0
other                 531.7us    3.5%  (startup/teardown/untimed)
TOTAL                 15.25ms
───────────────────────────────────────────────────────────
```
