//! `#[task]` on a free function and `#[tasks]` on an `impl` block.
//!
//! A free function `greet` keeps its body and gains a unit struct `Greet` (the typed marker). In a `#[tasks]` impl a task
//! method `charge` is renamed to a hidden method and replaced by an associated const `charge`, so `Shop::charge` is the
//! marker, exactly as it reads at call sites.
use crate::args::Args;
use crate::util::{camel_case, has_receiver, hidden, is_named, ok_type, take_attr, typed_params};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Ident, ImplItem, ImplItemFn, ItemFn, ItemImpl, LitStr, Signature, Type};

/// What a task function looks like to the engine.
struct Shape {
    /// Whether the function takes a `TaskContext` before its input.
    context: bool,
    input: Type,
    output: Type,
}

fn shape(sig: &Signature) -> syn::Result<Shape> {
    let params = typed_params(sig);
    let (context, input) = match params.as_slice() {
        [input] => (false, &input.ty),
        [context, input] if is_named(&context.ty, "TaskContext") => (true, &input.ty),
        _ => {
            return Err(syn::Error::new_spanned(
                &sig.inputs,
                "a task takes its input, optionally preceded by a `TaskContext`: `fn(input: In)` or `fn(ctx: TaskContext, input: In)`",
            ));
        }
    };
    Ok(Shape {
        context,
        input: (**input).clone(),
        output: ok_type(&sig.output, &["Result"])?,
    })
}

fn task_name(args: &Args, ident: &Ident) -> LitStr {
    args.name
        .clone()
        .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()))
}

pub fn expand_fn(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args = Args::parse(attr)?;
    let function: ItemFn = syn::parse2(item)?;
    let sig = &function.sig;

    if has_receiver(sig) {
        return Err(syn::Error::new_spanned(
            &sig.inputs,
            "a method is a task only inside a `#[tasks]` impl block",
        ));
    }
    if args.blocking == sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            if args.blocking {
                "`#[task(blocking)]` is for synchronous functions; remove `async` or `blocking`"
            } else {
                "a synchronous function must be marked `#[task(blocking)]`, so it runs off the async runtime's threads"
            },
        ));
    }

    let shape = shape(sig)?;
    let name = task_name(&args, &sig.ident);
    let cached = args.cached;
    let vis = &function.vis;
    let function_ident = &sig.ident;
    let marker = Ident::new(
        &camel_case(&function_ident.to_string()),
        function_ident.span(),
    );
    let (input, output) = (&shape.input, &shape.output);

    let ctx_param = if shape.context {
        quote!(ctx)
    } else {
        quote!(_ctx)
    };
    let call = if shape.context {
        quote!(#function_ident(ctx, input))
    } else {
        quote!(#function_ident(input))
    };
    let native = if args.blocking {
        quote! {
            ::jackdaw::api::NativeTask::blocking::<#input, #output, _>(#name, #cached, |#ctx_param, input| #call)
        }
    } else {
        quote! {
            ::jackdaw::api::NativeTask::new::<#input, #output, _, _>(
                #name, #cached, |#ctx_param, input| async move { #call.await },
            )
        }
    };
    let doc = format!(
        "Typed marker for the `{name}` task: pass it to `ctx.call(..)` or `Engine::task(..)`.",
        name = name.value()
    );

    Ok(quote! {
        #function

        #[doc = #doc]
        #[derive(Debug, Clone, Copy)]
        #vis struct #marker;

        impl ::jackdaw::api::TaskDef for #marker {
            type In = #input;
            type Out = #output;
            fn task_name(&self) -> &'static str { #name }
            fn cacheable(&self) -> bool { #cached }
        }

        impl ::jackdaw::api::RegisterTask for #marker {
            fn native(self) -> ::jackdaw::api::NativeTask {
                #native
            }
        }
    })
}

pub fn expand_impl(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[tasks]` takes no arguments",
        ));
    }
    let mut block: ItemImpl = syn::parse2(item)?;
    if block.trait_.is_some() {
        return Err(syn::Error::new_spanned(
            &block,
            "`#[tasks]` goes on an inherent `impl` block",
        ));
    }
    let self_ty = (*block.self_ty).clone();
    let (impl_generics, _, where_clause) = block.generics.split_for_impl();
    let impl_generics = quote!(#impl_generics);
    let where_clause = quote!(#where_clause);

    let mut consts = Vec::new();
    let mut registrations = Vec::new();
    let mut rewritten: Vec<ImplItem> = Vec::new();

    for item in std::mem::take(&mut block.items) {
        let ImplItem::Fn(mut method) = item else {
            rewritten.push(item);
            continue;
        };
        let Some(task_attr) = take_attr(&mut method.attrs, "task") else {
            rewritten.push(ImplItem::Fn(method));
            continue;
        };
        let args = Args::from_attr(&task_attr)?;
        if args.blocking {
            return Err(syn::Error::new_spanned(
                &task_attr,
                "`blocking` tasks must be free functions",
            ));
        }
        if method.sig.asyncness.is_none() {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "a task method must be `async`",
            ));
        }
        if !has_receiver(&method.sig) {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "a task method takes `self: Arc<Self>` so it can use its state",
            ));
        }
        let shape = shape(&method.sig)?;
        let name = task_name(&args, &method.sig.ident);
        let cached = args.cached;
        let (input, output) = (&shape.input, &shape.output);
        let vis = method.vis.clone();
        let original = method.sig.ident.clone();
        let hidden_ident = hidden("task", &original);

        let invoke = if shape.context {
            quote!(this.#hidden_ident(ctx, input).await)
        } else {
            quote!(this.#hidden_ident(input).await)
        };
        let ctx_param = if shape.context {
            quote!(ctx)
        } else {
            quote!(_ctx)
        };
        registrations.push(quote! {
            {
                let this = ::std::sync::Arc::clone(&self);
                ::jackdaw::api::NativeTask::new::<#input, #output, _, _>(
                    #name, #cached,
                    move |#ctx_param, input| {
                        let this = ::std::sync::Arc::clone(&this);
                        async move { #invoke }
                    },
                )
            }
        });
        consts.push(quote! {
            #[allow(non_upper_case_globals)]
            #vis const #original: ::jackdaw::api::TaskRef<#self_ty, #input, #output> =
                ::jackdaw::api::TaskRef::new(#name, #cached);
        });

        let mut renamed: ImplItemFn = method;
        renamed.sig.ident = hidden_ident;
        renamed.vis = syn::Visibility::Inherited;
        renamed.attrs.push(syn::parse_quote!(#[doc(hidden)]));
        rewritten.push(ImplItem::Fn(renamed));
    }
    block.items = rewritten;

    Ok(quote! {
        #block

        impl #impl_generics #self_ty #where_clause {
            #(#consts)*
        }

        impl #impl_generics ::jackdaw::api::TaskSet for #self_ty #where_clause {
            fn native_tasks(self: ::std::sync::Arc<Self>) -> ::std::vec::Vec<::jackdaw::api::NativeTask> {
                ::std::vec![ #(#registrations),* ]
            }
        }
    })
}
