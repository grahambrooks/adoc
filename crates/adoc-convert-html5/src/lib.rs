//! HTML5 converter for the adoc AsciiDoc toolchain.

use adoc_core::{ConvertError, Converter, Document};

pub struct Html5Converter;

impl Converter for Html5Converter {
    fn convert(&self, _doc: &Document) -> Result<String, ConvertError> {
        Ok(String::from("<!doctype html>\n<html><body></body></html>\n"))
    }
}
