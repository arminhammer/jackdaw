//! Parsing of attribute arguments such as `#[task(name = "x", cached)]`.
use proc_macro2::TokenStream;
use syn::meta::ParseNestedMeta;
use syn::parse::Parser;
use syn::{Attribute, LitStr, Meta, Result};

#[derive(Default)]
pub struct Args {
    pub name: Option<LitStr>,
    pub namespace: Option<LitStr>,
    pub version: Option<LitStr>,
    pub replay: Option<LitStr>,
    pub source: Option<LitStr>,
    pub cached: bool,
    pub blocking: bool,
}

impl Args {
    pub fn parse(tokens: TokenStream) -> Result<Self> {
        let mut args = Self::default();
        let parser = syn::meta::parser(|meta: ParseNestedMeta| {
            if meta.path.is_ident("name") {
                args.name = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("namespace") {
                args.namespace = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("version") {
                args.version = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("replay") {
                args.replay = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("source") {
                args.source = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("cached") {
                args.cached = true;
            } else if meta.path.is_ident("blocking") {
                args.blocking = true;
            } else {
                return Err(meta.error("unsupported argument"));
            }
            Ok(())
        });
        parser.parse2(tokens)?;
        Ok(args)
    }

    /// Arguments of an already-parsed attribute, such as the inner `#[task(cached)]` of a `#[tasks]` impl.
    pub fn from_attr(attr: &Attribute) -> Result<Self> {
        match &attr.meta {
            Meta::Path(_) => Ok(Self::default()),
            Meta::List(list) => Self::parse(list.tokens.clone()),
            Meta::NameValue(_) => Err(syn::Error::new_spanned(
                attr,
                "expected `#[name]` or `#[name(args)]`",
            )),
        }
    }
}
