// SPDX-License-Identifier: MPL-2.0
//! A bounded element tree over `quick-xml` for WS-Discovery messages:
//! elements are matched by local name (namespaces vary by sender), depth
//! and element count are capped because the input is a datagram or a
//! small HTTP answer from any device on the LAN.

use quick_xml::Reader;
use quick_xml::events::Event;

/// Deepest nesting accepted.
const MAX_DEPTH: usize = 32;
/// Most elements accepted in one document.
const MAX_ELEMENTS: usize = 4096;

/// One element: local name, text and children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct El {
    pub name: String,
    pub text: String,
    pub children: Vec<El>,
}

impl El {
    /// The first element named `name` at any depth (this one included).
    pub fn find(&self, name: &str) -> Option<&El> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }

    /// Every element named `name` at any depth, in document order.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        if self.name == name {
            out.push(self);
        }
        for c in &self.children {
            c.find_all(name, out);
        }
    }

    /// Trimmed text of the first element named `name` at any depth, if
    /// not empty.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.find(name)
            .map(|c| c.text.trim())
            .filter(|t| !t.is_empty())
    }
}

/// Parses `xml` into its root element; `None` for anything malformed,
/// too deep or too large.
pub(crate) fn parse(xml: &str) -> Option<El> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack: Vec<El> = Vec::new();
    let mut count = 0usize;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => {
                count += 1;
                if stack.len() >= MAX_DEPTH || count > MAX_ELEMENTS {
                    return None;
                }
                stack.push(El {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                    ..El::default()
                });
            }
            Event::End(_) => {
                let el = stack.pop()?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => return Some(el),
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
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// `text` with the five XML entities escaped.
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
    fn parses_namespaced_trees_and_refuses_bombs() {
        let el = parse(r#"<s:E xmlns:s="x"><s:B><a:X xmlns:a="y">1 &amp; 2</a:X><e/></s:B></s:E>"#)
            .unwrap();
        assert_eq!(el.name, "E");
        assert_eq!(el.text_of("X"), Some("1 & 2"));
        assert!(el.find("e").is_some());
        assert!(parse("").is_none());
        assert!(parse("<a><b></a>").is_none());
        let deep = "<a>".repeat(40) + &"</a>".repeat(40);
        assert!(parse(&deep).is_none());
        let wide = format!("<r>{}</r>", "<a/>".repeat(MAX_ELEMENTS + 1));
        assert!(parse(&wide).is_none());
        assert_eq!(escape("a<&'\""), "a&lt;&amp;&apos;&quot;");
    }
}
