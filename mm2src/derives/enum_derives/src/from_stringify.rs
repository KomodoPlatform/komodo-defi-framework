use crate::{get_attr_meta, CompileError, IdentCtx, MacroAttr};
use proc_macro2::{Ident, TokenStream};
use quote::__private::ext::RepToTokensExt;
use quote::quote;
use syn::NestedMeta::Lit;
use syn::__private::TokenStream2;
use syn::{ExprPath, Fields, NestedMeta, Variant};

impl CompileError {
    /// This error constructor is involved to be used on `EnumFromStringify` macro.
    fn expected_literal_inner() -> CompileError {
        CompileError(format!(
            "'{}' attribute must consist of string Literal. For example, #[from_stringify(\"String\")]",
            MacroAttr::FromStringify,
        ))
    }
}

fn check_variant_unnamed_ident(fields: Fields) -> Result<Option<bool>, CompileError> {
    if let Fields::Unnamed(syn::FieldsUnnamed { unnamed, .. }) = fields {
        if unnamed.len() > 1 {
            return Err(CompileError::parsing_error(
                MacroAttr::FromStringify,
                "Cannot automatically infer format for types with more than 1 field".to_string(),
            ));
        }

        if let Some(field) = unnamed.iter().next() {
            if let Some(syn::Type::Path(type_path, ..)) = field.ty.next().cloned() {
                let type_path = match type_path.path.segments.iter().next().cloned() {
                    None => return Ok(None),
                    Some(t) => t.ident,
                };

                if SCALAR_TYPES.contains(&type_path.to_string().as_str()) {
                    return Ok(Some(false));
                };

                return Ok(Some(true));
            };
        };
    }

    Ok(None)
}

/// The `#[from_stringify(..)]` attribute value.
struct AttrIdentToken(ExprPath);

impl TryFrom<NestedMeta> for AttrIdentToken {
    type Error = CompileError;

    /// Try to get an Ident name from an attribute value.
    fn try_from(attr: NestedMeta) -> Result<Self, Self::Error> {
        let path_str = match attr {
            Lit(syn::Lit::Str(lit)) => lit.value(),
            _ => return Err(CompileError::expected_literal_inner()),
        };
        let path_stream = syn::parse_str(&path_str)
            .map_err(|err| CompileError::parsing_error(MacroAttr::FromStringify, err.to_string()))?;
        let path = syn::parse2(path_stream)
            .map_err(|err| CompileError::parsing_error(MacroAttr::FromStringify, err.to_string()))?;

        Ok(AttrIdentToken(path))
    }
}

const SCALAR_TYPES: [&str; 13] = [
    "String", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
];

pub(crate) fn impl_from_stringify(ctx: &IdentCtx<'_>, variant: &Variant) -> Result<Option<TokenStream2>, CompileError> {
    let enum_name = &ctx.ident;
    let variant_ident = &variant.ident;

    let maybe_attr = variant
        .attrs
        .iter()
        .flat_map(|attr| get_attr_meta(attr, MacroAttr::FromStringify))
        .collect::<Vec<_>>();

    let mut stream = TokenStream::new();
    for meta in maybe_attr {
        let AttrIdentToken(attr_path_id) = AttrIdentToken::try_from(meta)?;
        let to_custom = check_variant_unnamed_ident(variant.fields.to_owned())?.unwrap_or_default();

        if to_custom {
            stream.extend(quote! {
                impl From<#attr_path_id> for #enum_name {
                fn from(err: #attr_path_id) -> #enum_name {
                        #enum_name::#variant_ident(err)
                    }
                }
            });
        } else {
            stream.extend(quote! {
                impl From<#attr_path_id> for #enum_name {
                    fn from(err: #attr_path_id) -> #enum_name {
                        #enum_name::#variant_ident(err.to_string())
                    }
                }
            });
        }
    }

    Ok(Some(stream))
}
