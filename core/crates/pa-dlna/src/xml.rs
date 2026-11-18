// SPDX-License-Identifier: MPL-2.0
//! A small element tree over `quick-xml`: UPnP answers are small, and
//! their namespaces vary by server, so elements are matched by local name.

use pa_vfs::{ErrorKind, VfsError};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// One element: local name, attributes (local names), text and children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<El>,
}

impl El {
    /// The first child named `name`.
    pub fn child(&self, name: &str) -> Option<&El> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child named `name`.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Trimmed text of the first child named `name`, if not empty.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|c| c.text.trim())
            .filter(|t| !t.is_empty())
    }

    /// The attribute `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The first element named `name` at any depth (this one included).
    pub fn find(&self, name: &str) -> Option<&El> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }
}

fn bad(what: &str, e: impl std::fmt::Display) -> VfsError {
    VfsError::new(ErrorKind::Other, format!("bad {what}: {e}"))
}

fn start(e: &BytesStart<'_>, what: &str) -> Result<El, VfsError> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| bad(what, e))?;
        let key = a.key.local_name().as_ref().to_owned();
        let val = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| bad(what, e))?
            .into_owned();
        attrs.push((key, val));
    }
    Ok(El {
        name: e.local_name().as_ref().to_owned(),
        attrs,
        ..El::default()
    })
}

/// Parses `xml` (the `what` of error messages) into its root element.
pub(crate) fn parse(xml: &str, what: &str) -> Result<El, VfsError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack: Vec<El> = Vec::new();
    loop {
        match reader.read_event().map_err(|e| bad(what, e))? {
            Event::Start(e) => stack.push(start(&e, what)?),
            Event::End(_) => {
                let el = stack.pop().ok_or_else(|| bad(what, "unbalanced"))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => return Ok(el),
                }
            }
            Event::Text(t) => {
                if let Some(el) = stack.last_mut() {
                    el.text.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(el) = stack.last_mut() {
                    el.text.push_str(&t);
                }
            }
            Event::GeneralRef(r) => {
                if let Some(el) = stack.last_mut() {
                    if let Ok(Some(c)) = r.resolve_char_ref() {
                        el.text.push(c);
                    } else {
                        el.text.push_str(match &*r {
                            "amp" => "&",
                            "lt" => "<",
                            "gt" => ">",
                            "quot" => "\"",
                            "apos" => "'",
                            _ => "",
                        });
                    }
                }
            }
            Event::Eof => return Err(bad(what, "no root element")),
            _ => {}
        }
    }
}

/// `text` with the five XML entities escaped (for SOAP arguments).
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaced_trees_with_entities() {
        let el = parse(
            r#"<?xml version="1.0"?><s:Envelope xmlns:s="x"><s:Body><u:R xmlns:u="y"><Result>&lt;a b=&quot;1&quot;&gt;x &amp;amp; y&lt;/a&gt;</Result><N>2</N><e/></u:R></s:Body></s:Envelope>"#,
            "test",
        )
        .unwrap();
        assert_eq!(el.name, "Envelope");
        let r = el.find("R").unwrap();
        assert_eq!(r.text_of("Result"), Some(r#"<a b="1">x &amp; y</a>"#));
        assert_eq!(r.text_of("N"), Some("2"));
        assert!(r.child("e").is_some());
        assert_eq!(escape("a<b&'c\""), "a&lt;b&amp;&apos;c&quot;");
        assert!(parse("", "x").is_err());
        assert!(parse("<a><b></a>", "x").is_err());
    }
}
