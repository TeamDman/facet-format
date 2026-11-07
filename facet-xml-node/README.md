# teamy-facet-xml-node

> **Unofficial fork:** `teamy-facet-xml-node` is maintained by
> [TeamDman](https://github.com/TeamDman/facet-format), independently of facet-rs.
> It is not an official upstream release. The implementation derives from
> [official Facet Format at `4279debff780ae1cd5b028201f446c26594b1120`](https://github.com/facet-rs/facet-format/tree/4279debff780ae1cd5b028201f446c26594b1120).
> Original attribution and the [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)
> licenses are preserved. Fork issues belong in
> [TeamDman/facet-format](https://github.com/TeamDman/facet-format/issues).

Install with the original Rust import name and the matching Teamy facade:

```toml
[dependencies]
facet = { package = "teamy-facet", version = "=0.50.0-rc.8" }
facet-xml-node = { package = "teamy-facet-xml-node", version = "=0.50.0-rc.8" }
```

The Rust library name remains `facet_xml_node`. The Teamy graph retains
Core/Reflect `0.50.0-rc.7`; official and Teamy Facet types have distinct
Cargo identities. See the [fork release overview](https://github.com/TeamDman/facet-format/blob/teamy-main/README.md)
for compatible packages and publication details.

This fork also includes a bounded, namespace-aware read model that preserves
original source bytes, ordered content, and source ranges. Validate extracted
values before constructing write requests. The upstream API overview follows.

[![crates.io](https://img.shields.io/crates/v/teamy-facet-xml-node.svg)](https://crates.io/crates/teamy-facet-xml-node)
[![documentation](https://docs.rs/teamy-facet-xml-node/badge.svg)](https://docs.rs/teamy-facet-xml-node)
[![MIT/Apache-2.0 licensed](https://img.shields.io/crates/l/teamy-facet-xml-node.svg)](./LICENSE-MIT)
[![Upstream Discord](https://img.shields.io/discord/1379550208551026748?logo=discord&label=discord)](https://discord.gg/JhD7CwCJ8F)

Raw XML node types—represent arbitrary XML without a schema.

## Overview

This crate provides generic XML types (`Element`, `Content`) that can represent
any XML document without needing predefined Rust structs. It's useful when you
need to parse XML dynamically or work with XML of unknown structure.

## Types

### Element

Captures any XML element with its tag name, attributes, and children:

```rust
use facet_xml_node::Element;

let xml = r#"<item id="42" status="active">Hello <b>world</b></item>"#;
let element: Element = facet_xml::from_str(xml)?;

assert_eq!(element.tag, "item");
assert_eq!(element.attrs.get("id"), Some(&"42".to_string()));
```

### Content

Represents either text or a child element:

```rust
use facet_xml_node::{Element, Content};

for child in &element.children {
    match child {
        Content::Text(t) => println!("Text: {}", t),
        Content::Element(e) => println!("Element: <{}>", e.tag),
    }
}
```

## Use Cases

- Parsing XML of unknown or variable structure
- Building XML transformers or validators
- Bridging between typed and untyped XML representations
- Testing and debugging XML serialization

## Comparison

| Approach | Use Case |
|----------|----------|
| Typed structs with `#[derive(Facet)]` | Known XML schema, compile-time safety |
| `facet-xml-node::Element` | Unknown/dynamic XML, runtime flexibility |

## Part of the Facet Ecosystem

This crate is part of the [facet](https://facet.rs) ecosystem, providing reflection for Rust.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/facet-rs/facet/blob/main/LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](https://github.com/facet-rs/facet/blob/main/LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
