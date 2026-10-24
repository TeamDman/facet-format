//! Streaming DomParser implementation for XML using quick-xml.

extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use std::io::Cursor;

use facet_dom::{DomEvent, DomParser};
use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::escape::{resolve_xml_entity, unescape};
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

/// XML parsing error.
#[derive(Debug, Clone)]
pub enum XmlError {
    /// Error from quick-xml.
    Parse(String),
    /// Unexpected end of input.
    UnexpectedEof,
    /// Unbalanced tags.
    UnbalancedTags,
    /// Invalid UTF-8.
    InvalidUtf8(core::str::Utf8Error),
    /// A strict parsing rule or resource limit was violated.
    Strict(&'static str),
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlError::Parse(msg) => write!(f, "XML parse error: {}", msg),
            XmlError::UnexpectedEof => write!(f, "Unexpected end of XML"),
            XmlError::UnbalancedTags => write!(f, "Unbalanced XML tags"),
            XmlError::InvalidUtf8(e) => write!(f, "Invalid UTF-8 in XML: {}", e),
            XmlError::Strict(reason) => write!(f, "Strict XML parse rejected: {reason}"),
        }
    }
}

impl std::error::Error for XmlError {}

/// Streaming XML parser implementing `DomParser`.
pub struct XmlParser<'de> {
    reader: NsReader<Cursor<&'de [u8]>>,
    /// Original input for raw capture
    input: &'de [u8],
    /// Buffer for quick-xml events
    buf: Vec<u8>,
    /// Buffer for peeked event
    peeked: Option<DomEvent<'de>>,
    /// Stack tracking element depth for skip_node
    depth: usize,
    /// Pending attributes from the current element
    pending_attrs: Vec<(Option<String>, String, String)>,
    /// Index into pending_attrs
    attr_idx: usize,
    /// State machine for event generation
    state: ParserState,
    /// True if current element is empty (self-closing)
    is_empty_element: bool,
    /// Position where current node started (for raw capture)
    node_start_pos: u64,
    strict: bool,
    limits: Option<StrictXmlLimits>,
    raw_events: usize,
    root_seen: bool,
    root_closed: bool,
    decl_seen: bool,
}

/// Conservative limits for untrusted XML API responses.
#[derive(Debug, Clone, Copy)]
pub struct StrictXmlLimits {
    /// Maximum encoded input size, in bytes.
    pub max_input_bytes: usize,
    /// Maximum nested element depth. Strict mode rejects settings above 1024
    /// because quick-xml 0.41 counts namespace nesting with a u16.
    pub max_depth: usize,
    /// Maximum raw parser events.
    pub max_events: usize,
    /// Maximum attributes on one element, including namespace declarations.
    pub max_attributes: usize,
    /// Maximum decoded length of one text or attribute value.
    pub max_value_bytes: usize,
}

impl Default for StrictXmlLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_depth: 64,
            max_events: 100_000,
            max_attributes: 128,
            max_value_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ParserState {
    /// Ready to read next XML event
    Ready,
    /// Just emitted NodeStart, need to emit attributes
    EmittingAttrs,
    /// Done emitting attrs, need to emit ChildrenStart
    NeedChildrenStart,
    /// Inside element children
    InChildren,
    /// Need to emit ChildrenEnd before NodeEnd
    NeedChildrenEnd,
    /// Need to emit NodeEnd
    NeedNodeEnd,
    /// End of document
    Done,
}

impl<'de> XmlParser<'de> {
    /// Create a new streaming XML parser.
    ///
    /// quick-xml 0.41 caps namespace declarations at 256 per element in this
    /// legacy mode too. Other legacy text and attribute behavior is retained.
    pub fn new(input: &'de [u8]) -> Self {
        trace!(input_len = input.len(), "creating XML parser");

        let mut reader = NsReader::from_reader(Cursor::new(input));
        reader.config_mut().trim_text(true);

        Self {
            reader,
            input,
            buf: Vec::new(),
            peeked: None,
            depth: 0,
            pending_attrs: Vec::new(),
            attr_idx: 0,
            state: ParserState::Ready,
            is_empty_element: false,
            node_start_pos: 0,
            strict: false,
            limits: None,
            raw_events: 0,
            root_seen: false,
            root_closed: false,
            decl_seen: false,
        }
    }

