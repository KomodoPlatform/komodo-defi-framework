use crate::from_trait::get_attr_meta;
use crate::{CompileError, IdentCtx, MacroAttr};
use proc_macro2::{Ident, TokenStream};
use quote::__private::ext::RepToTokensExt;
use quote::{quote, ToTokens};
use syn::__private::TokenStream2;
use syn::spanned::Spanned;
use syn::NestedMeta::Lit;
use syn::{NestedMeta, Variant};

#[derive(Debug)]
pub(crate) enum InnerIdentTypes {
    String,
    Named,
    None,
}

pub(crate) fn get_inner_ident_type(ident: Option<Ident>) -> InnerIdentTypes {
    if let Some(ident) = ident {
        return match ident.to_string().as_str() {
            "String" => InnerIdentTypes::String,
            _ => InnerIdentTypes::Named,
        };
    };

    InnerIdentTypes::None
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

fn parse_inner_ident(token: &TokenStream) -> Result<Ident, CompileError> {
    let ident_to_impl_from = token.to_string();

    let strip_prefix = ident_to_impl_from.strip_prefix('\"').unwrap();
    let strip_suffix = strip_prefix.strip_suffix('\"').unwrap();

    if strip_suffix.is_empty() {
        return Err(CompileError::expected_an_ident(MacroAttr::FromStringify));
    }

    let to_ident = Ident::new(strip_suffix, token.span());
    Ok(to_ident)
}

/// The `#[from_stringify(..)]` attribute value.
struct AttrIdentToken(TokenStream);

impl TryFrom<NestedMeta> for AttrIdentToken {
    type Error = CompileError;

    /// Try to get a trait name and the method from an attribute value `Trait::method`.
    fn try_from(attr: NestedMeta) -> Result<Self, Self::Error> {
        match attr {
            Lit(lit) => Ok(Self(lit.to_token_stream())),
            _ => Err(CompileError::expected_trait_method_path()),
        }
    }
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
        let attr_ident = parse_inner_ident(&token)?;

        match get_inner_ident_type(inner_ident.clone()) {
            InnerIdentTypes::Named => stream.extend(quote! {
                impl From<#attr_ident> for #enum_name {
                    fn from(err: #attr_ident) -> #enum_name {
                        #enum_name::#variant_ident(err)
                    }
                }
            }),
            _ => stream.extend(quote! {
                impl From<#attr_ident> for #enum_name {
                    fn from(err: #attr_ident) -> #enum_name {
                        #enum_name::#variant_ident(err.to_string())
                    }
                }
            }),
        };
    }
    Ok(Some(stream))
}
