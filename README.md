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

The publishing branch is `teamy-main`. This workspace prepares these packages
as one compatible `0.50.0-rc.7` family:

| Published package | Rust library name |
| --- | --- |
| `teamy-facet-format` | `facet_format` |
| `teamy-facet-dessert` | `facet_dessert` |
| `teamy-facet-json` | `facet_json` |
| `teamy-facet-value` | `facet_value` |

Use Cargo package aliases to keep the original Rust import names:

```toml
[dependencies]
facet = { package = "teamy-facet", version = "=0.50.0-rc.7" }
facet-format = { package = "teamy-facet-format", version = "=0.50.0-rc.7" }
facet-dessert = { package = "teamy-facet-dessert", version = "=0.50.0-rc.7" }
facet-json = { package = "teamy-facet-json", version = "=0.50.0-rc.7" }
facet-value = { package = "teamy-facet-value", version = "=0.50.0-rc.7" }
```

All public Facet types must come from the same Teamy core family. The fork
packages depend on `teamy-facet-*` through exact versions, without relying on
a consumer's `[patch.crates-io]`. Select only the format packages you need.

For local development, clone [TeamDman/facet](https://github.com/TeamDman/facet)
on `teamy-main` beside this repository as `../facet`. Versioned path
dependencies use that checkout locally and become registry dependencies in
Cargo's published manifests. Publish the Teamy core family first, then
`teamy-facet-dessert`, `teamy-facet-format`, and finally
`teamy-facet-json` and `teamy-facet-value`.

The complete library, integration, and documentation suites run from these
source checkouts. Internal development dependencies use path-only package aliases
so Cargo omits them from published manifests; this avoids publication cycles
between the core, Format, and JSON packages. Registry archive validation builds
the published libraries. Full source suites remain available in the forks.

Other workspace members keep their upstream package names for local development
and validation. They are excluded from this fork's release automation.
Publishing is manual through the owner-guarded workflow; pushing commits does
not trigger a registry release. Initial publication must wait until all required
Teamy core versions are available on crates.io.

The original licenses are preserved in [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE), and copied unchanged into each fork package.
