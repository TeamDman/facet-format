use facet_xml::StrictXmlLimits;
use facet_xml_node::{StrictContent, StrictDocument, StrictError};

const RESPONSE: &str = r#"<?xml version="1.0"?>
<ApiResponse xmlns:vendor="urn:example:vendor" Status="OK">
  <CommandResponse>
    <DomainDNSGetHostsResult>
      <host Name="www" Type="A" Address="192.0.2.5" TTL="1800" vendor:guard="keep"/>
      <vendor:future vendor:data="preserve">  untouched  </vendor:future>
    </DomainDNSGetHostsResult>
  </CommandResponse>
</ApiResponse>"#;

fn known_element(name: &facet_xml_node::ExpandedName) -> bool {
    name.namespace().is_none()
        && matches!(
            name.local(),
            "ApiResponse" | "CommandResponse" | "DomainDNSGetHostsResult" | "host"
        )
}

fn known_attribute(
    owner: &facet_xml_node::ExpandedName,
    name: &facet_xml_node::ExpandedName,
) -> bool {
    name.namespace().is_none()
        && match owner.local() {
            "ApiResponse" => name.local() == "Status",
            "host" => matches!(name.local(), "Name" | "Type" | "Address" | "TTL"),
            _ => false,
        }
}

fn first_element(children: &[StrictContent]) -> &facet_xml_node::StrictElement {
    children
        .iter()
        .find_map(|child| match child {
            StrictContent::Element(element) => Some(element),
            StrictContent::Text(_) => None,
        })
        .unwrap()
}

