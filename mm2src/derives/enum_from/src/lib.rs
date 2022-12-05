//use crate::from_variant::{get_inner_ident_type, map_enum_data_from_variant, InnerIdentTypes};
use proc_macro::{self, TokenStream};
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::quote;
use std::fmt;
use syn::Variant;
use syn::{parse_macro_input, Data, DeriveInput, Error, Field, Fields, ImplGenerics, Type, TypeGenerics, WhereClause};

mod from_inner;
mod from_stringify;
mod from_trait;

const MACRO_IDENT: &str = "EnumFromInner";

/// Implements `From<Inner>` trait for the given enumeration.
///
/// # Usage
///
/// ```rust
/// use enum_from::EnumFromInner;
///
/// #[derive(EnumFromInner)]
/// enum FooBar {
///     #[from_inner]
///     Foo(i32),
///     #[from_inner]
///     Bar(&'static str),
/// }
///
/// match FooBar::from(10i32) {
///     FooBar::Foo(10) => (),
///     _ => panic!(),
/// }
/// match FooBar::from("Hello, world") {
///     FooBar::Bar("Hello, world") => (),
///     _ => panic!(),
/// }
/// ```
#[proc_macro_derive(EnumFromInner, attributes(from_inner))]
pub fn enum_from_inner(input: TokenStream) -> TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    match derive_enum_from_macro(input, MacroAttr::FromInner) {
        Ok(output) => output,
        Err(e) => e.into(),
    }
}

/// Implements `From<Inner>` trait for the given enumeration.
///
/// # Usage
///
/// ```rust
/// use enum_from::EnumFromTrait;
///
/// #[derive(EnumFromTrait)]
/// enum FooBar {
///     #[from_trait(Foo::foo)]
///     Foo(i32),
///     #[from_trait(Bar::bar)]
///     Bar(&'static str),
/// }
///
/// trait Foo {
///     fn foo(num: i32) -> Self;
/// }
///
/// trait Bar {
///     fn bar(str: &'static str) -> Self;
/// }
///
/// match FooBar::foo(10) {
///     FooBar::Foo(10) => (),
///     _ => panic!(),
/// }
/// match FooBar::bar("Hello, world") {
///     FooBar::Bar("Hello, world") => (),
///     _ => panic!(),
/// }
/// ```
#[proc_macro_derive(EnumFromTrait, attributes(from_trait))]
pub fn enum_from_trait(input: TokenStream) -> TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    match derive_enum_from_macro(input, MacroAttr::FromTrait) {
        Ok(output) => output,
        Err(e) => e.into(),
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy)]
enum MacroAttr {
    /// `from_inner` attribute of the `EnumFromInner` derive macro.
    FromInner,
    /// `from_trait` attribute of the `EnumFromTrait` derive macro.
    FromTrait,
    /// `from_trait` attribute of the `EnumFromTrait` derive macro.
    FromStringify,
}

impl fmt::Display for MacroAttr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MacroAttr::FromInner => write!(f, "from_inner"),
            MacroAttr::FromTrait => write!(f, "from_trait"),
            MacroAttr::FromStringify => write!(f, "from_stringify"),
        }
    }
}

struct CompileError(String);

impl CompileError {
    fn expected_enum(found: &str) -> CompileError {
        CompileError(format!("'{}' cannot be implement for a {}", MACRO_IDENT, found))
    }

    fn expected_unnamed_inner(attr: MacroAttr) -> CompileError {
        CompileError(format!(
            "'{}' attribute must be used for a variant with one unnamed inner type",
            attr
        ))
    }

    fn expected_one_attr_on_variant(attr: MacroAttr) -> CompileError {
        CompileError(format!("An enum variant can have only one '{}' attribute", attr))
    }

    fn attr_must_be_used(attr: MacroAttr) -> CompileError {
        CompileError(format!("'{}' must be used at least once", attr))
    }
}

impl From<CompileError> for TokenStream {
    fn from(e: CompileError) -> Self { TokenStream2::from(e).into() }
}

impl From<CompileError> for TokenStream2 {
    fn from(e: CompileError) -> Self { Error::new(Span::call_site(), e.0).to_compile_error() }
}

/// An information about the derive ident.
struct IdentCtx<'a> {
    ident: &'a Ident,
    impl_generics: ImplGenerics<'a>,
    type_generics: TypeGenerics<'a>,
    where_clause: Option<&'a WhereClause>,
}

