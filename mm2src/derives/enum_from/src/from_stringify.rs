use crate::from_trait::get_attr_meta;
use crate::{CompileError, IdentCtx, MacroAttr};
use itertools::Itertools;
use proc_macro2::{Ident, Span, TokenStream};
use quote::__private::ext::RepToTokensExt;
use quote::{quote, ToTokens};
use syn::__private::TokenStream2;
use syn::spanned::Spanned;
use syn::NestedMeta::Lit;
use syn::{NestedMeta, Variant};

pub(crate) fn check_inner_ident_type(ident: Option<Ident>) -> Result<(), CompileError> {
    if let Some(ident) = ident {
        if ident.to_string().as_str() == "String" {
            return Ok(());
        }
    };

    Err(CompileError::expected_string_inner_ident(MacroAttr::FromStringify))
}

fn get_variant_unnamed_ident(fields: syn::Fields) -> Option<Ident> {
    if let syn::Fields::Unnamed(syn::FieldsUnnamed { unnamed, .. }) = fields {
        if let Some(field) = unnamed.iter().next() {
            if let Some(syn::Type::Path(type_path, ..)) = field.ty.next().cloned() {
                let type_path = type_path.path.segments.iter().next().cloned()?.ident;
                return Some(type_path);
            };
        };
    }
    None
}

/// The `#[from_stringify(..)]` attribute value.
struct AttrIdentToken(TokenStream);

impl TryFrom<NestedMeta> for AttrIdentToken {
    type Error = CompileError;

    /// Try to get a trait name and the method from an attribute value `Trait::method`.
    fn try_from(attr: NestedMeta) -> Result<Self, Self::Error> {
        match attr {
            Lit(lit) => Ok(Self(lit.to_token_stream())),
            _ => Err(CompileError::expected_literal_inner()),
        }
    }
}

fn parse_inner_ident(ident: String, span: Span) -> Result<Ident, CompileError> {
    let ident = ident.split("").into_iter().filter(|&e| e != "\"").join("");
    Ok(Ident::new(&ident, span))
}

pub(crate) fn impl_from_stringify(ctx: &IdentCtx<'_>, variant: &Variant) -> Result<Option<TokenStream2>, CompileError> {
    let enum_name = &ctx.ident;
    let variant_ident = &variant.ident;
    let inner_ident = get_variant_unnamed_ident(variant.fields.to_owned());
    let maybe_attr = variant
        .attrs
        .iter()
        .flat_map(|attr| get_attr_meta(attr, MacroAttr::FromStringify))
        .collect::<Vec<_>>();

    let mut stream = TokenStream::new();
    for meta in maybe_attr {
        let AttrIdentToken(token) = AttrIdentToken::try_from(meta)?;
        let attr_ident = parse_inner_ident(token.to_string(), token.span())?;

        match check_inner_ident_type(inner_ident.clone()) {
            Ok(_) => stream.extend(quote! {
                impl From<#attr_ident> for #enum_name {
                    fn from(err: #attr_ident) -> #enum_name {
                        #enum_name::#variant_ident(err)
                    }
                }
            }),
            Err(err) => return Err(err),
        };
    }
    Ok(Some(stream))
}

#[test]
fn test_ident() {
    let span = Span::call_site();
    let new = parse_inner_ident("\"Sami\"".to_string(), span).ok().unwrap();
    let expected = Ident::new("Sami", span);

    assert_eq!(new, expected)
}