#[test]
fn preserves_unknown_namespaced_fields_and_blocks_write_schema() {
    let document = StrictDocument::parse(RESPONSE.as_bytes(), StrictXmlLimits::default()).unwrap();
    assert_eq!(document.source(), RESPONSE.as_bytes());
    let command = first_element(document.root().children());
    let result = first_element(command.children());
    let host = first_element(result.children());
    assert!(
        host.attributes()
            .iter()
            .any(|attr| attr.name().namespace() == Some("urn:example:vendor")
                && attr.name().local() == "guard")
    );
    assert!(result.children().iter().any(|child| matches!(child, StrictContent::Element(element) if element.name().namespace() == Some("urn:example:vendor") && element.name().local() == "future")));
    let unknown = result
        .children()
        .iter()
        .find_map(|child| match child {
            StrictContent::Element(element) if element.name().local() == "future" => Some(element),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        document.raw_node(unknown),
        Some(b"<vendor:future vendor:data=\"preserve\">  untouched  </vendor:future>".as_slice())
    );
    assert!(matches!(
        document.require_known(known_element, known_attribute),
        Err(StrictError::UnknownAttribute)
    ));
}

#[test]
fn rejects_unrecognized_element_even_without_unknown_attribute() {
    let input = b"<ApiResponse><vendor:future xmlns:vendor='urn:example:vendor'/></ApiResponse>";
    let document = StrictDocument::parse(input, StrictXmlLimits::default()).unwrap();
    assert!(matches!(
        document.require_known(known_element, known_attribute),
        Err(StrictError::UnknownElement)
    ));
}

#[test]
fn rejects_lossy_or_unbounded_input() {
    let limits = StrictXmlLimits::default();
    for input in [
        "<!DOCTYPE ApiResponse><ApiResponse/>",
        "<ApiResponse><x:host/></ApiResponse>",
        "<ApiResponse a='1' a='2'/>",
        "<ApiResponse/ ><ApiResponse/>",
        "<ApiResponse>&unknown;</ApiResponse>",
        "<ApiResponse><!-- hidden --></ApiResponse>",
        "<ApiResponse><host><![CDATA[ ]]></host></ApiResponse>",
    ] {
        assert!(
            StrictDocument::parse(input.as_bytes(), limits).is_err(),
            "accepted {input}"
        );
    }
    let mut small = limits;
    small.max_input_bytes = 8;
    assert!(StrictDocument::parse(b"<ApiResponse/>", small).is_err());
    small = limits;
    small.max_depth = 1;
    assert!(StrictDocument::parse(b"<ApiResponse><host/></ApiResponse>", small).is_err());
}

#[test]
fn write_schema_rejects_text_not_represented_by_host_attributes() {
    for input in [
        b"<ApiResponse><host Name='www'>unexpected</host></ApiResponse>".as_slice(),
        b"<ApiResponse><host Name='www'> </host></ApiResponse>".as_slice(),
    ] {
        let document = StrictDocument::parse(input, StrictXmlLimits::default()).unwrap();
        assert!(matches!(
            document.require_known(known_element, known_attribute),
            Err(StrictError::UnexpectedText)
        ));
    }
}

#[test]
fn parser_diagnostics_do_not_echo_response_values() {
    let error = StrictDocument::parse(
        b"<ApiResponse value='SYNTHETIC_MARKER' value='SYNTHETIC_MARKER'/>",
        StrictXmlLimits::default(),
    )
    .err()
    .unwrap();
    assert!(!format!("{error:?}").contains("SYNTHETIC_MARKER"));
    assert!(!error.to_string().contains("SYNTHETIC_MARKER"));
}

#[test]
fn namespace_and_attribute_caps_apply_at_parser_boundary() {
    let mut limits = StrictXmlLimits::default();
    limits.max_attributes = 2;
    for input in ["<r xmlns:a='urn:a' xmlns:b='urn:b'/>", "<r a='1' b='2'/>"] {
        assert!(
            StrictDocument::parse(input.as_bytes(), limits).is_ok(),
            "{input}"
        );
    }
    for input in [
        "<r xmlns:a='urn:a' xmlns:b='urn:b' xmlns:c='urn:c'/>",
        "<r a='1' b='2' c='3'/>",
    ] {
        assert!(
            StrictDocument::parse(input.as_bytes(), limits).is_err(),
            "{input}"
        );
    }
    limits.max_depth = 1025;
    assert!(StrictDocument::parse(b"<r/>", limits).is_err());

    let default_limits = StrictXmlLimits::default();
    for namespace in [true, false] {
        let mut input = String::from("<r");
        for index in 0..129 {
            if namespace {
                input.push_str(&format!(" xmlns:n{index}='urn:{index}'"));
            } else {
                input.push_str(&format!(" a{index}='{index}'"));
            }
        }
        input.push_str("/>");
        assert!(StrictDocument::parse(input.as_bytes(), default_limits).is_err());
    }

    let mut event_limited = default_limits;
    event_limited.max_events = 1;
    assert!(StrictDocument::parse(b"<r/>", event_limited).is_err());
    let mut value_limited = default_limits;
    value_limited.max_value_bytes = 2;
    assert!(StrictDocument::parse(b"<r a='long'/>", value_limited).is_err());
    assert!(StrictDocument::parse(b"<r xmlns:x='long'/>", value_limited).is_err());
}

#[test]
fn strict_mode_rejects_ambiguous_declarations_and_literal_whitespace() {
    for input in [
        "<?xml version='1.1'?><r/>",
        "<?xml version='1.0' encoding='ISO-8859-1'?><r/>",
        "<?xml version='1.0'?><?xml version='1.0'?><r/>",
        "<r value='a\tb'/>",
        "<r value='a\nb'/>",
        "<r value='a\rb'/>",
        "<r xmlns:x='urn:a&amp;b'><x:host/></r>",
    ] {
        assert!(
            StrictDocument::parse(input.as_bytes(), StrictXmlLimits::default()).is_err(),
            "{input}"
        );
    }
    let documented = StrictDocument::parse(
        b"<?xml version='1.0' encoding='UTF-8'?><r value='a&#9;b'/>",
        StrictXmlLimits::default(),
    )
    .unwrap();
    assert_eq!(documented.root().attributes()[0].value(), "a\tb");
}