impl<'a> From<&'a DeriveInput> for IdentCtx<'a> {
    fn from(input: &'a DeriveInput) -> Self {
        let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
        IdentCtx {
            ident: &input.ident,
            impl_generics,
            type_generics,
            where_clause,
        }
    }
}

/// Represents an unnamed inner field aka `new type`.
struct UnnamedInnerField<'a> {
    field: &'a Field,
}

impl<'a> UnnamedInnerField<'a> {
    /// Try to get an unnamed inner field of the given `variant`.
    /// `attr` is an attribute identifier that is used to generate correct error message.
    fn try_from_variant(variant: &'a Variant, attr: MacroAttr) -> Result<Self, CompileError> {
        match variant.fields {
            Fields::Unnamed(ref fields) if fields.unnamed.len() == 1 => Ok(UnnamedInnerField {
                field: &fields.unnamed[0],
            }),
            _ => Err(CompileError::expected_unnamed_inner(attr)),
        }
    }

    /// Get a type of the field.
    fn ty(&self) -> &Type { &self.field.ty }
}

/// An implementation of `EnumFromInner` and `EnumFromTrait` macros.
fn derive_enum_from_macro(input: DeriveInput, attr: MacroAttr) -> Result<TokenStream, CompileError> {
    let enumeration = match input.data {
        Data::Enum(ref enumeration) => enumeration,
        Data::Struct(_) => return Err(CompileError::expected_enum("struct")),
        Data::Union(_) => return Err(CompileError::expected_enum("union")),
    };

    let ctx = IdentCtx::from(&input);

    let mut impls = Vec::with_capacity(enumeration.variants.len());
    for variant in enumeration.variants.iter() {
        let maybe_impl = match attr {
            MacroAttr::FromInner => from_inner::impl_from_inner(&ctx, variant)?,
            MacroAttr::FromTrait => from_trait::impl_from_trait(&ctx, variant)?,
            MacroAttr::FromStringify => from_stringify::impl_from_stringify(&ctx, variant)?,
        };
        if let Some(variant_impl) = maybe_impl {
            impls.push(variant_impl);
        }
    }

    if impls.is_empty() {
        return Err(CompileError::attr_must_be_used(attr));
    }

    let output = quote! {
        #(#impls)*
    };

    Ok(wrap_const(output))
}

fn wrap_const(code: TokenStream2) -> TokenStream {
    let output = quote! {
        const _: () = {
            #code
        };
    };
    output.into()
}

///! Rust Derive Impl from enum
///
///
/// `EnumFromStringify` is very useful for generating `From<T>` trait from one enum to another enum
/// Currently, this crate can only convert enum variant with only some basic inner type such as `String`, and `Enum`
/// type just like the example below. Can not be used for tuple, struct etc for now .
///
/// More support will be added soon
///
///
/// ### USAGE:
/// ```rust
/// use enum_from::EnumFromVariant;
/// use derive_more::Display;
///
/// // E.G, this converts from whatever Bar is to FooBar::Bar(String) and
/// // whatever Foor to FooBar::Foo(Foo)
/// #[derive(Debug, EnumFromStringify, PartialEq, Eq)]
/// pub enum FooBar {
///     #[from_stringify("Bar")]
///     Bar(String),
///     #[from_stringify("Foo")]
///     Foo(Foo),
/// }
///
/// #[derive(Debug, Display, PartialEq, Eq)]
/// pub enum Bar {
///     Bar(String),
/// }
///
/// #[derive(Debug, Clone, Display, PartialEq, Eq)]
/// pub enum Foo {
///     Foo(String),
/// }
///
/// #[test]
/// fn test_from_variant() {
///     let bar = Bar::Bar("Bar".to_string());
///     assert_eq!(FooBar::Bar("Bar".to_string()), bar.into());
///
///     let foo = Foo::Foo("Foo".to_string());
///     assert_eq!(FooBar::Foo(foo.clone()), foo.into());
/// }
///  ```
#[proc_macro_derive(EnumFromStringify, attributes(from_stringify))]
pub fn derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    match derive_enum_from_macro(ast, MacroAttr::FromStringify) {
        Ok(output) => output,
        Err(e) => e.into(),
    }
}
