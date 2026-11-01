# jackdaw-wf-macros

Procedural macros for [jackdaw-wf](https://crates.io/crates/jackdaw-wf), the durable workflow library: `#[task]`, `#[tasks]`,
`#[workflow]`, `#[workflow_methods]` and `#[catalog]`.

Don't depend on this crate directly. Depend on `jackdaw-wf` and use the macros through `jackdaw::prelude`; the code they
generate refers to `jackdaw`, its library name, and the two crates are released together with the same version.
