use crate::{CompileError, IdentCtx, MacroAttr};
use proc_macro2::{Ident, TokenStream};
use quote::__private::ext::RepToTokensExt;
use quote::{quote, ToTokens};
use syn::__private::TokenStream2;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token::Comma;
use syn::Variant;

#[derive(Clone)]
pub struct MapEnumDataPunctuated {
    variant_ident: Ident,
    nested_meta: Punctuated<syn::NestedMeta, Comma>,
    inner_ident: Option<Ident>,
}

#[derive(Clone)]
pub struct MapEnumData {
    pub(crate) variant_ident: Ident,
    pub(crate) meta: syn::NestedMeta,
    pub(crate) inner_ident: Option<Ident>,
}

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

pub(crate) fn get_attributes(variants: Variant) -> Result<MapEnumDataPunctuated, CompileError> {
    let variant_ident = &variants.ident;
    let fields = &variants.fields;

    for attribute in variants.attrs {
        if let Ok(meta) = attribute.parse_meta() {
            return match meta {
                syn::Meta::List(syn::MetaList { nested, .. }) => {
                    return Ok(MapEnumDataPunctuated {
                        variant_ident: variant_ident.to_owned(),
                        nested_meta: nested,
                        inner_ident: get_variant_unnamed_ident(fields.to_owned()),
                    });
                },
                _ => Err(CompileError::expected_enum_from_stringify(
                    MacroAttr::FromStringify,
                    &attribute.tokens.to_string(),
                )),
            };
        };
    }
    Err(CompileError::expected_enum_from_stringify(
        MacroAttr::FromStringify,
        &variant_ident.to_string(),
    ))
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

pub(crate) fn map_enum_data_from_variant(variant: &Variant) -> Result<Vec<MapEnumData>, CompileError> {
    let mut meta_vec = vec![];
    let attr = get_attributes(variant.to_owned())?;
    for meta in attr.nested_meta {
        let variant_ident = attr.variant_ident.to_owned();
        meta_vec.push(MapEnumData {
            variant_ident,
            meta: meta.clone(),
            inner_ident: attr.inner_ident.clone(),
        });
    }
    Ok(meta_vec)
}

fn parse_inner_ident(attr: &TokenStream) -> Result<Ident, CompileError> {
    let ident_to_impl_from = attr.to_string();
    if ident_to_impl_from.is_empty() {
        return Err(CompileError::expected_an_ident(MacroAttr::FromStringify));
    }
    let strip_prefix = ident_to_impl_from.strip_prefix('\"').unwrap();
    let strip_suffix = strip_prefix.strip_suffix('\"').unwrap();

    let to_ident = Ident::new(strip_suffix, attr.span());
    Ok(to_ident)
}

pub(crate) fn impl_from_stringify(ctx: &IdentCtx<'_>, variant: &Variant) -> Result<Option<TokenStream2>, CompileError> {
    let enum_data = map_enum_data_from_variant(variant)?;
    let enum_name = &ctx.ident;
    if let Some(m) = enum_data.get(0) {
        let variant_ident = &m.variant_ident;
        let ident_to_impl_from = parse_inner_ident(&m.meta.to_token_stream())?;

        return match get_inner_ident_type(m.inner_ident.to_owned()) {
            InnerIdentTypes::Named => Ok(Some(quote! {
                impl From<#ident_to_impl_from> for #enum_name {
                    fn from(err: #ident_to_impl_from) -> #enum_name {
                        #enum_name::#variant_ident(err)
                    }
                }
            })),
            _ => Ok(Some(quote! {
                impl From<#ident_to_impl_from> for #enum_name {
                    fn from(err: #ident_to_impl_from) -> #enum_name {
                        #enum_name::#variant_ident(err.to_string())
                    }
                }
            })),
        };
    }

    Ok(None)
}
