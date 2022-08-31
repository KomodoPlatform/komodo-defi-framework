extern crate quote;

use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::__private::ext::RepToTokensExt;
use quote::{quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{parse_macro_input, DeriveInput};

#[derive(Debug, Clone)]
struct MapEnumNestedPath {
    variant_ident: Ident,
    nested_meta: Punctuated<syn::NestedMeta, Comma>,
    inner_ident: Option<Ident>,
}

#[derive(Debug, Clone)]
struct MapEnumMeta {
    variant_ident: Ident,
    meta: syn::NestedMeta,
    inner_ident: Option<Ident>,
}

#[derive(Debug)]
enum VariantInnerIdentTypes {
    String,
    Named,
    None,
    // Todo
}

#[proc_macro_derive(EnumFromDisplaying, attributes(enum_from_displaying))]
pub fn derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let enum_name = &ast.ident;
    let variants = if let syn::Data::Enum(syn::DataEnum { variants, .. }) = ast.data {
        variants
    } else {
        panic!("Couldn't enum fetch variants")
    };

    let variant_unamed_ident = |fields: syn::Fields| {
        if let syn::Fields::Unnamed(unamed) = fields {
            if let Some(field) = unamed.unnamed.iter().next() {
                if let syn::Type::Path(path, ..) = &field.ty {
                    if let Some(path) = &path.next() {
                        if let Some(segment) = path.path.segments.next() {
                            if let Some(path_segment) = segment.iter().next() {
                                return Some(path_segment.ident.to_owned());
                            }
                        }
                    }
                };
            };
        }
        None
    };

    let attributes = variants.iter().map(|v| {
        let variant_ident = &v.ident;
        let fields = &v.fields;
        let attr = &v.attrs;
        if !attr.is_empty() {
            for attr in attr {
                if let Ok(meta) = attr.parse_meta() {
                    match meta {
                        syn::Meta::List(syn::MetaList { nested, .. }) => {
                            if let Some(ident) = variant_unamed_ident(fields.to_owned()) {
                                return syn::Result::Ok(MapEnumNestedPath {
                                    variant_ident: variant_ident.to_owned(),
                                    nested_meta: nested,
                                    inner_ident: Some(ident),
                                });
                            };
                            return syn::Result::Ok(MapEnumNestedPath {
                                variant_ident: variant_ident.to_owned(),
                                nested_meta: nested,
                                inner_ident: None,
                            });
                        },
                        _ => {
                            return syn::Result::Err(syn::Error::new_spanned(
                                attr.clone().tokens,
                                "expected #[enum_from_displaying(..)]".to_string(),
                            ));
                        },
                    };
                }
            }
        }
        syn::Result::Err(syn::Error::new_spanned(
            variant_ident.to_token_stream(),
            "Please provide atleast one attribute to use.".to_string(),
        ))
    });

    let map_to_meta = {
        let mut meta_c = vec![];
        for map_enum_paths in attributes {
            if let Ok(nested) = map_enum_paths.clone() {
                for meta in nested.nested_meta.into_iter() {
                    let variant_ident = map_enum_paths.clone().unwrap().variant_ident;
                    meta_c.push(MapEnumMeta {
                        variant_ident,
                        meta: meta.clone(),
                        inner_ident: nested.inner_ident.clone(),
                    });
                }
            }
        }
        meta_c
    };

    let extract_inner_ident = |ident: Option<Ident>| {
        if let Some(ident) = ident {
            let n = Ident::new("String", ident.span());
            return if ident == n {
                VariantInnerIdentTypes::String
            } else {
                VariantInnerIdentTypes::Named
            };
        }
        VariantInnerIdentTypes::None
    };

    let construct_meta = map_to_meta.iter().map(|mm| {
        let variant_ident = &mm.variant_ident;
        if let syn::NestedMeta::Lit(syn::Lit::Str(str)) = &mm.meta {
            let lit_ident = syn::Ident::new(&str.value(), str.span());
            let ex_ident = extract_inner_ident(mm.inner_ident.to_owned());
            match ex_ident {
                VariantInnerIdentTypes::Named => {
                    return Some(quote! {
                        impl From<#lit_ident> for #enum_name {
                            fn from(err: #lit_ident) -> #enum_name {
                                #enum_name::#variant_ident(err.into())
                            }
                        }
                    });
                },
                _ => {
                    return Some(quote! {
                        impl From<#lit_ident> for #enum_name {
                            fn from(err: #lit_ident) -> #enum_name {
                                #enum_name::#variant_ident(err.to_string())
                            }
                        }
                    })
                },
            }
        }
        None
    });

    quote!(#(#construct_meta)*).into()
}
