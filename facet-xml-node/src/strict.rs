//! Bounded, namespace-aware read model for untrusted XML responses.
//!
//! The original bytes are retained because an expanded-name tree cannot
//! reconstruct namespace prefixes, declarations, or exact lexical spelling.
//! Callers must validate every field before constructing a write request.

use facet_dom::{DomEvent, DomParser};
use facet_xml::{StrictXmlLimits, XmlError, XmlParser};

/// XML expanded name: namespace URI plus local name.
#[derive(Clone, PartialEq, Eq, facet::Facet)]
pub struct ExpandedName {
    pub namespace: Option<String>,
    pub local: String,
}

/// An ordered XML attribute. Namespace declarations remain in `source`.
#[derive(Clone, PartialEq, Eq, facet::Facet)]
pub struct StrictAttribute {
    pub name: ExpandedName,
    pub value: String,
}

/// Text or a child element, in document order.
#[derive(Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum StrictContent {
    Text(String),
    Element(StrictElement),
}

/// Namespace-aware element with ordered attributes and children.
#[derive(Clone, PartialEq, Eq, facet::Facet)]
pub struct StrictElement {
    pub name: ExpandedName,
    pub attributes: Vec<StrictAttribute>,
    #[facet(recursive_type)]
    pub children: Vec<StrictContent>,
    /// Inclusive start byte offset in the original response.
    pub raw_start: usize,
    /// Exclusive end byte offset in the original response.
    pub raw_end: usize,
}

/// A parsed document together with its exact source bytes.
pub struct StrictDocument {
    pub root: StrictElement,
    source: Vec<u8>,
}

impl StrictDocument {
    /// Parse a bounded response, retaining its exact bytes for audit and diff binding.
    pub fn parse(input: &[u8], limits: StrictXmlLimits) -> Result<Self, StrictError> {
        let mut parser = XmlParser::new_strict(input, limits).map_err(redact_xml_error)?;
        let mut stack: Vec<StrictElement> = Vec::new();
        let mut root = None;
        while let Some(event) = parser.next_event().map_err(redact_xml_error)? {
            match event {
                DomEvent::NodeStart { tag, namespace } => stack.push(StrictElement {
                    name: ExpandedName {
                        namespace: namespace.map(|s| s.into_owned()),
                        local: tag.into_owned(),
                    },
                    attributes: Vec::new(),
                    children: Vec::new(),
                    raw_start: parser.last_node_start(),
                    raw_end: 0,
                }),
                DomEvent::Attribute {
                    name,
                    value,
                    namespace,
                } => {
                    let element = stack.last_mut().ok_or(StrictError::Structure)?;
                    let name = ExpandedName {
                        namespace: namespace.map(|s| s.into_owned()),
                        local: name.into_owned(),
                    };
                    if element.attributes.iter().any(|attr| attr.name == name) {
                        return Err(StrictError::DuplicateAttribute);
                    }
                    element.attributes.push(StrictAttribute {
                        name,
                        value: value.into_owned(),
                    });
                }
                DomEvent::Text(value) => stack
                    .last_mut()
                    .ok_or(StrictError::Structure)?
                    .children
                    .push(StrictContent::Text(value.into_owned())),
                DomEvent::NodeEnd => {
                    let mut element = stack.pop().ok_or(StrictError::Structure)?;
                    element.raw_end = parser.input_offset();
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(StrictContent::Element(element));
                    } else if root.replace(element).is_some() {
                        return Err(StrictError::Structure);
                    }
                }
                DomEvent::ChildrenStart | DomEvent::ChildrenEnd => {}
                DomEvent::Comment(_)
                | DomEvent::Doctype(_)
                | DomEvent::ProcessingInstruction { .. } => return Err(StrictError::Structure),
            }
        }
        if !stack.is_empty() {
            return Err(StrictError::Structure);
        }
        Ok(Self {
            root: root.ok_or(StrictError::Structure)?,
            source: input.to_vec(),
        })
    }

    /// Original bytes, including namespace declarations and lexical details.
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// Exact source bytes for an element, including its original prefixes and declarations.
    pub fn raw_node(&self, element: &StrictElement) -> Option<&[u8]> {
        self.source.get(element.raw_start..element.raw_end)
    }

    /// Reject any element or attribute outside an explicit caller-owned schema.
    /// This is a prerequisite for a write plan, not permission to perform a write.
    pub fn require_known(
        &self,
        element: impl Fn(&ExpandedName) -> bool,
        attribute: impl Fn(&ExpandedName, &ExpandedName) -> bool,
    ) -> Result<(), StrictError> {
        fn visit(
            node: &StrictElement,
            element: &impl Fn(&ExpandedName) -> bool,
            attribute: &impl Fn(&ExpandedName, &ExpandedName) -> bool,
        ) -> Result<(), StrictError> {
            if !element(&node.name) {
                return Err(StrictError::UnknownElement);
            }
            for attr in &node.attributes {
                if !attribute(&node.name, &attr.name) {
                    return Err(StrictError::UnknownAttribute);
                }
            }
            for child in &node.children {
                match child {
                    StrictContent::Element(child) => visit(child, element, attribute)?,
                    StrictContent::Text(text) if text.trim().is_empty() => {}
                    StrictContent::Text(_) => return Err(StrictError::UnexpectedText),
                }
            }
            Ok(())
        }
        visit(&self.root, &element, &attribute)
    }
}

/// Failure to preserve or understand the complete XML response.
#[derive(Debug)]
pub enum StrictError {
    Xml(XmlError),
    Structure,
    DuplicateAttribute,
    UnknownElement,
    UnknownAttribute,
    UnexpectedText,
}

impl std::fmt::Display for StrictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xml(error) => error.fmt(f),
            Self::Structure => f.write_str("invalid XML event structure"),
            Self::DuplicateAttribute => f.write_str("duplicate expanded attribute name"),
            Self::UnknownElement => f.write_str("unrecognized XML element"),
            Self::UnknownAttribute => f.write_str("unrecognized XML attribute"),
            Self::UnexpectedText => f.write_str("unexpected XML text content"),
        }
    }
}

impl std::error::Error for StrictError {}

fn redact_xml_error(error: XmlError) -> StrictError {
    match error {
        // Parser diagnostics may quote attacker-controlled source. The strict
        // API never exposes source fragments through Display or Debug.
        XmlError::Parse(_) => StrictError::Xml(XmlError::Strict("malformed XML")),
        other => StrictError::Xml(other),
    }
}
