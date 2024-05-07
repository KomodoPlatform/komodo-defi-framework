use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{parse::Parser, parse_quote, punctuated::Punctuated, spanned::Spanned, Error, Field, Fields, ItemEnum,
          ItemStruct, Meta, Token, Type};

pub(crate) trait IteratorExt {
    fn collect_error(self) -> Result<(), Error>
    where
        Self: Iterator<Item = Result<(), Error>> + Sized,
    {
        let accu = Ok(());
        self.fold(accu, |accu, error| match (accu, error) {
            (Ok(()), error) => error,
            (accu, Ok(())) => accu,
            (Err(mut err), Err(error)) => {
                err.combine(error);
                Err(err)
            },
        })
    }
}
impl<I> IteratorExt for I where I: Iterator<Item = Result<(), Error>> + Sized {}

#[proc_macro_attribute]
pub fn skip_serializing_none(_args: TokenStream, input: TokenStream) -> TokenStream {
    let res = match apply_function_to_struct_and_enum_fields(input, skip_serializing_none_add_attr_to_field) {
        Ok(res) => res,
        Err(err) => err.to_compile_error(),
    };
    TokenStream::from(res)
}

fn apply_function_to_struct_and_enum_fields<F>(input: TokenStream, function: F) -> Result<TokenStream2, Error>
where
    F: Copy,
    F: Fn(&mut Field) -> Result<(), String>,
{
    fn apply_on_fields<F>(fields: &mut Fields, function: F) -> Result<(), Error>
    where
        F: Fn(&mut Field) -> Result<(), String>,
    {
        match fields {
            Fields::Unit => Ok(()),
            Fields::Named(ref mut fields) => fields
                .named
                .iter_mut()
                .map(|field| function(field).map_err(|err| Error::new(field.span(), err)))
                .collect_error(),
            Fields::Unnamed(ref mut fields) => fields
                .unnamed
                .iter_mut()
                .map(|field| function(field).map_err(|err| Error::new(field.span(), err)))
                .collect_error(),
        }
    }
    if let Ok(mut input) = syn::parse::<ItemStruct>(input.clone()) {
        apply_on_fields(&mut input.fields, function)?;
        Ok(quote!(#input))
    } else if let Ok(mut input) = syn::parse::<ItemEnum>(input) {
        input
            .variants
            .iter_mut()
            .map(|variant| apply_on_fields(&mut variant.fields, function))
            .collect_error()?;
        Ok(quote!(#input))
    } else {
        Err(Error::new(
            Span::call_site(),
            "The attribute can only be applied to struct or enum definitions.",
        ))
    }
}

fn is_serialize_always(attr: &syn::Attribute) -> Result<bool, String> {
    attr.parse_meta()
        .map_err(|e| e.to_string())
        .map(|meta| meta.path().is_ident("serialize_always"))
}

fn skip_serializing_none_add_attr_to_field(field: &mut Field) -> Result<(), String> {
    if !is_std_option(&field.ty) {
        for attr in field.attrs.iter() {
            if is_serialize_always(attr)? {
                return Err("`serialize_always` may only be used on fields of type `Option`.".into());
            }
        }
        return Ok(());
    }
    let has_skip_serializing_if = field_has_attribute(field, "serde", "skip_serializing_if");
    let mut has_always_attr = false;
    let mut attrs_to_retain = Vec::new();
    for attr in &field.attrs {
        let has_attr = is_serialize_always(attr)?;
        has_always_attr |= has_attr;
        if !has_attr {
            attrs_to_retain.push(attr.clone());
        }
    }
    field.attrs = attrs_to_retain;
    if has_always_attr && has_skip_serializing_if {
        let mut msg = r#"The attributes `serialize_always` and `serde(skip_serializing_if = "...")` cannot be used on the same field"#.to_string();
        if let Some(ident) = &field.ident {
            msg += ": `";
            msg += &ident.to_string();
            msg += "`";
        }
        msg += ".";
        return Err(msg);
    }
    if !has_skip_serializing_if && !has_always_attr {
        field.attrs.push(parse_quote!(
            #[serde(skip_serializing_if = "Option::is_none")]
        ));
    }
    Ok(())
}

fn is_std_option(type_: &Type) -> bool {
    match type_ {
        Type::Array(_)
        | Type::BareFn(_)
        | Type::ImplTrait(_)
        | Type::Infer(_)
        | Type::Macro(_)
        | Type::Never(_)
        | Type::Ptr(_)
        | Type::Reference(_)
        | Type::Slice(_)
        | Type::TraitObject(_)
        | Type::Tuple(_)
        | Type::Verbatim(_) => false,

        Type::Group(syn::TypeGroup { elem, .. })
        | Type::Paren(syn::TypeParen { elem, .. })
        | Type::Path(syn::TypePath {
            qself: Some(syn::QSelf { ty: elem, .. }),
            ..
        }) => is_std_option(elem),

        Type::Path(syn::TypePath { qself: None, path }) => {
            (path.leading_colon.is_none() && path.segments.len() == 1 && path.segments[0].ident == "Option")
                || (path.segments.len() == 3
                    && (path.segments[0].ident == "std" || path.segments[0].ident == "core")
                    && path.segments[1].ident == "option"
                    && path.segments[2].ident == "Option")
        },
        _ => false,
    }
}

fn field_has_attribute(field: &Field, namespace: &str, name: &str) -> bool {
    for attr in &field.attrs {
        if let Ok(meta) = attr.parse_meta() {
            if meta.path().is_ident(namespace) {
                if let Ok(Meta::List(expr)) = &attr.parse_meta() {
                    let nested = match Punctuated::<Meta, Token![,]>::parse_terminated.parse2(expr.to_token_stream()) {
                        Ok(nested) => nested,
                        Err(_) => continue,
                    };
                    for expr in nested {
                        match expr {
                            Meta::NameValue(expr) => {
                                if let Some(ident) = expr.path.get_ident() {
                                    if *ident == name {
                                        return true;
                                    }
                                }
                            },
                            Meta::Path(expr) => {
                                if let Some(ident) = expr.get_ident() {
                                    if *ident == name {
                                        return true;
                                    }
                                }
                            },
                            _ => (),
                        }
                    }
                }
            }
        }
    }
    false
}
