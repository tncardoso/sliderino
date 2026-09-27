//! A small XML writer for the parts of the PPTX package.
//!
//! Elements are written in one pass: `start` opens a tag, `attr` adds to
//! it until a child, text or `end` closes the start tag. `end` writes `/>`
//! when the element has no content.

use std::fmt::{Display, Write as _};

pub const DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

pub struct Xml {
    out: String,
    stack: Vec<&'static str>,
    /// The start tag of the top element is not closed yet.
    pending: bool,
}

impl Default for Xml {
    fn default() -> Self {
        Self::new()
    }
}

impl Xml {
    /// A document that starts with the XML declaration.
    pub fn new() -> Self {
        Self {
            out: format!("{DECLARATION}\n"),
            stack: Vec::new(),
            pending: false,
        }
    }

    /// A fragment without the declaration, to embed in another document.
    pub fn fragment() -> Self {
        Self {
            out: String::new(),
            stack: Vec::new(),
            pending: false,
        }
    }

    fn close_start(&mut self) {
        if self.pending {
            self.out.push('>');
            self.pending = false;
        }
    }

    pub fn start(&mut self, name: &'static str) -> &mut Self {
        self.close_start();
        self.out.push('<');
        self.out.push_str(name);
        self.stack.push(name);
        self.pending = true;
        self
    }

    pub fn attr(&mut self, name: &str, value: impl Display) -> &mut Self {
        debug_assert!(self.pending, "attribute {name} after the start tag");
        self.out.push(' ');
        self.out.push_str(name);
        self.out.push_str("=\"");
        let value = value.to_string();
        escape_into(&mut self.out, &value, true);
        self.out.push('"');
        self
    }

    /// Adds the attribute only when `value` is `Some`.
    pub fn attr_opt(&mut self, name: &str, value: Option<impl Display>) -> &mut Self {
        if let Some(value) = value {
            self.attr(name, value);
        }
        self
    }

    pub fn text(&mut self, text: &str) -> &mut Self {
        self.close_start();
        escape_into(&mut self.out, text, false);
        self
    }

    /// Writes already serialized XML as content.
    pub fn raw(&mut self, xml: &str) -> &mut Self {
        self.close_start();
        self.out.push_str(xml);
        self
    }

    pub fn end(&mut self) -> &mut Self {
        let name = self.stack.pop().expect("end without start");
        if self.pending {
            self.out.push_str("/>");
            self.pending = false;
        } else {
            let _ = write!(self.out, "</{name}>");
        }
        self
    }

    /// An element without content: `start`, `end`, with attributes between.
    pub fn empty(&mut self, name: &'static str, attrs: &[(&str, &dyn Display)]) -> &mut Self {
        self.start(name);
        for (key, value) in attrs {
            self.attr(key, value);
        }
        self.end()
    }

    pub fn finish(self) -> String {
        assert!(self.stack.is_empty(), "unclosed element {:?}", self.stack);
        self.out
    }
}

/// Escapes the markup characters, and drops the characters that XML 1.0
/// does not allow (control characters other than tab and line feeds).
fn escape_into(out: &mut String, text: &str, attribute: bool) {
    for char in text.chars() {
        match char {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\t' | '\n' | '\r' if attribute => {
                let _ = write!(out, "&#{};", char as u32);
            }
            '\t' | '\n' | '\r' => out.push(char),
            '\u{0}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}' => {}
            _ => out.push(char),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nests_and_closes_empty_elements() {
        let mut xml = Xml::fragment();
        xml.start("a:p").start("a:r").attr("lang", "en").end().end();
        assert_eq!(xml.finish(), r#"<a:p><a:r lang="en"/></a:p>"#);
    }

    #[test]
    fn escapes_text_and_attributes() {
        let mut xml = Xml::fragment();
        xml.start("t")
            .attr("name", "a \"b\" & <c>\n")
            .text("x < y & z > \"w\"\u{1}")
            .end();
        assert_eq!(
            xml.finish(),
            r#"<t name="a &quot;b&quot; &amp; &lt;c&gt;&#10;">x &lt; y &amp; z &gt; "w"</t>"#
        );
    }

    #[test]
    fn a_document_starts_with_the_declaration() {
        let mut xml = Xml::new();
        xml.empty("root", &[("n", &3)]);
        assert_eq!(xml.finish(), format!("{DECLARATION}\n<root n=\"3\"/>"));
    }
}
