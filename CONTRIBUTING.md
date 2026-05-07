# Contributing

## Crypto Changes Checklist

Before publishing changes to password hashing or verification, run the ignored
constant-time characterization test on an idle local machine. It is intentionally
excluded from CI because shared runners add timing noise; a failure means the
constant-time property may have regressed.

```bash
cargo test -p zk-auth --test password verify_constant_time_property -- --ignored
```