    /// Apply response-size and structural limits without whitespace trimming.
    /// This entry point rejects DTDs, processing instructions, comments, CDATA,
    /// unknown entities, undeclared prefixes, and multiple roots. It does not serialize.
    /// Namespace declarations are capped inside quick-xml before it returns a
    /// start event. Strict mode accepts only XML 1.0 in UTF-8, and rejects
    /// literal tab, CR, or LF inside attribute values and escaped namespace
    /// declarations to avoid representing normalized or unresolved values.
    pub fn new_strict(input: &'de [u8], limits: StrictXmlLimits) -> Result<Self, XmlError> {
        // quick-xml 0.41 uses a u16 namespace nesting counter before returning
        // each start event, so the downstream depth check must stay below it.
        if limits.max_depth > 1024 {
            return Err(XmlError::Strict(
                "configured depth limit exceeds parser bound",
            ));
        }
        if input.len() > limits.max_input_bytes {
            return Err(XmlError::Strict("input size limit"));
        }
        let mut parser = Self::new(input);
        parser.reader.config_mut().trim_text(false);
        parser
            .reader
            .resolver_mut()
            .set_max_declarations_per_element(limits.max_attributes.min(256));
        parser.strict = true;
        parser.limits = Some(limits);
        Ok(parser)
    }

    /// Byte offset of the most recently opened element in the original input.
    pub fn last_node_start(&self) -> usize {
        self.node_start_pos as usize
    }

    /// Byte offset immediately after the most recently consumed XML token.
    pub fn input_offset(&self) -> usize {
        self.reader.buffer_position() as usize
    }

    /// Capture the current node as raw XML and skip past it.
    /// Must be called right after a NodeStart event has been consumed.
    fn do_capture_raw_node(&mut self) -> Result<Cow<'de, str>, XmlError> {
        // Save start position before it gets overwritten by child elements
        let start = self.node_start_pos as usize;
        let start_depth = self.depth;

        // Skip through the node - consume events until depth drops below starting
        loop {
            // Handle peeked event first
            let event = if let Some(e) = self.peeked.take() {
                Some(e)
            } else {
                self.read_next()?
            };

            match event {
                Some(DomEvent::NodeEnd) if self.depth < start_depth => break,
                None => break,
                _ => {}
            }
        }

