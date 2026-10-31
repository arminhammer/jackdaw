# jackdaw-macros

Procedural macros for [jackdaw](https://crates.io/crates/jackdaw), the durable workflow library: `#[task]`, `#[tasks]`,
`#[workflow]`, `#[workflow_methods]` and `#[catalog]`.

Don't depend on this crate directly. Depend on `jackdaw` and use the macros through `jackdaw::prelude`; the code they
generate refers to `jackdaw`, and the two crates are released together with the same version.
