# Phase-0 baseline — label `p2-instrument`

host: `Linux 7.0.0-08394-g7bd8ef8008b2-dirty x86_64`  cores: 24  date: 2026-06-17T09:08:55-07:00

## ripgrep

- inputs: 419
- sha256: `bbd111ec2444f8877b096aaa7ac9d5886aff038768a041f3822e89f8f72afa4c` — DETERMINISTIC

### --stats @ threads=1
```
phase                    wall       %         bytes      items
resolve-inputs        15.13ms    7.7%           0B          0
parse+resolve         55.60ms   28.4%           0B        423
reloc-scan             8.09ms    4.1%           0B          0
reloc-postproc        18.95ms    9.7%           0B          0
layout                12.73ms    6.5%           0B          0
finalize-syms          5.99ms    3.1%           0B          0
emit                  75.48ms   38.5%           0B          0
other                  3.99ms    2.0%  (startup/teardown/untimed)
TOTAL                195.97ms
───────────────────────────────────────────────────────────
```
### --stats @ threads=24 (0=all)
```
phase                    wall       %         bytes      items
resolve-inputs        13.84ms    7.9%           0B          0
parse+resolve         45.08ms   25.8%           0B        423
reloc-scan             2.81ms    1.6%           0B          0
reloc-postproc        19.58ms   11.2%           0B          0
layout                15.52ms    8.9%           0B          0
finalize-syms          5.85ms    3.3%           0B          0
emit                  67.15ms   38.4%           0B          0
other                  4.96ms    2.8%  (startup/teardown/untimed)
TOTAL                174.79ms
───────────────────────────────────────────────────────────
```

