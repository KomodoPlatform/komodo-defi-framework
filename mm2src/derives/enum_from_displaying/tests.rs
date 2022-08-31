use derive_more::Display;
use enum_from_displaying::EnumFromDisplaying;

#[derive(Debug, EnumFromDisplaying)]
pub enum TestError {
    #[enum_from_displaying("TestError2")]
    Foo(String),
    #[enum_from_displaying("TestError3")]
    Bar(TestError3),
}

#[derive(Debug, Display)]
pub enum TestError2 {
    Internal(String),
}

#[derive(Debug, Display)]
pub enum TestError3 {
    Internal(String),
}

#[allow(dead_code)]
fn test_error_2() -> TestError2 { TestError2::Internal("foo".to_string()) }

#[test]
fn test() {
    #[allow(dead_code)]
    fn foo() -> TestError {
        let err = test_error_2();
        err.into()
    }
}
