use derive_more::Display;
use enum_from::EnumFromStringify;

#[derive(Debug, EnumFromStringify, PartialEq, Eq)]
pub enum FooBar {
    #[from_stringify("Bar")]
    Bar(String),
    #[from_stringify("Foo")]
    Foo(Foo),
}

#[derive(Debug, Display, PartialEq, Eq)]
pub enum Bar {
    Bar(String),
}

#[derive(Debug, Clone, Display, PartialEq, Eq)]
pub enum Foo {
    Foo(String),
}

fn main() {
    let bar = Bar::Bar("Bar".to_string());
    assert_eq!(FooBar::Bar("Bar".to_string()), bar.into());

    let foo = Foo::Foo("Foo".to_string());
    assert_eq!(FooBar::Foo(foo.clone()), foo.into());
}
