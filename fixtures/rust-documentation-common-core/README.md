# Rust documentation common-core probe

Public synthetic Rust 2021 crate. Select
`cargo:Cargo.toml#documentation_probe#lib#documentation_probe` with `rust-syntax`.

`render`, `Request` and `limits` exercise exact retained function signatures,
type/module declaration text, guards, locals, explicit returns and function-tail
values through the common reader. Calls remain unresolved syntax; matching a
helper name does not establish a compiler-selected callee. `opaque` keeps the
question-mark control boundary explicit. `discarded` prevents nested block
values from being mislabeled as returns from the function.

The source deliberately starts with BOM plus same-line `pub fn`, and uses CRLF,
Unicode and inert markup. This fixture does not qualify every Cargo project.
