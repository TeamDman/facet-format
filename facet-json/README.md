# teamy-facet-json

> **Unofficial fork:** `teamy-facet-json` is maintained by
> [TeamDman](https://github.com/TeamDman/facet-format), independently of facet-rs.
> It is not an official upstream release. The source derives from
> [official Facet Format at `4279debff780ae1cd5b028201f446c26594b1120`](https://github.com/facet-rs/facet-format/tree/4279debff780ae1cd5b028201f446c26594b1120)
> and the reviewed [Teamy integration](https://github.com/TeamDman/facet-format/tree/a1ca5f97253eb49fb080a39e32377dcebb0499f5).
> Original attribution and MIT OR Apache-2.0 licenses are preserved.

Install with the original Rust import name and the matching Teamy core:

```toml
[dependencies]
facet = { package = "teamy-facet", version = "=0.50.0-rc.7" }
facet-json = { package = "teamy-facet-json", version = "=0.50.0-rc.7" }
```

[![crates.io](https://img.shields.io/crates/v/teamy-facet-json.svg)](https://crates.io/crates/teamy-facet-json)
[![documentation](https://docs.rs/teamy-facet-json/badge.svg)](https://docs.rs/teamy-facet-json)
[![MIT/Apache-2.0 licensed](https://img.shields.io/crates/l/teamy-facet-json.svg)](https://github.com/facet-rs/facet/blob/main/LICENSE-MIT)

`facet-json` is the JSON serializer and deserializer for the facet ecosystem. It
reads and writes JSON for any type that derives `Facet` — no manual `Serialize`
or `Deserialize` implementations, no attribute-heavy schemas.

```rust
use facet::Facet;
use facet_json::{from_str, to_string};

#[derive(Facet, Debug, PartialEq)]
struct Person {
    name: String,
    age: u32,
}

let json = r#"{"name":"Alice","age":30}"#;
let person: Person = from_str(json).unwrap();
assert_eq!(person.name, "Alice");
assert_eq!(person.age, 30);

let out = to_string(&person).unwrap();
println!("{out}");
```

The primary entry points are [`from_str`] and [`to_string`] for the common case,
with [`from_slice`] / [`to_vec`] for byte-oriented callers and
[`to_string_pretty`] for human-readable output. Zero-copy deserialization of
`&str` fields is available via [`from_str_borrowed`].

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/facet-rs/facet/blob/main/LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](https://github.com/facet-rs/facet/blob/main/LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