        let end = self.reader.buffer_position() as usize;
        let raw = &self.input[start..end];
        let s = core::str::from_utf8(raw).map_err(XmlError::InvalidUtf8)?;
        Ok(Cow::Borrowed(s))
    }

    /// Read the next raw event from quick-xml and convert to DomEvent.
    fn read_next(&mut self) -> Result<Option<DomEvent<'de>>, XmlError> {
        let strict = self.strict;
        loop {
            match self.state {
                ParserState::Done => return Ok(None),

                ParserState::EmittingAttrs => {
                    if self.attr_idx < self.pending_attrs.len() {
                        let (ns, name, value) = &self.pending_attrs[self.attr_idx];
                        let event = DomEvent::Attribute {
                            name: Cow::Owned(name.clone()),
                            value: Cow::Owned(value.clone()),
                            namespace: ns.clone().map(Cow::Owned),
                        };
                        self.attr_idx += 1;
                        return Ok(Some(event));
                    }
                    // Done with attrs
                    self.pending_attrs.clear();
                    self.attr_idx = 0;
                    self.state = ParserState::NeedChildrenStart;
                }

                ParserState::NeedChildrenStart => {
                    if self.is_empty_element {
                        self.state = ParserState::NeedChildrenEnd;
                        self.is_empty_element = false;
                    } else {
                        self.state = ParserState::InChildren;
                    }
                    return Ok(Some(DomEvent::ChildrenStart));
                }

                ParserState::NeedChildrenEnd => {
                    self.state = ParserState::NeedNodeEnd;
                    return Ok(Some(DomEvent::ChildrenEnd));
                }

                ParserState::NeedNodeEnd => {
                    self.depth -= 1;
                    self.state = if self.depth == 0 {
                        if self.strict {
                            self.root_closed = true;
                            ParserState::Ready
                        } else {
                            ParserState::Done
                        }
                    } else {
                        ParserState::InChildren
                    };
                    return Ok(Some(DomEvent::NodeEnd));
                }

                ParserState::Ready | ParserState::InChildren => {
                    // Record position before reading (for raw capture)
                    let pos_before = self.reader.buffer_position();

                    self.buf.clear();
                    let (resolve, event) = self
                        .reader
                        .read_resolved_event_into(&mut self.buf)
                        .map_err(|e| parser_error(strict, e))?;

                    if let Some(limits) = self.limits {
                        self.raw_events += 1;
                        if self.raw_events > limits.max_events {
                            return Err(XmlError::Strict("event count limit"));
                        }
                    }

                    // Resolve element namespace upfront
                    let elem_ns = resolve_namespace(resolve, self.strict)?;

                    match event {
                        Event::Start(ref e) | Event::Empty(ref e) => {
                            if self.strict && self.depth == 0 {
                                if self.root_seen {
                                    return Err(XmlError::Strict("multiple document roots"));
                                }
                                self.root_seen = true;
                            }
                            if let Some(limits) = self.limits {
                                if self.depth >= limits.max_depth {
                                    return Err(XmlError::Strict("element depth limit"));
                                }
                                if e.attributes().count() > limits.max_attributes {
                                    return Err(XmlError::Strict("attribute count limit"));
                                }
                            }
                            let is_empty = matches!(event, Event::Empty(_));
                            // Record start position for potential raw capture
                            self.node_start_pos = pos_before;

                            // Get element local name
                            let local_name = e.local_name();
                            let local = core::str::from_utf8(local_name.as_ref())
                                .map_err(XmlError::InvalidUtf8)?;
                            let local_owned = local.to_string();

                            // Collect attributes
                            self.pending_attrs.clear();
                            self.attr_idx = 0;

                            for attr in e.attributes() {
                                let attr = attr.map_err(|e| parser_error(strict, e))?;

                                // Skip xmlns declarations
                                let key = attr.key;
                                if strict {
                                    core::str::from_utf8(key.as_ref())
                                        .map_err(XmlError::InvalidUtf8)?;
                                }
                                if key.as_ref() == b"xmlns" {
                                    check_namespace_declaration(self.limits, attr.value.as_ref())?;
                                    continue;
                                }
                                if let Some(prefix) = key.prefix()
                                    && prefix.as_ref() == b"xmlns"
                                {
                                    check_namespace_declaration(self.limits, attr.value.as_ref())?;
                                    continue;
                                }

                                let (attr_resolve, _) =
                                    self.reader.resolver().resolve_attribute(key);
                                let attr_ns = resolve_namespace(attr_resolve, self.strict)?;
                                let attr_local_name = key.local_name();
                                let attr_local = core::str::from_utf8(attr_local_name.as_ref())
                                    .map_err(XmlError::InvalidUtf8)?;

                                let value = if strict {
                                    if attr
                                        .value
                                        .as_ref()
                                        .iter()
                                        .any(|byte| matches!(*byte, b'\t' | b'\r' | b'\n'))
                                    {
                                        return Err(XmlError::Strict(
                                            "literal control whitespace in attribute",
                                        ));
                                    }
                                    attr.decoded_and_normalized_value(
                                        XmlVersion::Implicit1_0,
                                        self.reader.decoder(),
                                    )
                                    .map_err(|e| parser_error(strict, e))?
                                } else {
                                    // Preserve the 0.39 Facet XML behavior. The 0.41
                                    // convenience method now normalizes literal whitespace.
                                    let decoded = self
                                        .reader
                                        .decoder()
                                        .decode(attr.value.as_ref())
                                        .map_err(|e| parser_error(strict, e))?;
                                    Cow::Owned(
                                        unescape(decoded.as_ref())
                                            .map_err(|e| parser_error(strict, e))?
                                            .into_owned(),
                                    )
                                };
                                if self
                                    .limits
                                    .is_some_and(|limits| value.len() > limits.max_value_bytes)
                                {
                                    return Err(XmlError::Strict("attribute value limit"));
                                }

                                self.pending_attrs.push((
                                    attr_ns,
                                    attr_local.to_string(),
                                    value.into_owned(),
                                ));
                            }

                            self.depth += 1;
                            self.is_empty_element = is_empty;

                            if self.pending_attrs.is_empty() {
                                self.state = ParserState::NeedChildrenStart;
                            } else {
                                self.state = ParserState::EmittingAttrs;
                            }

                            return Ok(Some(DomEvent::NodeStart {
                                tag: Cow::Owned(local_owned),
                                namespace: elem_ns.map(Cow::Owned),
                            }));
                        }
                        Event::End(_) => {
                            if self.strict && self.depth == 0 {
                                return Err(XmlError::Strict("unmatched closing element"));
                            }
                            self.state = ParserState::NeedChildrenEnd;
                        }
                        Event::Text(e) => {
                            let text = e.decode().map_err(|e| parser_error(strict, e))?;
                            if self.strict {
                                if self.depth == 0 {
                                    if text.trim().is_empty() {
                                        continue;
                                    }
                                    return Err(XmlError::Strict("text outside document root"));
                                }
                                if self
                                    .limits
                                    .is_some_and(|limits| text.len() > limits.max_value_bytes)
                                {
                                    return Err(XmlError::Strict("text value limit"));
                                }
                                return Ok(Some(DomEvent::Text(Cow::Owned(text.into_owned()))));
                            }
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                return Ok(Some(DomEvent::Text(Cow::Owned(trimmed.to_string()))));
                            }
                        }
                        Event::CData(e) => {
                            if self.strict {
                                return Err(XmlError::Strict("CDATA is not supported"));
                            }
                            let text =
                                core::str::from_utf8(e.as_ref()).map_err(XmlError::InvalidUtf8)?;
                            if !text.is_empty() {
                                return Ok(Some(DomEvent::Text(Cow::Owned(text.to_string()))));
                            }
                        }
                        Event::Comment(e) => {
                            if self.strict {
                                return Err(XmlError::Strict("comments are not supported"));
                            }
                            let text =
                                core::str::from_utf8(e.as_ref()).map_err(XmlError::InvalidUtf8)?;
                            return Ok(Some(DomEvent::Comment(Cow::Owned(text.to_string()))));
                        }
                        Event::PI(e) => {
                            if self.strict {
                                return Err(XmlError::Strict(
                                    "processing instructions are not supported",
                                ));
                            }
                            let content =
                                core::str::from_utf8(e.as_ref()).map_err(XmlError::InvalidUtf8)?;
                            let (target, data) = content
                                .split_once(char::is_whitespace)
                                .unwrap_or((content, ""));
                            return Ok(Some(DomEvent::ProcessingInstruction {
                                target: Cow::Owned(target.to_string()),
                                data: Cow::Owned(data.trim().to_string()),
                            }));
                        }
                        Event::Decl(e) => {
                            if strict {
                                if self.root_seen || self.decl_seen {
                                    return Err(XmlError::Strict(
                                        "duplicate or misplaced XML declaration",
                                    ));
                                }
                                self.decl_seen = true;
                                if e.xml_version()
                                    .map_err(|error| parser_error(strict, error))?
                                    != XmlVersion::Explicit1_0
                                {
                                    return Err(XmlError::Strict("unsupported XML version"));
                                }
                                if let Some(encoding) = e.encoding() {
                                    let encoding =
                                        encoding.map_err(|error| parser_error(strict, error))?;
                                    if !encoding.as_ref().eq_ignore_ascii_case(b"UTF-8") {
                                        return Err(XmlError::Strict("unsupported XML encoding"));
                                    }
                                }
                            }
                        }
                        Event::DocType(e) => {
                            if self.strict {
                                return Err(XmlError::Strict("DOCTYPE is not supported"));
                            }
                            // Parse DOCTYPE declaration and emit as DomEvent
                            let text =
                                core::str::from_utf8(e.as_ref()).map_err(XmlError::InvalidUtf8)?;
                            return Ok(Some(DomEvent::Doctype(Cow::Owned(text.to_string()))));
                        }
                        Event::Eof => {
                            if self.strict
                                && (!self.root_seen || !self.root_closed || self.depth != 0)
                            {
                                return Err(XmlError::Strict("incomplete XML document"));
                            }
                            self.state = ParserState::Done;
                            return Ok(None);
                        }
                        Event::GeneralRef(e) => {
                            let raw = e.decode().map_err(|e| parser_error(strict, e))?;
                            let resolved = resolve_entity(&raw).map_err(|error| {
                                if strict {
                                    XmlError::Strict("malformed XML entity")
                                } else {
                                    error
                                }
                            })?;
                            if self.strict {
                                if self.depth == 0 {
                                    return Err(XmlError::Strict("entity outside document root"));
                                }
                                if resolved.starts_with('&') && resolved.ends_with(';') {
                                    return Err(XmlError::Strict("unknown entity"));
                                }
                            }
                            return Ok(Some(DomEvent::Text(Cow::Owned(resolved))));
                        }
                    }
                }
            }
        }
    }
}

