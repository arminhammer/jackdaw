//! `#[workflow]` on a struct and `#[workflow_methods]` on its `impl`.
//!
//! The struct is the workflow's state. In the impl, `#[run]`, `#[signal]`, `#[update]` and `#[query]` methods are renamed to hidden
//! methods and replaced by typed associated consts of the same name, so `OrderWorkflow::run`, `OrderWorkflow::approve`
//! and `OrderWorkflow::status` are the markers used at call sites (a struct with fields cannot be a value itself).
use crate::args::Args;
use crate::scan::{self, Scan};
use crate::util::{has_receiver, hidden, ok_type, take_attr, typed_params};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{Fields, Ident, ImplItem, ItemImpl, ItemStruct, Pat, ReturnType, Type};

pub fn expand_struct(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args = Args::parse(attr)?;
    let item: ItemStruct = syn::parse2(item)?;
    let require = |value: &Option<syn::LitStr>, key: &str| {
        value.clone().ok_or_else(|| {
            syn::Error::new_spanned(&item.ident, format!("`#[workflow]` needs `{key} = \"..\"`"))
        })
    };
    let namespace = require(&args.namespace, "namespace")?;
    let name = require(&args.name, "name")?;
    let version = require(&args.version, "version")?;
    let replay = match args.replay.as_ref().map(syn::LitStr::value).as_deref() {
        None | Some("strict") => quote!(::jackdaw::__private::ReplayMode::Strict),
        Some("lenient") => quote!(::jackdaw::__private::ReplayMode::Lenient),
        Some(_) => {
            return Err(syn::Error::new_spanned(
                args.replay,
                "`replay` must be \"strict\" (the default) or \"lenient\"",
            ));
        }
    };
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "a workflow cannot be generic",
        ));
    }
    let ident = &item.ident;
    let default_impl = match &item.fields {
        Fields::Unit => quote! {
            impl ::core::default::Default for #ident {
                fn default() -> Self { Self }
            }
        },
        Fields::Named(_) => TokenStream::new(),
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                &item.fields,
                "a workflow is a unit struct or a struct with named fields",
            ));
        }
    };

    Ok(quote! {
        #item

        impl ::jackdaw::__private::WorkflowMeta for #ident {
            const NAMESPACE: &'static str = #namespace;
            const NAME: &'static str = #name;
            const VERSION: &'static str = #version;
            const REPLAY: ::jackdaw::__private::ReplayMode = #replay;
        }

        #default_impl
    })
}

struct Run {
    original: Ident,
    hidden: Ident,
    vis: syn::Visibility,
    input: Type,
    output: Type,
}

struct Signal {
    original: Ident,
    hidden: Ident,
    vis: syn::Visibility,
    arg: Type,
}

struct Update {
    original: Ident,
    hidden: Ident,
    vis: syn::Visibility,
    arg: Type,
    output: Type,
}

struct Query {
    original: Ident,
    hidden: Ident,
    vis: syn::Visibility,
    output: Type,
}

fn ident_of(pat: &Pat) -> String {
    if let Pat::Ident(ident) = pat {
        ident.ident.to_string()
    } else {
        "ctx".to_string()
    }
}

