# fixtures/
Oracle cases. Layout: `fixtures/<case>/case.toml` + the files it names. Generated from inputs WE authored; never corpus files with unclear rights, never personal data.

`case.toml`:
```toml
cmd = ["oracle", "parse-len"]   # args for target/release/hexe-cli; absolute input path is appended
input = "input.txt"
expected = "expected.txt"       # what the original produced (stdout bytes of the CLI must match per `compare`)
compare = "bytes"               # bytes | json | text-f1 | png-psnr | geo | stat | stat-disp (statistical modes + optional `base`: parity/README.md)
feature = "example.core-parse-len"  # optional ledger id: if that row is shipped the case must reach `threshold`
threshold = 1.0                 # optional; defaults: bytes/json 1.0, text-f1 0.97, png-psnr 40 dB
```
Every file is listed in `MANIFEST.toml` (sha256, producer, original's version, date). Floors: `xtask/baselines/oracle.json` (`cargo xtask oracle --raise` ratchets up, never down).
A case may have a perf budget in `perf/budgets.toml` (`case = "<dir name>"`).
