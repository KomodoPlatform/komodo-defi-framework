use serde_json::Value as Json;
use std::fmt::Display;

pub trait Printer {
    fn print_response(&self, result: Json) -> Result<(), ()>;
    fn display_response<T: Display + 'static>(&self, result: T) -> Result<(), ()>;
}

pub struct TablePrinter {}

impl Printer for TablePrinter {
    fn print_response(&self, result: Json) -> Result<(), ()> {
        println!("Print result as table");
        Ok(())
    }

    fn display_response<T: Display + 'static>(&self, result: T) -> Result<(), ()> { todo!() }
}
