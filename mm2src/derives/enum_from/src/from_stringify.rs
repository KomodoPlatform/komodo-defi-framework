use crate::{get_attr_meta, CompileError, IdentCtx, MacroAttr};
use proc_macro2::{Ident, Span, TokenStream};
use quote::__private::ext::RepToTokensExt;
use quote::{quote, ToTokens};
use syn::__private::TokenStream2;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token::Colon2;
use syn::NestedMeta::Lit;
use syn::{NestedMeta, Path, PathSegment, Variant};

impl CompileError {
    /// This error constructor is involved to be used on `EnumFromStringify` macro.
    fn expected_literal_inner() -> CompileError {
        CompileError(format!(
            "'{}' attribute must consist of string Literal. For example, #[from_stringify(\"String\")]",
            MacroAttr::FromStringify,
        ))
    }
}

fn check_inner_ident_type(ident: Option<Ident>) -> Result<(), CompileError> {
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

    /// Try to get an Ident name from an attribute value.
    fn try_from(attr: NestedMeta) -> Result<Self, Self::Error> {
        match attr {
            Lit(lit) => Ok(Self(lit.into_token_stream())),
            _ => Err(CompileError::expected_literal_inner()),
        }
    }
}

fn parse_inner_path_id(ident: String, span: Span) -> Result<Path, CompileError> {
    let ident = ident
        .strip_suffix('\"')
        .ok_or("")
        .map_err(|_| CompileError::expected_string_inner_ident(MacroAttr::FromStringify))?;
    let ident = ident
        .strip_prefix('\"')
        .ok_or("")
        .map_err(|_| CompileError::expected_string_inner_ident(MacroAttr::FromStringify))?;

    Ok(Path {
        leading_colon: None,
        segments: ident
            .split("::")
            .map(|s| PathSegment {
                ident: Ident::new(s, span),
                arguments: Default::default(),
            })
            .collect::<Punctuated<PathSegment, Colon2>>(),
    })
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
        let attr_path_id = parse_inner_path_id(token.to_string(), token.span())?;
        check_inner_ident_type(inner_ident.clone())?;

        stream.extend(quote! {
            impl From<#attr_path_id> for #enum_name {
                fn from(err: #attr_path_id) -> #enum_name {
                    #enum_name::#variant_ident(err.to_string())
                }
            }
        })
    }

    Ok(Some(stream))
}
