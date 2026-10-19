# teamy-facet-format

> **Unofficial fork:** `teamy-facet-format` is maintained by
> [TeamDman](https://github.com/TeamDman/facet-format), independently of facet-rs.
> It is not an official upstream release. The source derives from
> [official Facet Format at `4279debff780ae1cd5b028201f446c26594b1120`](https://github.com/facet-rs/facet-format/tree/4279debff780ae1cd5b028201f446c26594b1120)
> and the reviewed [Teamy integration](https://github.com/TeamDman/facet-format/tree/a1ca5f97253eb49fb080a39e32377dcebb0499f5).
> Original attribution and MIT OR Apache-2.0 licenses are preserved.

Install with the original Rust import name and the matching Teamy core:

```toml
[dependencies]
facet = { package = "teamy-facet", version = "=0.50.0-rc.7" }
facet-format = { package = "teamy-facet-format", version = "=0.50.0-rc.8" }
```

## Cooperative serialization cancellation

`serialize_root_with_cancellation(serializer, value, &should_cancel)` accepts a
borrowed `Fn() -> bool`. Returning `true` stops traversal with
`SerializeError::Cancelled`. The callback can borrow local state and needs no
`Send`, `Sync`, or `'static` bound; no cancellation-library dependency is needed.
Existing `serialize_root` callers keep their current behavior.

Use `serialize_value_with_shape_with_cancellation` for the separate shape-guided
dynamic-value traversal. `SerializeContext::with_cancellation` exposes the same
contract for direct context users.

Checks occur between values, fields (including omitted fields), collection
iterations, and backend calls. A backend, custom formatter, or proxy that performs
long-running work within one call needs its own internal checks; this API cannot
preempt it. Cancellation can leave a partial document in the backend, so discard
that serialization attempt. Backend errors retain precedence over cancellation
requested during the failing call.

<!-- cargo-reedme: start -->

<!-- cargo-reedme: info-start

    Do not edit this region by hand
    ===============================

    This region was generated from Rust documentation comments by `cargo-reedme` using this command:

        cargo +nightly reedme --workspace

    for more info: https://github.com/nik-rev/cargo-reedme

cargo-reedme: info-end -->

Prototype types for the format deserializer.

<!-- cargo-reedme: end -->
