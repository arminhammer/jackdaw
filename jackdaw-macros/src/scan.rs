//! A pass over the tokens of a `#[run]` body.
//!
//! It does three jobs in one walk, working on tokens rather than a syntax tree so that code inside macro calls
//! (`futures::try_join!(ctx.call(..))`, `jackdaw::select! { .. }`) is seen too:
//! 1. the determinism lint: calls that would behave differently on replay are compile errors, with the fix in the message;
//! 2. dependency discovery: `ctx.call(Shop::charge, ..)` and `ctx.run(Child::run, ..)` name the tasks and child workflows
//!    a workflow needs, so the engine can check them when it is built and register children automatically;
//! 3. stripping `#[allow_nondeterminism]`, the per-statement opt-out, which rustc would not understand.
use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};

#[derive(Default)]
pub struct Scan {
    pub errors: Vec<syn::Error>,
    /// Paths such as `Shop::charge`, `Greet` or `Fulfil::run`, as written in the body.
    pub dependencies: Vec<TokenStream>,
}

/// The per-statement opt-out. It is a plain attribute rather than `#[allow(jackdaw::..)]` because rustc rejects an
/// unknown tool in `allow` before any attribute macro gets to remove it.
const OPT_OUT: &str = "allow_nondeterminism";

const NONDETERMINISM: &str =
    "audited code can opt out with `#[allow_nondeterminism]` on the statement";

/// (a path that must appear as consecutive segments, why it is forbidden and what to use instead)
const RULES: &[(&[&str], &str)] = &[
    (
        &["SystemTime", "now"],
        "the wall clock differs on every replay; use `ctx.now()`",
    ),
    (
        &["Instant", "now"],
        "the clock differs on every replay; use `ctx.now()`",
    ),
    (
        &["Utc", "now"],
        "the wall clock differs on every replay; use `ctx.now()`",
    ),
    (
        &["Local", "now"],
        "the wall clock differs on every replay; use `ctx.now()`",
    ),
    (
        &["Uuid", "new_v4"],
        "random ids differ on every replay; use `ctx.uuid()`",
    ),
    (
        &["Uuid", "new_v7"],
        "generated ids differ on every replay; use `ctx.uuid()`",
    ),
    (
        &["rand"],
        "randomness differs on every replay; use `ctx.random()`",
    ),
    (
        &["thread_rng"],
        "randomness differs on every replay; use `ctx.random()`",
    ),
    (
        &["tokio", "spawn"],
        "a spawned task escapes the journal; put the work in a `#[task]` and `ctx.call` it",
    ),
    (
        &["spawn_blocking"],
        "work outside the journal is not replayed; put it in a `#[task]` and `ctx.call` it",
    ),
    (
        &["spawn_local"],
        "a spawned task escapes the journal; put the work in a `#[task]` and `ctx.call` it",
    ),
    (
        &["tokio", "time", "sleep"],
        "a real sleep is not durable; use `ctx.sleep(duration)`",
    ),
    (
        &["tokio", "time", "timeout"],
        "use `.timeout(duration)` on the step instead",
    ),
    (
        &["thread", "sleep"],
        "a real sleep is not durable; use `ctx.sleep(duration)`",
    ),
    (
        &["tokio", "select"],
        "`tokio::select!` races without recording the winner; use `jackdaw::select!`",
    ),
    (
        &["std", "fs"],
        "file I/O can return something different on replay; put it in a `#[task]`",
    ),
    (
        &["std", "net"],
        "network I/O can return something different on replay; put it in a `#[task]`",
    ),
    (
        &["std", "env"],
        "the environment can differ between runs; read it in a `#[task]`",
    ),
    (
        &["tokio", "fs"],
        "file I/O can return something different on replay; put it in a `#[task]`",
    ),
    (
        &["tokio", "net"],
        "network I/O can return something different on replay; put it in a `#[task]`",
    ),
    (
        &["reqwest"],
        "HTTP calls can return something different on replay; put them in a `#[task]`",
    ),
];

/// Scans `tokens`, a `#[run]` body whose context parameter is called `ctx`, and returns the tokens to compile.
pub fn scan(tokens: TokenStream, ctx: &str, scan: &mut Scan) -> TokenStream {
    walk(tokens, ctx, scan, false)
}

