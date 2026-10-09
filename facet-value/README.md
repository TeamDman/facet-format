# teamy-facet-value

> **Unofficial fork:** `teamy-facet-value` is maintained by
> [TeamDman](https://github.com/TeamDman/facet-format), independently of facet-rs.
> It is not an official upstream release. The source derives from
> [official Facet Format at `4279debff780ae1cd5b028201f446c26594b1120`](https://github.com/facet-rs/facet-format/tree/4279debff780ae1cd5b028201f446c26594b1120)
> and the reviewed [Teamy integration](https://github.com/TeamDman/facet-format/tree/a1ca5f97253eb49fb080a39e32377dcebb0499f5).
> Original attribution and MIT OR Apache-2.0 licenses are preserved.

Install with the original Rust import name and the matching Teamy core:

```toml
[dependencies]
facet = { package = "teamy-facet", version = "=0.50.0-rc.7" }
facet-value = { package = "teamy-facet-value", version = "=0.50.0-rc.7" }
```

[![crates.io](https://img.shields.io/crates/v/teamy-facet-value.svg)](https://crates.io/crates/teamy-facet-value)
[![documentation](https://docs.rs/teamy-facet-value/badge.svg)](https://docs.rs/teamy-facet-value)
[![MIT/Apache-2.0 licensed](https://img.shields.io/crates/l/teamy-facet-value.svg)](LICENSE-MIT)

<!-- cargo-reedme: start -->

<!-- cargo-reedme: info-start

    Do not edit this region by hand
    ===============================

    This region was generated from Rust documentation comments by `cargo-reedme` using this command:

        cargo +nightly reedme --workspace

    for more info: https://github.com/nik-rev/cargo-reedme

cargo-reedme: info-end -->

`facet-value` provides a memory-efficient dynamic value type for representing
structured data similar to JSON, but with added support for binary data and datetime.

## Features

- **Pointer-sized `Value` type**: The main `Value` type is exactly one pointer in size
- **Eight value types**: Null, Bool, Number, String, Bytes, Array, Object, DateTime
- **`no_std` compatible**: Works with just `alloc`, no standard library required
- **Bytes support**: First-class support for binary data (useful for MessagePack, CBOR, etc.)
- **DateTime support**: First-class support for temporal data (useful for TOML, YAML, etc.)

## Design

`Value` uses a tagged pointer representation with 8-byte alignment, giving us 3 tag bits
to distinguish between value types. Inline values (null, true, false) don’t require
heap allocation.

<!-- cargo-reedme: end -->

# facet-value

A memory-efficient dynamic value type for representing structured data, with support for bytes.

## Features

- **Pointer-sized**: `Value` is exactly one pointer in size using tagged pointers
- **Rich type support**: Null, Bool, Number, String, Bytes, Array, Object, DateTime
- **Typed extraction**: Convert from `Value` into any type implementing `Facet`
- **Companion serializer**: Use `facet-format` to serialize typed values into `Value`

## Example

```rust
use facet::Facet;
use facet_value::{Value, from_value};
use facet_format::to_value;

#[derive(Debug, Facet, PartialEq)]
struct Person {
    name: String,
    age: u32,
}

// Convert a typed value to a dynamic Value
let person = Person { name: "Alice".into(), age: 30 };
let value: Value = to_value(&person).unwrap();

// Inspect the value dynamically
let obj = value.as_object().unwrap();
assert_eq!(obj.get("name").unwrap().as_string().unwrap().as_str(), "Alice");

// Convert back to a typed value
let person2: Person = from_value(value).unwrap();
assert_eq!(person, person2);
```

## Sponsors

Thanks to all individual sponsors:

<p> <a href="https://github.com/sponsors/fasterthanlime">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/github-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/github-light.svg" height="40" alt="GitHub Sponsors">
</picture>
</a> <a href="https://patreon.com/fasterthanlime">
    <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/patreon-dark.svg">
    <img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/patreon-light.svg" height="40" alt="Patreon">
    </picture>
</a> </p>

...along with corporate sponsors:

<p> <a href="https://aws.amazon.com">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/aws-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/aws-light.svg" height="40" alt="AWS">
</picture>
</a> <a href="https://zed.dev">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/zed-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/zed-light.svg" height="40" alt="Zed">
</picture>
</a> <a href="https://depot.dev?utm_source=facet">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/depot-dark.svg">
<img src="https://github.com/facet-rs/facet/raw/main/static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a> </p>

...without whom this work could not exist.

## Special thanks

The facet logo was drawn by [Misiasart](https://misiasart.com/).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/facet-rs/facet/blob/main/LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](https://github.com/facet-rs/facet/blob/main/LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
