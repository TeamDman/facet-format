# Teamy Facet format forks

These are **unofficial forks** maintained independently by TeamDman.
The `teamy-facet-*` packages are not official facet-rs releases. Upstream owns
the original implementation and project; its authors retain their attribution
and the MIT OR Apache-2.0 licensing.

The source is based on official
[`facet-rs/facet-format` at `4279debff780ae1cd5b028201f446c26594b1120`](https://github.com/facet-rs/facet-format/tree/4279debff780ae1cd5b028201f446c26594b1120).
The reviewed fork integration at
[`a1ca5f97253eb49fb080a39e32377dcebb0499f5`](https://github.com/TeamDman/facet-format/tree/a1ca5f97253eb49fb080a39e32377dcebb0499f5)
fixes the error span for an invalid scalar enum variant. Official development
continues in the [Facet monorepo](https://github.com/facet-rs/facet).
Report Teamy packaging or fork issues to
[TeamDman/facet-format](https://github.com/TeamDman/facet-format/issues).

The publishing branch is `teamy-main`. Current release preparation updates
Format, JSON, Value, and CSV to `0.50.0-rc.10`, alongside the first Teamy DOM/XML
packages. Dessert remains `0.50.0-rc.7`. The graph uses the Facet facade and
macro fix at `0.50.0-rc.8` with unchanged Core/Reflect packages at
`0.50.0-rc.7`.

Generic `Cow<T>` decoding now enters the smart-pointer pointee before decoding,
so validating proxies on `T` are applied correctly. Existing borrowed string
and byte fast paths are retained. Format traversal still emits fields
incrementally; JSON writes directly to caller-owned writers, whose errors also
provide cooperative cancellation.

The XML packages include the merged strict, bounded, namespace-aware read
model and pin `quick-xml` to `0.41.0`. The read model retains original source
bytes; callers must validate extracted values before constructing write requests.

| Package | Prepared version | Rust library name |
| --- | --- | --- |
| `teamy-facet-format` | `0.50.0-rc.10` | `facet_format` |
| `teamy-facet-dessert` (unchanged) | `0.50.0-rc.7` | `facet_dessert` |
| `teamy-facet-json` | `0.50.0-rc.10` | `facet_json` |
| `teamy-facet-value` | `0.50.0-rc.10` | `facet_value` |
| `teamy-facet-csv` | `0.50.0-rc.10` | `facet_csv` |
| `teamy-facet-dom` (first Teamy release) | `0.50.0-rc.7` | `facet_dom` |
| `teamy-facet-xml` | `0.50.0-rc.8` | `facet_xml` |
| `teamy-facet-xml-node` | `0.50.0-rc.8` | `facet_xml_node` |

Use Cargo package aliases to keep the original Rust import names:

```toml
[dependencies]
facet = { package = "teamy-facet", version = "=0.50.0-rc.8" }
facet-format = { package = "teamy-facet-format", version = "=0.50.0-rc.10" }
facet-dessert = { package = "teamy-facet-dessert", version = "=0.50.0-rc.7" }
facet-json = { package = "teamy-facet-json", version = "=0.50.0-rc.10" }
facet-value = { package = "teamy-facet-value", version = "=0.50.0-rc.10" }
```

All public Facet types must come from the same Teamy core family. The fork
packages depend on `teamy-facet-*` through exact versions, without relying on
a consumer's `[patch.crates-io]`. Select only the format packages you need.

For local development, clone [TeamDman/facet](https://github.com/TeamDman/facet)
on `teamy-main` beside this repository as `../facet`. Versioned path
dependencies use that checkout locally and become registry dependencies in
Cargo's published manifests. Publish the changed Teamy core packages first.
The existing Core/Reflect and Dessert `0.50.0-rc.7` releases remain dependencies;
they do not need another release. Publish `teamy-facet-format` before
`teamy-facet-json`, `teamy-facet-value`, and `teamy-facet-csv`.
For XML, publish `teamy-facet-dom`, then `teamy-facet-xml`, then
`teamy-facet-xml-node`. DOM uses the official `facet-singularize` package for
English singularization; its default features do not enable Facet dependencies.

The complete library, integration, and documentation suites run from these
source checkouts. Internal development dependencies that use path-only package
aliases are omitted from published manifests; this avoids publication cycles
between the core, Format, and JSON packages. Registry archive validation builds
the published libraries. Full source suites remain available in the forks.

Other workspace members keep their upstream package names for local development
and validation. They are excluded from this fork's release automation.
Publishing is manual through the owner-guarded workflow; pushing commits does
not trigger a registry release. Initial publication must wait until all required
Teamy core versions are available on crates.io.

The original licenses are preserved in [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE), and copied unchanged into each fork package.
