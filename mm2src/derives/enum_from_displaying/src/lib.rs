extern crate quote;

use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::__private::ext::RepToTokensExt;
use quote::{quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{parse_macro_input, DeriveInput};

#[derive(Debug, Clone)]
pub(crate) struct MapEnumNestedIter {
    variant_ident: Ident,
    nested_meta: Punctuated<syn::NestedMeta, Comma>,
    inner_ident: Option<Ident>,
}

#[derive(Debug, Clone)]
pub(crate) struct MapEnumMeta {
    pub(crate) variant_ident: Ident,
    pub(crate) meta: syn::NestedMeta,
    pub(crate) inner_ident: Option<Ident>,
}

#[derive(Debug)]
pub(crate) enum InnerIdentTypes {
    String,
    Named,
    None,
    // TODO
}

pub(crate) fn get_inner_ident(ident: Option<Ident>) -> InnerIdentTypes {
    if let Some(ident) = ident {
        let n = Ident::new("String", ident.span());
        return if ident == n {
            InnerIdentTypes::String
        } else {
            InnerIdentTypes::Named
        };
    }
    InnerIdentTypes::None
}

pub(crate) fn get_attributes(variants: syn::Variant) -> Result<MapEnumNestedIter, syn::Error> {
    let variant_ident = &variants.ident;
    let fields = &variants.fields;
    let attr = &variants.attrs;

    for attr in attr {
        if let Ok(meta) = attr.parse_meta() {
            match meta {
                syn::Meta::List(syn::MetaList { nested, .. }) => {
                    if let Some(ident) = get_variant_unamed_ident(fields.to_owned()) {
                        return syn::Result::Ok(MapEnumNestedIter {
                            variant_ident: variant_ident.to_owned(),
                            nested_meta: nested,
                            inner_ident: Some(ident),
                        });
                    } else {
                        return syn::Result::Ok(MapEnumNestedIter {
                            variant_ident: variant_ident.to_owned(),
                            nested_meta: nested,
                            inner_ident: None,
                        });
                    };
                },
                _ => {
                    return syn::Result::Err(syn::Error::new_spanned(
                        attr.clone().tokens,
                        "expected #[enum_from_displaying(..)]".to_string(),
                    ));
                },
            };
        };
    }
    syn::Result::Err(syn::Error::new_spanned(
        variant_ident.to_token_stream(),
        "Operation Error.".to_string(),
    ))
}

pub(crate) fn get_variant_unamed_ident(fields: syn::Fields) -> Option<Ident> {
    if let syn::Fields::Unnamed(fields_unnamed) = fields {
        let syn::FieldsUnnamed { unnamed, .. } = fields_unnamed;
        if let Some(field) = unnamed.iter().next() {
            let type_path = if let Some(syn::Type::Path(type_path, ..)) = field.ty.next().cloned() {
                type_path
            } else {
                return None;
            };
            let path_segment = type_path.path.segments.iter().next().cloned()?;
            return Some(path_segment.ident);
        };
    }
    None
}

pub(crate) fn map_field_meta(variants: Punctuated<syn::Variant, Comma>) -> Vec<MapEnumMeta> {
    let mut meta_vec = vec![];
    for variant in variants.iter() {
        let _ = get_attributes(variant.to_owned()).map(|attr| {
            for meta in attr.nested_meta.iter() {
                let variant_ident = attr.clone().variant_ident.to_owned();
                meta_vec.push(MapEnumMeta {
                    variant_ident,
                    meta: meta.clone(),
                    inner_ident: attr.inner_ident.clone(),
                });
            }
        });
    }
    meta_vec
}

///
///
///  `EnumFromDisplaying` can be very useful for generating `From<T>` trait for enum variants
/// Currently, this crate can only convert enum variant with only some basic inner type such as `String`, Another enum
/// type just like the example below. Can not be used for tuple, struct etc for now .
///
/// More support will be added soon
///
///
/// ### USAGE:
/// ```rust
/// use derive_impl_macro::EnumFromDisplaying;
/// use derive_more::Display;
///
///  // E.G, this converts from whatever Bar is to Foo::Bar(String) and
/// // whatever FooBar to Foo::FooBar(FooBar)
/// #[derive(Debug, EnumFromDisplaying)]
/// pub enum Foo {
///     #[enum_from_displaying("Bar")]
///     Bar(String),
///     #[enum_from_displaying("FooBar")]
///     FooBar(FooBar),
/// }
///
/// #[derive(Debug, Display)]
/// pub enum Bar {
///     Foo(String),
/// }
///
/// #[derive(Debug, Display)]
/// pub enum FooBar {
///     Foo(String),
/// }
///
/// fn foo_fn() -> Result<(), Foo> {
///     Ok(bar_fn()?)
/// }
///
/// fn bar_fn() -> Result<(), Bar> {
///     Err(Bar::Foo("Err".to_string()))
/// }
/// ```
///
///
///
#[proc_macro_derive(EnumFromDisplaying, attributes(enum_from_displaying))]
pub fn derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let variants = if let syn::Data::Enum(syn::DataEnum { variants, .. }) = ast.data {
        variants
    } else {
        panic!("Couldn't fetch variants")
    };

    let construct_meta_binding = map_field_meta(variants);
    let construct_meta = construct_meta_binding.iter().map(|mm| {
        let variant_ident = &mm.variant_ident;

        if let syn::NestedMeta::Lit(syn::Lit::Str(str)) = &mm.meta {
            let lit_ident = syn::Ident::new(&str.value(), str.span());
            let inner_ident = get_inner_ident(mm.inner_ident.to_owned());
            match inner_ident {
                InnerIdentTypes::Named => {
                    return Some(quote! {
                        impl From<#lit_ident> for #name {
                            fn from(err: #lit_ident) -> #name {
                                #name::#variant_ident(err)
                            }
                        }
                    });
                },
                _ => {
                    return Some(quote! {
                        impl From<#lit_ident> for #name {
                            fn from(err: #lit_ident) -> #name {
                                #name::#variant_ident(err.to_string())
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
