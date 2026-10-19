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

## Cooperative serialization cancellation

Attach a borrowed checker to the existing `SerializeOptions`; return `true` to
stop. No cancellation-library dependency or static lifetime is required. The
checker requires `Send + Sync`, preserving those traits on options and serializers:

```rust
use std::sync::atomic::{AtomicBool, Ordering};

let stop = AtomicBool::new(false);
let checker = || stop.load(Ordering::Relaxed);
let options = facet_json::SerializeOptions::new().pretty().cancellation(&checker);
let json = facet_json::to_string_with_options(&vec![1_u32, 2, 3], &options)?;
# Ok::<(), facet_json::SerializeError<facet_json::JsonSerializeError>>(())
```

Existing `_with_options` helpers use the checker during shared value traversal
and between bounded chunks of large strings, raw JSON, byte arrays/hex, and pretty
indentation. Default options have no checker and retain their existing behavior.
High-level helpers report `SerializeError::Cancelled`; low-level serializer
operations expose `JsonSerializeError::is_cancelled()`. When driving
`JsonSerializer::with_options(options)` through Facet Format directly, also pass
the checker to `facet_format::serialize_root_with_cancellation` for traversal checks.

Writer helpers stream directly to the supplied sink. A configured checker is
observed during traversal, scalar emission, writes, and retries after
`Interrupted`. Original I/O errors retain their kind and typed payload;
callback cancellation is an `Other` I/O error containing
`SerializeError::Cancelled`, so it is not retried as an interrupted write.
Allocations, user formatting/proxy callbacks, standard UTF-8 string validation,
and one blocked operating-system write cannot be preempted internally. String
helpers check before and after that validation scan. Integrate execution scopes
by checking whether the scope may stop rather than raw cancellation intent,
so required-completion scopes remain protected.

## Writer ownership and streaming

`to_writer_std`, `to_writer_std_pretty`, and `to_writer_std_with_options` emit JSON
as traversal proceeds. They do not assemble a complete document first, install
an internal `BufWriter`, or call the destination's `flush` method. Some encoding
steps make small writes; callers choose buffering and explicitly handle its
flush result:

```rust
use std::io::{BufWriter, Write};

let mut destination = BufWriter::new(Vec::new());
facet_json::to_writer_std(&mut destination, &vec![1_u32, 2, 3])?;
destination.flush()?;
# Ok::<(), std::io::Error>(())
```

`JsonSerializer::from_writer(writer, options)` exposes the same writer-backed
encoder to low-level Facet Format callers. `finish()` returns the supplied sink
without flushing it. Passing `&mut` retains ownership of a buffer, file, or other
resource in the caller, including when serialization fails. An owned sink's own
`Drop` behavior still follows its type's normal rules.

The existing Vec and String helpers collect their results using this same
encoder. A serialization error, cancellation request, or writer failure can leave
a partial JSON prefix in a streaming destination, and earlier writes cannot be
rolled back. When a complete durable document is required, callers can write to
a temporary file, check serialization and flush/persistence results, then publish
the completed artifact.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/facet-rs/facet/blob/main/LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](https://github.com/facet-rs/facet/blob/main/LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
