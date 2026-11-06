//! Generic Cow values must enter their pointee before applying its wire contract.
//! Calling begin_inner on Cow instead selects a field on its opaque Rust layout.

use facet::Facet;
use facet_json::{from_str, from_str_borrowed, to_string};
use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq, Eq, Facet)]
#[facet(proxy = String)]
struct ValidatedName {
    value: String,
}

impl TryFrom<String> for ValidatedName {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err("name must not be empty");
        }
        Ok(Self { value })
    }
}

impl From<&ValidatedName> for String {
    fn from(value: &ValidatedName) -> Self {
        value.value.clone()
    }
}

#[derive(Debug, Facet)]
struct Request<'a> {
    name: Cow<'a, ValidatedName>,
}

#[test]
fn cow_container_proxy_roundtrips_owned_and_borrowed_values() {
    let name = ValidatedName::try_from("example".to_owned()).unwrap();
    let borrowed = Request {
        name: Cow::Borrowed(&name),
    };
    let owned = Request {
        name: Cow::Owned(name.clone()),
    };
    let json = to_string(&borrowed).unwrap();
    assert_eq!(json, to_string(&owned).unwrap());
    assert_eq!(json, r#"{"name":"example"}"#);

    let decoded: Request<'static> = from_str(&json).unwrap();
    assert!(matches!(decoded.name, Cow::Owned(ref value) if value == &name));

    // Borrowing the input does not permit borrowing a domain value that the
    // proxy constructor creates. That value must still be retained as owned.
    let decoded: Request<'_> = from_str_borrowed(&json).unwrap();
    assert!(matches!(decoded.name, Cow::Owned(ref value) if value == &name));
}

#[test]
fn cow_container_proxy_preserves_validation_errors() {
    let error = from_str::<Request<'static>>(r#"{"name":""}"#).unwrap_err();
    assert!(
        error.to_string().contains("name must not be empty"),
        "expected the pointee's proxy validation failure, got: {error}"
    );
}

#[derive(Clone, Debug, PartialEq, Eq, Facet)]
struct Record {
    label: String,
    values: Vec<u32>,
}

#[test]
fn cow_record_roundtrip_retains_an_owned_pointee() {
    let record = Record {
        label: "record".to_owned(),
        values: vec![1, 2, 3],
    };
    let json = to_string(&Cow::Borrowed(&record)).unwrap();
    let decoded: Cow<'static, Record> = from_str(&json).unwrap();
    assert!(matches!(decoded, Cow::Owned(ref value) if value == &record));
}
