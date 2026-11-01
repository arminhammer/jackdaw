//! Small helpers shared by the macros.
use syn::{
    Attribute, FnArg, GenericArgument, Ident, PatType, PathArguments, ReturnType, Signature, Type,
};

/// `fake_http` -> `FakeHttp`
pub fn camel_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// The last path segment of a type, e.g. `TaskContext` for `jackdaw::TaskContext`.
pub fn last_segment(ty: &Type) -> Option<&syn::PathSegment> {
    if let Type::Path(path) = ty {
        path.path.segments.last()
    } else {
        None
    }
}

pub fn is_named(ty: &Type, name: &str) -> bool {
    last_segment(ty).is_some_and(|segment| segment.ident == name)
}

/// The first generic argument of `Result<Out, E>` / `WorkflowResult<Out>` in a return type.
pub fn ok_type(output: &ReturnType, wrappers: &[&str]) -> syn::Result<Type> {
    if let ReturnType::Type(_, ty) = output
        && let Some(segment) = last_segment(ty)
        && wrappers.iter().any(|wrapper| segment.ident == wrapper)
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(ok)) = args.args.first()
    {
        return Ok(ok.clone());
    }
    Err(syn::Error::new_spanned(
        output,
        format!(
            "expected a return type of the form `{}<Output, ..>`",
            wrappers.first().copied().unwrap_or("Result")
        ),
    ))
}

/// Removes and returns the first attribute with the given single-segment name.
pub fn take_attr(attrs: &mut Vec<Attribute>, name: &str) -> Option<Attribute> {
    let position = attrs.iter().position(|attr| attr.path().is_ident(name))?;
    Some(attrs.remove(position))
}

/// The typed (non-receiver) parameters of a signature.
pub fn typed_params(sig: &Signature) -> Vec<&PatType> {
    sig.inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(typed) => Some(typed),
            FnArg::Receiver(_) => None,
        })
        .collect()
}

pub fn has_receiver(sig: &Signature) -> bool {
    sig.inputs
        .iter()
        .any(|arg| matches!(arg, FnArg::Receiver(_)))
}

/// `r#type` is not something we expect, but identifiers coming from users should still be copied verbatim.
pub fn hidden(prefix: &str, ident: &Ident) -> Ident {
    Ident::new(&format!("__jackdaw_{prefix}_{ident}"), ident.span())
}