pub fn expand_methods(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[workflow_methods]` takes no arguments",
        ));
    }
    let mut block: ItemImpl = syn::parse2(item)?;
    if block.trait_.is_some() {
        return Err(syn::Error::new_spanned(
            &block,
            "`#[workflow_methods]` goes on an inherent `impl` block",
        ));
    }
    let self_ty = (*block.self_ty).clone();

    let mut run: Option<Run> = None;
    let mut init: Option<Ident> = None;
    let mut signals: Vec<Signal> = Vec::new();
    let mut queries: Vec<Query> = Vec::new();
    let mut updates: Vec<Update> = Vec::new();
    let mut scan_state = Scan::default();
    let mut errors: Vec<syn::Error> = Vec::new();

    let mut rewritten: Vec<ImplItem> = Vec::new();
    for item in std::mem::take(&mut block.items) {
        let ImplItem::Fn(mut method) = item else {
            rewritten.push(item);
            continue;
        };

        if take_attr(&mut method.attrs, "init").is_some() {
            init = Some(method.sig.ident.clone());
            rewritten.push(ImplItem::Fn(method));
        } else if take_attr(&mut method.attrs, "run").is_some() {
            let params = typed_params(&method.sig);
            let [ctx, input] = params.as_slice() else {
                errors.push(syn::Error::new_spanned(
                    &method.sig.inputs,
                    "`#[run]` takes `(ctx: &mut WorkflowContext<Self>, input: In)`",
                ));
                continue;
            };
            let (ctx_name, input_ty) = (ident_of(&ctx.pat), (*input.ty).clone());
            let output = match ok_type(&method.sig.output, &["WorkflowResult", "Result"]) {
                Ok(output) => output,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            if method.sig.asyncness.is_none() || has_receiver(&method.sig) {
                errors.push(syn::Error::new_spanned(
                    &method.sig,
                    "`#[run]` is an `async fn` without `self`",
                ));
                continue;
            }
            if run.is_some() {
                errors.push(syn::Error::new_spanned(
                    &method.sig,
                    "a workflow has exactly one `#[run]` method",
                ));
                continue;
            }
            let body = scan::scan(method.block.to_token_stream(), &ctx_name, &mut scan_state);
            match syn::parse2(body) {
                Ok(block) => method.block = block,
                Err(error) => errors.push(error),
            }
            let hidden_ident = hidden("run", &method.sig.ident);
            run = Some(Run {
                original: method.sig.ident.clone(),
                hidden: hidden_ident.clone(),
                vis: method.vis.clone(),
                input: input_ty,
                output,
            });
            method.sig.ident = hidden_ident;
            method.vis = syn::Visibility::Inherited;
            method.attrs.push(syn::parse_quote!(#[doc(hidden)]));
            rewritten.push(ImplItem::Fn(method));
        } else if take_attr(&mut method.attrs, "signal").is_some() {
            let params = typed_params(&method.sig);
            let [_, arg] = params.as_slice() else {
                errors.push(syn::Error::new_spanned(
                    &method.sig.inputs,
                    "`#[signal]` takes `(&mut self, ctx: &mut SyncWorkflowContext<Self>, arg: Arg)`",
                ));
                continue;
            };
            let arg_ty = (*arg.ty).clone();
            let hidden_ident = hidden("signal", &method.sig.ident);
            signals.push(Signal {
                original: method.sig.ident.clone(),
                hidden: hidden_ident.clone(),
                vis: method.vis.clone(),
                arg: arg_ty,
            });
            method.sig.ident = hidden_ident;
            method.vis = syn::Visibility::Inherited;
            method.attrs.push(syn::parse_quote!(#[doc(hidden)]));
            rewritten.push(ImplItem::Fn(method));
        } else if take_attr(&mut method.attrs, "update").is_some() {
            let params = typed_params(&method.sig);
            let [_, arg] = params.as_slice() else {
                errors.push(syn::Error::new_spanned(
                    &method.sig.inputs,
                    "`#[update]` takes `(&mut self, ctx: &mut SyncWorkflowContext<Self>, arg: Arg)`",
                ));
                continue;
            };
            let arg_ty = (*arg.ty).clone();
            let output = match ok_type(&method.sig.output, &["Result"]) {
                Ok(output) => output,
                Err(_) => {
                    errors.push(syn::Error::new_spanned(
                        &method.sig.output,
                        "`#[update]` returns `Result<R, UpdateError>`",
                    ));
                    continue;
                }
            };
            let hidden_ident = hidden("update", &method.sig.ident);
            updates.push(Update {
                original: method.sig.ident.clone(),
                hidden: hidden_ident.clone(),
                vis: method.vis.clone(),
                arg: arg_ty,
                output,
            });
            method.sig.ident = hidden_ident;
            method.vis = syn::Visibility::Inherited;
            method.attrs.push(syn::parse_quote!(#[doc(hidden)]));
            rewritten.push(ImplItem::Fn(method));
        } else if take_attr(&mut method.attrs, "query").is_some() {
            let ReturnType::Type(_, output) = &method.sig.output else {
                errors.push(syn::Error::new_spanned(
                    &method.sig,
                    "`#[query]` returns the value it reads",
                ));
                continue;
            };
            let output = (**output).clone();
            let hidden_ident = hidden("query", &method.sig.ident);
            queries.push(Query {
                original: method.sig.ident.clone(),
                hidden: hidden_ident.clone(),
                vis: method.vis.clone(),
                output,
            });
            method.sig.ident = hidden_ident;
            method.vis = syn::Visibility::Inherited;
            method.attrs.push(syn::parse_quote!(#[doc(hidden)]));
            rewritten.push(ImplItem::Fn(method));
        } else {
            rewritten.push(ImplItem::Fn(method));
        }
    }
    block.items = rewritten;

    errors.append(&mut scan_state.errors);
    let Some(run) = run else {
        let missing = syn::Error::new_spanned(&self_ty, "a workflow needs a `#[run]` method");
        errors.push(missing);
        return Err(combine(errors));
    };
    if !errors.is_empty() {
        return Err(combine(errors));
    }

    let Run {
        original,
        hidden: hidden_run,
        vis,
        input,
        output,
    } = &run;
    let run_const = quote! {
        #[allow(non_upper_case_globals)]
        #vis const #original: ::jackdaw::__private::WorkflowRef<#self_ty, #input, #output> = ::jackdaw::__private::WorkflowRef::new();
    };
    let signal_consts = signals.iter().map(|signal| {
        let (name, vis, arg) = (&signal.original, &signal.vis, &signal.arg);
        let label = name.to_string();
        quote! {
            #[allow(non_upper_case_globals)]
            #vis const #name: ::jackdaw::__private::SignalRef<#self_ty, #arg> = ::jackdaw::__private::SignalRef::new(#label);
        }
    });
    let update_consts = updates.iter().map(|update| {
        let (name, vis, arg, output) = (&update.original, &update.vis, &update.arg, &update.output);
        let label = name.to_string();
        quote! {
            #[allow(non_upper_case_globals)]
            #vis const #name: ::jackdaw::__private::UpdateRef<#self_ty, #arg, #output> = ::jackdaw::__private::UpdateRef::new(#label);
        }
    });
    let update_handlers = updates.iter().map(|update| {
        let (name, hidden, arg, output) = (&update.original, &update.hidden, &update.arg, &update.output);
        let label = name.to_string();
        quote! {
            ::jackdaw::__private::UpdateHandler::new::<#arg, #output, _>(
                #label,
                |state: &mut #self_ty, ctx: &mut ::jackdaw::__private::SyncWorkflowContext<#self_ty>, arg: #arg| {
                    state.#hidden(ctx, arg)
                },
            )
        }
    });
    let query_consts = queries.iter().map(|query| {
        let (name, vis, output) = (&query.original, &query.vis, &query.output);
        let label = name.to_string();
        quote! {
            #[allow(non_upper_case_globals)]
            #vis const #name: ::jackdaw::__private::QueryRef<#self_ty, #output> = ::jackdaw::__private::QueryRef::new(#label);
        }
    });
    let signal_handlers = signals.iter().map(|signal| {
        let (name, hidden, arg) = (&signal.original, &signal.hidden, &signal.arg);
        let label = name.to_string();
        quote! {
            ::jackdaw::__private::SignalHandler::new::<#arg, _>(
                #label,
                |state: &mut #self_ty, ctx: &mut ::jackdaw::__private::SyncWorkflowContext<#self_ty>, arg: #arg| {
                    state.#hidden(ctx, arg)
                },
            )
        }
    });
    let query_handlers = queries.iter().map(|query| {
        let (name, hidden, output) = (&query.original, &query.hidden, &query.output);
        let label = name.to_string();
        quote! {
            ::jackdaw::__private::QueryHandler::new::<#output, _>(
                #label,
                |state: &#self_ty, ctx: &::jackdaw::__private::WorkflowContextView| state.#hidden(ctx),
            )
        }
    });
    let init_expr = match &init {
        Some(init) => quote!(<#self_ty>::#init(ctx)),
        None => quote!(<#self_ty as ::core::default::Default>::default()),
    };
    let dependencies = scan_state
        .dependencies
        .iter()
        .map(|path| quote!(::jackdaw::__private::AsDependency::dependency(&#path)));

    Ok(quote! {
        #block

        impl #self_ty {
            #run_const
            #(#signal_consts)*
            #(#update_consts)*
            #(#query_consts)*
        }

        impl ::jackdaw::__private::WorkflowDef for #self_ty {
            type In = #input;
            type Out = #output;

            fn init(ctx: &::jackdaw::__private::WorkflowContextView) -> Self {
                #init_expr
            }

            fn run<'a>(
                ctx: &'a mut ::jackdaw::__private::WorkflowContext<Self>,
                input: #input,
            ) -> ::jackdaw::__private::BoxFuture<'a, ::jackdaw::__private::WorkflowResult<#output>> {
                ::std::boxed::Box::pin(<#self_ty>::#hidden_run(ctx, input))
            }

            fn signals() -> ::std::vec::Vec<::jackdaw::__private::SignalHandler<Self>> {
                ::std::vec![ #(#signal_handlers),* ]
            }

            fn queries() -> ::std::vec::Vec<::jackdaw::__private::QueryHandler<Self>> {
                ::std::vec![ #(#query_handlers),* ]
            }

            fn updates() -> ::std::vec::Vec<::jackdaw::__private::UpdateHandler<Self>> {
                ::std::vec![ #(#update_handlers),* ]
            }

            fn dependencies() -> ::std::vec::Vec<::jackdaw::__private::Dependency> {
                ::std::vec![ #(#dependencies),* ]
            }
        }
    })
}

fn combine(errors: Vec<syn::Error>) -> syn::Error {
    let mut iter = errors.into_iter();
    let mut first = iter
        .next()
        .unwrap_or_else(|| syn::Error::new(Span::call_site(), "invalid workflow"));
    for error in iter {
        first.combine(error);
    }
    first
}
