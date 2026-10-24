use facet::Facet;
use facet_dom::DomEvent;
use facet_dom::DomParser;
use facet_xml as xml;
use facet_xml::{StrictXmlLimits, XmlParser};

#[test]
fn direct_strict_parser_does_not_echo_input_on_error() {
    let mut parser = XmlParser::new_strict(
        b"<response value='SYNTHETIC_MARKER' value='SYNTHETIC_MARKER'/>",
        StrictXmlLimits::default(),
    )
    .unwrap();
    let error = parser.next_event().err().unwrap();
    assert!(!format!("{error:?}").contains("SYNTHETIC_MARKER"));
    assert!(!error.to_string().contains("SYNTHETIC_MARKER"));
}

#[test]
fn legacy_attribute_whitespace_is_preserved_after_parser_upgrade() {
    for input in [
        b"<x value='a\tb\nc'/>".as_slice(),
        b"<x value='a&#9;b&#10;c'/>".as_slice(),
    ] {
        let mut parser = XmlParser::new(input);
        loop {
            match parser.next_event().unwrap() {
                Some(DomEvent::Attribute { value, .. }) => {
                    assert_eq!(value.as_ref(), "a\tb\nc");
                    break;
                }
                Some(_) => {}
                None => panic!("missing attribute"),
            }
        }
    }
}

#[derive(Facet)]
struct LegacyAttribute {
    #[facet(xml::attribute)]
    value: String,
}

#[test]
fn normal_facet_deserialization_preserves_literal_attribute_whitespace() {
    let parsed: LegacyAttribute =
        facet_xml::from_str("<legacyAttribute value='a\tb\nc'/>").unwrap();
    assert_eq!(parsed.value, "a\tb\nc");
}