impl<'de> DomParser<'de> for XmlParser<'de> {
    type Error = XmlError;

    fn next_event(&mut self) -> Result<Option<DomEvent<'de>>, Self::Error> {
        if let Some(event) = self.peeked.take() {
            return Ok(Some(event));
        }
        self.read_next()
    }

    fn peek_event(&mut self) -> Result<Option<&DomEvent<'de>>, Self::Error> {
        if self.peeked.is_none() {
            self.peeked = self.read_next()?;
        }
        Ok(self.peeked.as_ref())
    }

    fn skip_node(&mut self) -> Result<(), Self::Error> {
        let start_depth = self.depth;

        loop {
            let event = self.next_event()?;
            match event {
                Some(DomEvent::NodeEnd) if self.depth < start_depth => break,
                None => break,
                _ => {}
            }
        }

        Ok(())
    }

    fn current_span(&self) -> Option<facet_reflect::Span> {
        None
    }

    fn format_namespace(&self) -> Option<&'static str> {
        Some("xml")
    }

    fn capture_raw_node(&mut self) -> Result<Option<Cow<'de, str>>, Self::Error> {
        Ok(Some(self.do_capture_raw_node()?))
    }
}

fn parser_error(strict: bool, error: impl fmt::Display) -> XmlError {
    if strict {
        XmlError::Strict("malformed XML")
    } else {
        XmlError::Parse(error.to_string())
    }
}

