//! Guarantees that hold at compile time (via `trybuild`): the determinism lint is a hard error with a fix in the
//! message, the types of tasks and workflows are checked, and audited code can opt out.
//!
//! Each case is a small program in `ui/`. The expected compiler output sits next to it as `<case>.stderr`;
//! write or refresh those files with `TRYBUILD=overwrite cargo test -p jackdaw-wf --test rust compile_fail`, then
//! review the diff: the messages are part of the API (each one names the call and the replacement).

#[test]
fn nondeterministic_calls_in_run_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/rust/ui/nondeterministic_*.rs");
}

#[test]
fn mistyped_task_calls_and_outputs_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/rust/ui/mismatched_*.rs");
}

#[test]
fn misused_macros_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/rust/ui/misuse_*.rs");
}

#[test]
fn audited_code_can_opt_out_of_the_lint() {
    trybuild::TestCases::new().pass("tests/rust/ui/pass_*.rs");
}