fn walk(tokens: TokenStream, ctx: &str, scan: &mut Scan, mut audited: bool) -> TokenStream {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::with_capacity(trees.len());
    let mut i = 0;
    while let Some(tree) = trees.get(i) {
        // `#[allow_nondeterminism]` applies to the next statement or block, and is removed from the output.
        if is_punct(tree, '#')
            && let Some(TokenTree::Group(group)) = trees.get(i + 1)
            && group.delimiter() == Delimiter::Bracket
            && group.stream().to_string().trim() == OPT_OUT
        {
            audited = true;
            i += 2;
            continue;
        }
        match tree {
            TokenTree::Group(group) => {
                let inner = walk(group.stream(), ctx, scan, audited);
                let mut rebuilt = Group::new(group.delimiter(), inner);
                rebuilt.set_span(group.span());
                out.push(TokenTree::Group(rebuilt));
                if audited && group.delimiter() == Delimiter::Brace {
                    audited = false;
                }
            }
            TokenTree::Punct(punct) if punct.as_char() == ';' => {
                audited = false;
                out.push(tree.clone());
            }
            TokenTree::Ident(ident) => {
                if ident == ctx {
                    record_dependency(&trees, i, scan);
                }
                if !audited {
                    check_path(&trees, i, scan);
                }
                out.push(tree.clone());
            }
            TokenTree::Punct(_) | TokenTree::Literal(_) => out.push(tree.clone()),
        }
        i += 1;
    }
    out.into_iter().collect()
}

fn is_punct(tree: &TokenTree, ch: char) -> bool {
    matches!(tree, TokenTree::Punct(punct) if punct.as_char() == ch)
}

fn punct_at(trees: &[TokenTree], index: usize, ch: char) -> bool {
    trees.get(index).is_some_and(|tree| is_punct(tree, ch))
}

/// Looks for `ctx.call(PATH, ..)`, `ctx.run(PATH, ..)` and `ctx.for_each(items, PATH)`.
fn record_dependency(trees: &[TokenTree], i: usize, scan: &mut Scan) {
    if !punct_at(trees, i + 1, '.') {
        return;
    }
    let Some(TokenTree::Ident(method)) = trees.get(i + 2) else {
        return;
    };
    let Some(TokenTree::Group(args)) = trees.get(i + 3) else {
        return;
    };
    if args.delimiter() != Delimiter::Parenthesis {
        return;
    }
    let wanted = match method.to_string().as_str() {
        "call" | "run" => 0,
        "for_each" => 1,
        _ => return,
    };
    let Some(arg) = split_top_level(args.stream()).into_iter().nth(wanted) else {
        return;
    };
    if is_marker_path(&arg) {
        scan.dependencies.push(arg.into_iter().collect());
    }
}

/// `Shop::charge`, `Greet`, `v1::OrderFlow::run`: identifiers joined by `::`, and not a plain lowercase variable.
fn is_marker_path(tokens: &[TokenTree]) -> bool {
    let mut idents = 0;
    for (position, tree) in tokens.iter().enumerate() {
        let expects_ident = position % 3 == 0;
        let ok = match tree {
            TokenTree::Ident(_) => {
                idents += 1;
                expects_ident
            }
            TokenTree::Punct(punct) => punct.as_char() == ':' && !expects_ident,
            TokenTree::Group(_) | TokenTree::Literal(_) => false,
        };
        if !ok {
            return false;
        }
    }
    let Some(TokenTree::Ident(first)) = tokens.first() else {
        return false;
    };
    let has_path = idents > 1;
    let starts_upper = first
        .to_string()
        .chars()
        .next()
        .is_some_and(char::is_uppercase);
    has_path || starts_upper
}

fn split_top_level(stream: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for tree in stream {
        if is_punct(&tree, ',') {
            parts.push(Vec::new());
        } else if let Some(last) = parts.last_mut() {
            last.push(tree);
        }
    }
    parts
}

/// If the identifier at `i` starts a path (`std::time::SystemTime::now`), checks it against the forbidden list.
fn check_path(trees: &[TokenTree], i: usize, scan: &mut Scan) {
    if i >= 2 && punct_at(trees, i - 1, ':') && punct_at(trees, i - 2, ':') {
        return; // not the start of the path
    }
    let mut segments: Vec<String> = Vec::new();
    let mut j = i;
    while let Some(TokenTree::Ident(ident)) = trees.get(j) {
        segments.push(ident.to_string());
        if punct_at(trees, j + 1, ':') && punct_at(trees, j + 2, ':') {
            j += 3;
        } else {
            break;
        }
    }
    for (pattern, advice) in RULES {
        if contains_run(&segments, pattern) {
            let span = trees.get(i).map_or_else(Span::call_site, TokenTree::span);
            scan.errors.push(syn::Error::new(
                span,
                format!(
                    "`{}` is not allowed in a workflow's `#[run]`: {advice} ({NONDETERMINISM})",
                    segments.join("::")
                ),
            ));
            return;
        }
    }
}

/// Whether `pattern` appears in `segments` as consecutive elements.
fn contains_run(segments: &[String], pattern: &[&str]) -> bool {
    if pattern.is_empty() || segments.len() < pattern.len() {
        return false;
    }
    segments.windows(pattern.len()).any(|window| {
        window
            .iter()
            .zip(pattern)
            .all(|(segment, wanted)| segment == wanted)
    })
}
