//! XML 1.0 Fifth Edition Char/Name and Namespaces 1.0 QName checks.
//! See https://www.w3.org/TR/xml/#charsets and
//! https://www.w3.org/TR/xml-names/#ns-qualnames.

use crate::XmlError;

pub(crate) fn xml_whitespace(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
}

pub(crate) fn check_characters(value: &str) -> Result<(), XmlError> {
    if value.chars().all(|c| matches!(c as u32, 0x9 | 0xa | 0xd | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)) {
        Ok(())
    } else {
        Err(XmlError::Strict("invalid XML character"))
    }
}

fn name_start(c: char) -> bool {
    matches!(c, 'A'..='Z' | '_' | 'a'..='z')
        || matches!(c as u32,
        0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x2ff | 0x370..=0x37d |
        0x37f..=0x1fff | 0x200c..=0x200d | 0x2070..=0x218f |
        0x2c00..=0x2fef | 0x3001..=0xd7ff | 0xf900..=0xfdcf |
        0xfdf0..=0xfffd | 0x10000..=0xeffff)
}

fn ncname(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(name_start)
        && chars.all(|c| {
            name_start(c)
                || matches!(c, '-' | '.' | '0'..='9')
                || matches!(c as u32, 0xb7 | 0x300..=0x36f | 0x203f..=0x2040)
        })
}

pub(crate) fn check_name(value: &[u8]) -> Result<(), XmlError> {
    let value = core::str::from_utf8(value).map_err(XmlError::InvalidUtf8)?;
    let valid = match value.split_once(':') {
        Some((prefix, local)) => ncname(prefix) && ncname(local),
        None => ncname(value),
    };
    if valid {
        Ok(())
    } else {
        Err(XmlError::Strict("invalid XML qualified name"))
    }
}

pub(crate) fn check_reference(value: &str) -> Result<(), XmlError> {
    if let Some(number) = value.strip_prefix('#') {
        let valid = if let Some(hex) = number.strip_prefix('x') {
            !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        } else {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        };
        if !valid {
            return Err(XmlError::Strict("malformed XML entity"));
        }
    }
    Ok(())
}

pub(crate) fn check_attribute_references(value: &[u8]) -> Result<(), XmlError> {
    if value.contains(&b'<') {
        return Err(XmlError::Strict("unescaped delimiter in attribute"));
    }
    let mut tail = core::str::from_utf8(value).map_err(XmlError::InvalidUtf8)?;
    while let Some((_, reference)) = tail.split_once('&') {
        let (name, rest) = reference
            .split_once(';')
            .ok_or(XmlError::Strict("malformed XML entity"))?;
        check_reference(name)?;
        tail = rest;
    }
    Ok(())
}

pub(crate) fn check_declaration(value: &[u8]) -> Result<(), XmlError> {
    let invalid = || XmlError::Strict("malformed XML declaration");
    let text = core::str::from_utf8(value).map_err(XmlError::InvalidUtf8)?;
    let mut rest = text.strip_prefix("xml").ok_or_else(invalid)?;
    let mut stage = 0;
    while !rest.is_empty() {
        let trimmed = rest.trim_start_matches([' ', '\t', '\r', '\n']);
        if trimmed.len() == rest.len() {
            return Err(invalid());
        }
        rest = trimmed;
        if rest.is_empty() {
            break;
        }
        let end = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .ok_or_else(invalid)?;
        let name = &rest[..end];
        rest = rest[end..].trim_start_matches([' ', '\t', '\r', '\n']);
        rest = rest
            .strip_prefix('=')
            .ok_or_else(invalid)?
            .trim_start_matches([' ', '\t', '\r', '\n']);
        let quote = rest.chars().next().ok_or_else(invalid)?;
        if quote != '\'' && quote != '"' {
            return Err(invalid());
        }
        let (value, tail) = rest[1..].split_once(quote).ok_or_else(invalid)?;
        match (stage, name) {
            (0, "version") if value == "1.0" => stage = 1,
            (1, "encoding") if value.eq_ignore_ascii_case("UTF-8") => stage = 2,
            (1 | 2, "standalone") if matches!(value, "yes" | "no") => stage = 3,
            _ => return Err(invalid()),
        }
        rest = tail;
    }
    if stage == 0 { Err(invalid()) } else { Ok(()) }
}

/// quick-xml checks attribute values but accepts adjacent quoted attributes.
/// XML requires at least one S character before each attribute.
pub(crate) fn check_attribute_separators(tag: &[u8], name_len: usize) -> Result<(), XmlError> {
    let invalid = || XmlError::Strict("malformed XML attributes");
    let space = |byte: u8| matches!(byte, b' ' | b'\t' | b'\r' | b'\n');
    let mut rest = &tag[name_len..];
    while !rest.is_empty() {
        if !space(rest[0]) {
            return Err(invalid());
        }
        while rest.first().is_some_and(|byte| space(*byte)) {
            rest = &rest[1..];
        }
        if rest.is_empty() {
            return Ok(());
        }
        let end = rest
            .iter()
            .position(|byte| *byte == b'=' || space(*byte))
            .ok_or_else(invalid)?;
        if end == 0 {
            return Err(invalid());
        }
        rest = &rest[end..];
        while rest.first().is_some_and(|byte| space(*byte)) {
            rest = &rest[1..];
        }
        rest = rest.strip_prefix(b"=").ok_or_else(invalid)?;
        while rest.first().is_some_and(|byte| space(*byte)) {
            rest = &rest[1..];
        }
        let quote = *rest.first().ok_or_else(invalid)?;
        if quote != b'\'' && quote != b'"' {
            return Err(invalid());
        }
        let end = rest[1..]
            .iter()
            .position(|byte| *byte == quote)
            .ok_or_else(invalid)?;
        rest = &rest[end + 2..];
    }
    Ok(())
}