fn check_namespace_declaration(
    limits: Option<StrictXmlLimits>,
    value: &[u8],
) -> Result<(), XmlError> {
    if let Some(limits) = limits {
        if value.len() > limits.max_value_bytes {
            return Err(XmlError::Strict("namespace value limit"));
        }
        core::str::from_utf8(value).map_err(XmlError::InvalidUtf8)?;
        if value
            .iter()
            .any(|byte| matches!(*byte, b'&' | b'\t' | b'\r' | b'\n'))
        {
            return Err(XmlError::Strict("unsupported namespace declaration value"));
        }
    }
    Ok(())
}

/// Resolve a namespace from quick-xml's ResolveResult.
fn resolve_namespace(resolve: ResolveResult<'_>, strict: bool) -> Result<Option<String>, XmlError> {
    match resolve {
        ResolveResult::Bound(ns) if strict => Ok(Some(
            core::str::from_utf8(ns.as_ref())
                .map_err(XmlError::InvalidUtf8)?
                .to_owned(),
        )),
        ResolveResult::Bound(ns) => Ok(Some(String::from_utf8_lossy(ns.as_ref()).into_owned())),
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Unknown(_) if strict => Err(XmlError::Strict("undeclared namespace prefix")),
        ResolveResult::Unknown(_) => Ok(None),
    }
}

/// Resolve a general entity reference.
fn resolve_entity(raw: &str) -> Result<String, XmlError> {
    if let Some(resolved) = resolve_xml_entity(raw) {
        return Ok(resolved.into());
    }

    if let Some(rest) = raw.strip_prefix('#') {
        let code = if let Some(hex) = rest.strip_prefix('x').or_else(|| rest.strip_prefix('X')) {
            u32::from_str_radix(hex, 16)
                .map_err(|_| XmlError::Parse(format!("Invalid hex entity: #{}", rest)))?
        } else {
            rest.parse::<u32>()
                .map_err(|_| XmlError::Parse(format!("Invalid decimal entity: #{}", rest)))?
        };

        let ch = char::from_u32(code)
            .ok_or_else(|| XmlError::Parse(format!("Invalid Unicode: {}", code)))?;
        return Ok(ch.to_string());
    }

    Ok(format!("&{};", raw))
}
