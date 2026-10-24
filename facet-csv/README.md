# teamy-facet-csv

> **Unofficial fork:** `teamy-facet-csv` is maintained by
> [TeamDman](https://github.com/TeamDman/facet-format), independently of facet-rs.
> It is not an official upstream release. The source derives from
> [official Facet Format at `4279debff780ae1cd5b028201f446c26594b1120`](https://github.com/facet-rs/facet-format/tree/4279debff780ae1cd5b028201f446c26594b1120)
> and the reviewed [Teamy integration](https://github.com/TeamDman/facet-format/tree/a1ca5f97253eb49fb080a39e32377dcebb0499f5).
> Original attribution and MIT OR Apache-2.0 licenses are preserved.

Install with the original Rust import name and the matching Teamy core:

```toml
[dependencies]
facet = { package = "teamy-facet", version = "=0.50.0-rc.7" }
facet-csv = { package = "teamy-facet-csv", version = "=0.50.0-rc.9" }
```

The Teamy packages use distinct Cargo identities from upstream. Use the matching
`teamy-facet-*` family together; the Rust import name remains `facet_csv`.

<!-- cargo-reedme: start -->

<!-- cargo-reedme: info-start

    Do not edit this region by hand
    ===============================

    This region was generated from Rust documentation comments by `cargo-reedme` using this command:

        cargo +nightly reedme --workspace

    for more info: https://github.com/nik-rev/cargo-reedme

cargo-reedme: info-end -->

CSV parser and serializer using facet-format.

**Note:** CSV is a fundamentally different format from JSON/XML/YAML.
While those formats are tree-structured and map naturally to nested types,
CSV is a flat, row-based format where each row represents a single record
and each column represents a field.

This crate provides basic CSV support via the `FormatParser` trait, but
has significant limitations:

- No support for nested structures (CSV is inherently flat)
- No support for arrays/sequences as field values
- No support for enums beyond unit variants (encoded as strings)
- All values are strings and must be parseable to target types

For more sophisticated CSV handling, consider a dedicated CSV library.

<!-- cargo-reedme: end -->
