use facet_xml::StrictXmlLimits;
use facet_xml_node::{StrictContent, StrictDocument, StrictError};

#[test]
fn raw_ranges_only_apply_to_elements_owned_by_the_document() {
    let source = b"<r><a/></r>";
    let document = StrictDocument::parse(source, StrictXmlLimits::default()).unwrap();
    let foreign = StrictDocument::parse(b"<s><b/></s>", StrictXmlLimits::default()).unwrap();
    assert_eq!(document.raw_node(document.root()), Some(source.as_slice()));
    let StrictContent::Element(child) = &document.root().children()[0] else {
        panic!("missing child")
    };
    assert_eq!(document.raw_node(child), Some(b"<a/>".as_slice()));
    assert_eq!(document.raw_node(foreign.root()), None);
    assert_eq!(document.raw_node(&document.root().clone()), None);
}

#[test]
fn write_schema_does_not_discard_unicode_character_data_as_whitespace() {
    for whitespace in ["\u{a0}", "\u{2003}"] {
        let input = format!("<r><a/>{whitespace}<b/></r>");
        let document = StrictDocument::parse(input.as_bytes(), StrictXmlLimits::default()).unwrap();
        assert!(matches!(
            document.require_known(|_| true, |_, _| true),
            Err(StrictError::UnexpectedText)
        ));
    }
    let document =
        StrictDocument::parse(b"<r><a/> \t\r\n<b/></r>", StrictXmlLimits::default()).unwrap();
    assert!(document.require_known(|_| true, |_, _| true).is_ok());
}

#[test]
fn document_enforces_aggregate_parser_budget() {
    let namespace = "n".repeat(64);
    let input = format!("<r xmlns='{namespace}'>{}</r>", "<x/>".repeat(20));
    let limits = StrictXmlLimits {
        max_decoded_bytes: 512,
        ..Default::default()
    };
    assert!(StrictDocument::parse(input.as_bytes(), limits).is_err());
}

#[test]
fn bom_does_not_shift_original_source_ranges() {
    for prefix in ["\u{feff}", "\u{feff}<?xml version='1.0'?>"] {
        let input = format!("{prefix}<r><a/></r>");
        let document = StrictDocument::parse(input.as_bytes(), StrictXmlLimits::default()).unwrap();
        assert_eq!(document.source(), input.as_bytes());
        assert_eq!(
            document.raw_node(document.root()),
            Some(b"<r><a/></r>".as_slice())
        );
        let StrictContent::Element(child) = &document.root().children()[0] else {
            panic!("missing child")
        };
        assert_eq!(document.raw_node(child), Some(b"<a/>".as_slice()));
    }
}

#[test]
fn document_rejects_invalid_namespace_1_0_constructs() {
    for input in ["<xmlns:r/>", "<r xmlns:p=''/>"] {
        assert!(
            StrictDocument::parse(input.as_bytes(), StrictXmlLimits::default()).is_err(),
            "{input}"
        );
    }
}
