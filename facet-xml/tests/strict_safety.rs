use facet_dom::DomParser;
use facet_xml::{StrictXmlLimits, XmlError, XmlParser};

fn parse(input: &str, limits: StrictXmlLimits) -> Result<(), XmlError> {
    let mut parser = XmlParser::new_strict(input.as_bytes(), limits)?;
    while parser.next_event()?.is_some() {}
    Ok(())
}

#[test]
fn repeated_namespace_expansion_is_bounded_before_retention() {
    let namespace = "n".repeat(64);
    for input in [
        format!("<r xmlns='{namespace}'>{}</r>", "<x/>".repeat(20)),
        format!("<r xmlns:p='{namespace}'>{}</r>", "<x p:a='v'/>".repeat(20)),
    ] {
        let limits = StrictXmlLimits {
            max_decoded_bytes: 512,
            ..Default::default()
        };
        assert!(matches!(
            parse(&input, limits),
            Err(XmlError::Strict("decoded byte limit"))
        ));
        assert!(
            parse(
                &input,
                StrictXmlLimits {
                    max_decoded_bytes: 4096,
                    ..limits
                }
            )
            .is_ok()
        );
    }
}

#[test]
fn aggregate_budget_counts_names_attributes_text_and_entities() {
    for (input, bytes) in [("<r a='x'>y</r>", 4), ("<r>&#x1f600;</r>", 5)] {
        let limits = StrictXmlLimits {
            max_decoded_bytes: bytes,
            ..Default::default()
        };
        assert!(parse(input, limits).is_ok(), "{input}");
        assert!(
            parse(
                input,
                StrictXmlLimits {
                    max_decoded_bytes: bytes - 1,
                    ..limits
                }
            )
            .is_err(),
            "{input}"
        );
    }
}

#[test]
fn entity_text_obeys_the_decoded_value_limit() {
    let limits = StrictXmlLimits {
        max_value_bytes: 1,
        ..Default::default()
    };
    assert!(parse("<r>&#x1f600;</r>", limits).is_err());
    assert!(
        parse(
            "<r>&#x1f600;</r>",
            StrictXmlLimits {
                max_value_bytes: 4,
                ..limits
            }
        )
        .is_ok()
    );
}

#[test]
fn only_xml_whitespace_is_allowed_outside_root() {
    for input in ["\u{a0}<r/>", "<r/>\u{2003}"] {
        assert!(parse(input, StrictXmlLimits::default()).is_err(), "{input}");
    }
    assert!(parse(" \t\r\n<r/>\n\r\t ", StrictXmlLimits::default()).is_ok());
}

#[test]
fn strict_qnames_follow_xml_name_grammar() {
    for input in [
        "<1bad/>",
        "<r 1bad='x'/>",
        "<a:b:c xmlns:a='urn:a'/>",
        "<r xmlns:1bad='urn:a'/>",
        "<r xmlns:a='urn:a' a:b:c='x'/>",
        "<:r/>",
    ] {
        assert!(parse(input, StrictXmlLimits::default()).is_err(), "{input}");
    }
    for input in ["<éclair _a-1.b='x'/>", "<p:根 xmlns:p='urn:p' p:属性='x'/>"] {
        assert!(parse(input, StrictXmlLimits::default()).is_ok(), "{input}");
    }
}

#[test]
fn strict_xml_characters_and_numeric_reference_grammar_are_checked() {
    for value in [
        "&#0;", "&#1;", "&#xFFFF;", "&#+65;", "&#X41;", "&#x;", "&#;", "\u{0}", "\u{1}", "\u{ffff}",
    ] {
        for input in [format!("<r>{value}</r>"), format!("<r a='{value}'/>")] {
            assert!(
                parse(&input, StrictXmlLimits::default()).is_err(),
                "{input:?}"
            );
        }
    }
    for input in [
        "<r>&#9;&#10;&#13;&#x1f600;</r>",
        "<r a='&#9;&#10;&#13;&#x1f600;'/>",
        "<r>😀</r>",
    ] {
        assert!(parse(input, StrictXmlLimits::default()).is_ok(), "{input}");
    }
}

#[test]
fn forbidden_lexical_delimiters_are_rejected_but_escaped_values_work() {
    for input in [
        "<r a='<'/>",
        "<r>]]></r>",
        "<r xmlns:p='urn:<bad'><p:x/></r>",
    ] {
        assert!(parse(input, StrictXmlLimits::default()).is_err(), "{input}");
    }
    for input in ["<r a='&lt;'/>", "<r>]]&gt;</r>"] {
        assert!(parse(input, StrictXmlLimits::default()).is_ok(), "{input}");
    }
}

#[test]
fn complete_declaration_grammar_and_placement_are_checked() {
    for declaration in [
        "xml version='1.0' version='1.0'",
        "xml version='1.0' unknown='x'",
        "xml version='1.0' standalone='maybe'",
        "xml version='1.0' standalone='yes' encoding='UTF-8'",
        "xml encoding='UTF-8' version='1.0'",
        "xml version='1.0'encoding='UTF-8'",
        "xml version='1.0' encoding='UTF-8' encoding='UTF-8'",
    ] {
        let input = format!("<?{declaration}?><r/>");
        assert!(
            parse(&input, StrictXmlLimits::default()).is_err(),
            "{input}"
        );
    }
    assert!(parse(" <?xml version='1.0'?><r/>", StrictXmlLimits::default()).is_err());
    for input in [
        "<?xml version='1.0'?><r/>",
        "<?xml version = '1.0' encoding='utf-8' standalone='yes' ?><r/>",
        "<?xml version='1.0' standalone='no'?><r/>",
        "\u{feff}<?xml version='1.0'?><r/>",
    ] {
        assert!(parse(input, StrictXmlLimits::default()).is_ok(), "{input}");
    }
}

#[test]
fn direct_parser_rejects_duplicate_expanded_attributes() {
    assert!(
        parse(
            "<r xmlns:a='urn:a' xmlns:b='urn:a' a:k='x' b:k='y'/>",
            StrictXmlLimits::default()
        )
        .is_err()
    );
    assert!(
        parse(
            "<r xmlns:a='urn:a' xmlns:b='urn:b' a:k='x' b:k='y'/>",
            StrictXmlLimits::default()
        )
        .is_ok()
    );
}

#[test]
fn attributes_require_xml_whitespace_separators() {
    for input in [
        "<r a='x'b='y'/>",
        "<r a=\"x\"b=\"y\"/>",
        "<r xmlns:p='urn:p'a='x'/>",
        "<r a='x'xmlns:p='urn:p'/>",
    ] {
        assert!(parse(input, StrictXmlLimits::default()).is_err(), "{input}");
    }
    assert!(parse("<r a = 'x'\t b = \"y\" />", StrictXmlLimits::default()).is_ok());
}

#[test]
fn xml_namespace_1_0_reserved_prefix_and_undeclaration_rules() {
    for input in [
        "<xmlns:r/>",
        "<r xmlns:p=''/>",
        "<r xmlns:p='urn:p'><x xmlns:p=''/></r>",
    ] {
        assert!(parse(input, StrictXmlLimits::default()).is_err(), "{input}");
    }
    assert!(
        parse(
            "<r xmlns='urn:r'><x xmlns=''/></r>",
            StrictXmlLimits::default()
        )
        .is_ok()
    );
}
