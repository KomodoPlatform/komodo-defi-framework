use crate::{CompileError, IdentCtx, MacroAttr, UnnamedInnerField};
use itertools::Itertools;
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::quote;
use syn::Attribute;
use syn::Meta::{List, Path};
use syn::{NestedMeta, Variant};

impl CompileError {
    /// This error constructor is involved to be used on `EnumFromTrait` macro.
    fn expected_trait_method_path() -> CompileError {
        CompileError(format!(
            "'{}' attribute must consist of two parts: 'Trait::method'. For example, #[{}(Default::default)]",
            MacroAttr::FromTrait,
            MacroAttr::FromTrait
        ))
    }
}

/// Implement a given trait-constructor for the given enumeration `variant`.
pub(crate) fn impl_from_trait(ctx: &IdentCtx<'_>, variant: &Variant) -> Result<Option<TokenStream2>, CompileError> {
    let maybe_attr = variant
        .attrs
        .iter()
        .flat_map(|attr| get_attr_meta(attr, MacroAttr::FromTrait))
        .at_most_one()
        .map_err(|_| CompileError::expected_one_attr_on_variant(MacroAttr::FromTrait))?;
    let attr = match maybe_attr {
        Some(attr) => attr,
        None => return Ok(None),
    };

    let TraitIdentMethod {
        ident: trait_ident,
        method: trait_method,
    } = TraitIdentMethod::try_from(&attr)?;

    let inner_field = UnnamedInnerField::try_from_variant(variant, MacroAttr::FromTrait)?;
    let inner_type = inner_field.ty();

    let variant_ident = &variant.ident;
    let IdentCtx {
        ident,
        impl_generics,
        type_generics,
        where_clause,
    } = ctx;

    let output = quote! {
        #[automatically_derived]
        impl #impl_generics #trait_ident for #ident #type_generics #where_clause {
            fn #trait_method(inner: #inner_type) -> Self {
                Self::#variant_ident(inner)
            }
        }
    };
    Ok(Some(output))
}

/// The `Trait::method` attribute value.
struct TraitIdentMethod<'a> {
    /// The trait name.
    ident: &'a Ident,
    /// The trait method.
    method: &'a Ident,
}

impl<'a> TryFrom<&'a NestedMeta> for TraitIdentMethod<'a> {
    type Error = CompileError;

    /// Try to get a trait name and the method from an attribute value `Trait::method`.
    fn try_from(attr: &'a NestedMeta) -> Result<Self, Self::Error> {
        match attr {
            NestedMeta::Meta(Path(path)) if path.segments.len() == 2 => Ok(TraitIdentMethod {
                ident: &path.segments[0].ident,
                method: &path.segments[1].ident,
            }),
            _ => Err(CompileError::expected_trait_method_path()),
        }
    }
}

/// Get the meta information about the given `attr`.
fn get_attr_meta(attr: &Attribute, attr_ident: MacroAttr) -> Vec<NestedMeta> {
    if !attr.path.is_ident(&attr_ident.to_string()) {
        return Vec::new();
    }

    match attr.parse_meta() {
        // A meta list is like the `serde(tag = "...")` in `#[serde(tag = "...")]`
        // or `serde(untagged)` in `#[serde(untagged)]`
        Ok(List(meta)) => meta.nested.into_iter().collect(),
        _ => Vec::new(),
    }
}
