use crate::{CompileError, IdentCtx};
use proc_macro2::{Ident, TokenStream};
use quote::__private::ext::RepToTokensExt;
use quote::{quote, quote_spanned};
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
    ident
        .map(|ident| {
            if ident == Ident::new("String", ident.span()) {
                InnerIdentTypes::String
            } else {
                InnerIdentTypes::Named
            }
        })
        .unwrap_or(InnerIdentTypes::None)
}

pub(crate) fn get_attributes(variants: Variant) -> Result<MapEnumDataPunctuated, TokenStream> {
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
                _ => Err(quote_spanned!(
                attribute.tokens.span() => compile_error!("expected #[enum_from_stringify(..)]")
                )),
            };
        };
    }
    Err(quote_spanned!(
    variant_ident.span() => compile_error!("expected #[enum_from_stringify(..)]")
    ))
}

fn get_variant_unnamed_ident(fields: syn::Fields) -> Option<Ident> {
    if let syn::Fields::Unnamed(syn::FieldsUnnamed { unnamed, .. }) = fields {
        if let Some(field) = unnamed.iter().next() {
            let type_path = if let Some(syn::Type::Path(type_path, ..)) = field.ty.next().cloned() {
                type_path
            } else {
                return None;
            };
            return Some(type_path.path.segments.iter().next().cloned()?.ident);
        };
    }
    None
}

pub(crate) fn map_enum_data_from_variant(variant: &Variant) -> Vec<MapEnumData> {
    let mut meta_vec = vec![];
    let _ = get_attributes(variant.to_owned()).map(|attr| {
        for meta in attr.nested_meta.iter() {
            let variant_ident = attr.clone().variant_ident.to_owned();
            meta_vec.push(MapEnumData {
                variant_ident,
                meta: meta.clone(),
                inner_ident: attr.inner_ident.clone(),
            });
        }
    });
    meta_vec
}

pub(crate) fn impl_from_stringify(ctx: &IdentCtx<'_>, variant: &Variant) -> Result<Option<TokenStream2>, CompileError> {
    let enum_data = map_enum_data_from_variant(variant);
    let enum_name = &ctx.ident;
    let construct_meta = enum_data.iter().map(|m| {
        let variant_ident = &m.variant_ident;
        if let syn::NestedMeta::Lit(syn::Lit::Str(str)) = &m.meta {
            if str.value().is_empty() {
                return Some(quote_spanned!(
                str.span() => compile_error!("Expected this to be an `Ident`")
                ));
            };

            let ident_to_impl_from = Ident::new(&str.value(), str.span());
            return match get_inner_ident_type(m.inner_ident.to_owned()) {
                InnerIdentTypes::Named => Some(quote! {
                    impl From<#ident_to_impl_from> for #enum_name {
                        fn from(err: #ident_to_impl_from) -> #enum_name {
                            #enum_name::#variant_ident(err)
                        }
                    }
                }),
                _ => Some(quote! {
                    impl From<#ident_to_impl_from> for #enum_name {
                        fn from(err: #ident_to_impl_from) -> #enum_name {
                            #enum_name::#variant_ident(err.to_string())
                        }
                    }
                }),
            };
        }
        None
    });

    Ok(Some(quote!(#(#construct_meta)*)))
}
