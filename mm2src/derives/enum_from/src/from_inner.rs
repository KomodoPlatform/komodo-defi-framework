use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Variant;

use crate::{CompileError, IdentCtx, MacroAttr, UnnamedInnerField};

/// Implement `From<Inner>` trait for the given enumeration `variant`.
pub(crate) fn impl_from_inner(ctx: &IdentCtx<'_>, variant: &Variant) -> Result<Option<TokenStream2>, CompileError> {
    let attr_count = variant
        .attrs
        .iter()
        .filter(|attr| attr.path.is_ident(&MacroAttr::FromInner.to_string()))
        .count();
    match attr_count {
        // There is no `#[from_inner]` attribute.
        0 => return Ok(None),
        1 => (),
        // There are more than one `#[from_inner]` attributes.
        _ => return Err(CompileError::expected_one_attr_on_variant(MacroAttr::FromInner)),
    }

    let inner_field = UnnamedInnerField::try_from_variant(variant, MacroAttr::FromInner)?;
    let inner_type = &inner_field.ty();

    let variant_ident = &variant.ident;
    let IdentCtx {
        ident,
        impl_generics,
        type_generics,
        where_clause,
    } = ctx;

    let output = quote! {
        #[automatically_derived]
        impl #impl_generics From<#inner_type> for #ident #type_generics #where_clause {
            fn from(inner: #inner_type) -> Self {
                Self::#variant_ident(inner)
            }
        }
    };
    Ok(Some(output))
}
