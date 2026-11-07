//! `#[derive(Config)]`: the command line flags of a workflow's input type.
//!
//! One flag per named field, in kebab case, with the field's doc comment as its help. `Option<T>` and `Vec<T>` take
//! `T`'s parser; `bool` is a switch (`--verbose`, or `--verbose=false`). `#[config(skip)]` leaves a field without a
//! flag; `#[config(short = 'w')]` and `#[config(long = "..")]` rename it.
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Expr, ExprLit, Fields, GenericArgument, Lit, PathArguments, Type};

pub fn expand(item: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(item)?;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`Config` is derived for a struct with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`Config` is derived for a struct with named fields",
        ));
    };
    let mut flags = Vec::new();
    for field in &fields.named {
        let Some(ident) = &field.ident else { continue };
        let mut skip = false;
        let mut short: Option<char> = None;
        let mut long: Option<String> = None;
        let mut key = ident.to_string().trim_start_matches("r#").to_string();
        let mut help = Vec::new();
        for attr in &field.attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(nv) = &attr.meta
                    && let Expr::Lit(ExprLit {
                        lit: Lit::Str(text),
                        ..
                    }) = &nv.value
                {
                    help.push(text.value().trim().to_string());
                }
            } else if attr.path().is_ident("config") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("skip") {
                        skip = true;
                    } else if meta.path.is_ident("short") {
                        short = Some(meta.value()?.parse::<syn::LitChar>()?.value());
                    } else if meta.path.is_ident("long") {
                        long = Some(meta.value()?.parse::<syn::LitStr>()?.value());
                    } else {
                        return Err(meta.error("expected `skip`, `short = 'c'` or `long = \"..\"`"));
                    }
                    Ok(())
                })?;
            } else if attr.path().is_ident("serde") {
                // `rename` changes the key the value is stored under; `skip` leaves it out altogether.
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        key = meta.value()?.parse::<syn::LitStr>()?.value();
                    } else if meta.path.is_ident("skip") {
                        skip = true;
                    } else if meta.input.peek(syn::Token![=]) {
                        let _: Expr = meta.value()?.parse()?;
                    }
                    Ok(())
                });
            }
        }
        if skip {
            continue;
        }
        let long = long.unwrap_or_else(|| key.replace('_', "-"));
        let (element, is_list) = element(&field.ty);
        let is_bool = matches!(element, Type::Path(p) if p.path.is_ident("bool"));
        let kind = if is_list {
            quote!(::jackdaw::config::FlagKind::List)
        } else if is_bool {
            quote!(::jackdaw::config::FlagKind::Switch)
        } else {
            quote!(::jackdaw::config::FlagKind::Value)
        };
        let short = short.map_or_else(
            || quote!(::core::option::Option::None),
            |c| quote!(::core::option::Option::Some(#c)),
        );
        let help = help.join(" ");
        flags.push(quote! {
            ::jackdaw::config::Flag {
                key: #key,
                long: #long,
                short: #short,
                help: #help,
                kind: #kind,
                parse: ::jackdaw::config::parse_value::<#element>,
            }
        });
    }
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::jackdaw::config::Config for #ident #ty_generics #where_clause {
            fn flags() -> ::std::vec::Vec<::jackdaw::config::Flag> {
                ::std::vec![#(#flags),*]
            }
        }
    })
}

/// `T` for `T`, `Option<T>`, `Vec<T>` and `Option<Vec<T>>`, and whether it is a list.
fn element(ty: &Type) -> (&Type, bool) {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        if segment.ident == "Option" {
            let (inner, list) = element(inner);
            return (inner, list);
        }
        if segment.ident == "Vec" {
            return (inner, true);
        }
    }
    (ty, false)
}
