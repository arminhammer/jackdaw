//! `#[catalog(source = "..")] trait Local { .. }`: typed bindings to catalog entries.
//!
//! The trait is replaced by a unit struct whose associated consts are the typed markers (`Local::add`), so a catalog
//! entry is used exactly like a local task or workflow. The entry names and versions come from the attributes; the
//! input and output types are the ones declared on the method.
use crate::args::Args;
use crate::util::{ok_type, take_attr, typed_params};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemTrait, TraitItem};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args = Args::parse(attr)?;
    let source = args.source.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "`#[catalog]` needs `source = \"..\"`",
        )
    })?;
    let mut block: ItemTrait = syn::parse2(item)?;
    let ident = &block.ident;
    let vis = &block.vis;

    let mut consts = Vec::new();
    let mut entries = Vec::new();
    for item in &mut block.items {
        let TraitItem::Fn(method) = item else {
            return Err(syn::Error::new_spanned(
                item,
                "a catalog declares only `async fn` entries",
            ));
        };
        let (kind, attr) = if let Some(attr) = take_attr(&mut method.attrs, "task") {
            (Kind::Task, attr)
        } else if let Some(attr) = take_attr(&mut method.attrs, "workflow") {
            (Kind::Workflow, attr)
        } else {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "mark each catalog entry with `#[task(name = \"..\", version = \"..\")]` or `#[workflow(..)]`",
            ));
        };
        let entry_args = Args::from_attr(&attr)?;
        let name = entry_args.name.ok_or_else(|| {
            syn::Error::new_spanned(&attr, "a catalog entry needs `name = \"..\"`")
        })?;
        let version = entry_args.version.ok_or_else(|| {
            syn::Error::new_spanned(&attr, "a catalog entry needs `version = \"..\"`")
        })?;
        let params = typed_params(&method.sig);
        let [input] = params.as_slice() else {
            return Err(syn::Error::new_spanned(
                &method.sig.inputs,
                "a catalog entry takes exactly one input",
            ));
        };
        let input = (*input.ty).clone();
        let output = ok_type(&method.sig.output, &["Result", "WorkflowResult"])?;
        let function = &method.sig.ident;

        let (reference, entry) = match kind {
            Kind::Task => (
                quote!(::jackdaw::__private::CatalogTaskRef),
                quote!(::jackdaw::__private::CatalogEntry::task::<#input, #output>(#name, #version)),
            ),
            Kind::Workflow => (
                quote!(::jackdaw::__private::CatalogWorkflowRef),
                quote!(::jackdaw::__private::CatalogEntry::workflow::<#input, #output>(#name, #version)),
            ),
        };
        consts.push(quote! {
            #[allow(non_upper_case_globals)]
            #vis const #function: #reference<#input, #output> = #reference::new(#name, #version);
        });
        entries.push(entry);
    }

    Ok(quote! {
        #[derive(Debug, Clone, Copy)]
        #vis struct #ident;

        impl #ident {
            #(#consts)*
        }

        impl ::jackdaw::__private::CatalogDef for #ident {
            const SOURCE: &'static str = #source;

            fn entries() -> ::std::vec::Vec<::jackdaw::__private::CatalogEntry> {
                ::std::vec![ #(#entries),* ]
            }
        }
    })
}

enum Kind {
    Task,
    Workflow,
}
