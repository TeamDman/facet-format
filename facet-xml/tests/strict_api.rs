use facet_dom::DomParser;
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
