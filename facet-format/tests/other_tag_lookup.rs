#![forbid(unsafe_code)]

use facet::Facet;
use facet_format::FormatSerializer;
use facet_format::ScalarValue;
use facet_format::SerializeError;
use facet_format::StructFieldMode;
use facet_format::serialize_root;
use facet_reflect::Peek;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;

thread_local! {
    static FORMATTED_KEYS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Facet, Eq, PartialEq, Ord, PartialOrd)]
#[facet(opaque)]
struct CountedKey(u32);

impl fmt::Display for CountedKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        FORMATTED_KEYS.set(FORMATTED_KEYS.get() + 1);
        write!(formatter, "key-{}", self.0)
    }
}

#[derive(Facet)]
#[repr(u8)]
#[allow(dead_code, reason = "fixture fields are read through Facet reflection")]
enum UnknownVariant {
    #[facet(other)]
    Other {
        #[facet(flatten)]
        payload: BTreeMap<CountedKey, u32>,
        #[facet(tag)]
        tag: Option<String>,
    },
}

fn omit_tag(_value: &Option<String>) -> bool {
    true
}

#[derive(Facet)]
#[repr(u8)]
#[allow(dead_code, reason = "fixture fields are read through Facet reflection")]
enum UnknownVariantWithSkippedTag {
    #[facet(other)]
    Other {
        #[facet(flatten)]
        payload: BTreeMap<CountedKey, u32>,
        #[facet(tag, skip_serializing_if = omit_tag)]
        tag: Option<String>,
    },
}

#[derive(Debug, PartialEq)]
struct RejectedTag;

struct RejectingTagBackend {
    field_mode: StructFieldMode,
    tag: Option<String>,
}

impl FormatSerializer for RejectingTagBackend {
    type Error = RejectedTag;

    fn struct_field_mode(&self) -> StructFieldMode {
        self.field_mode
    }

    fn write_variant_tag(&mut self, tag: &str) -> Result<bool, Self::Error> {
        self.tag = Some(tag.to_owned());
        Err(RejectedTag)
    }

    fn begin_struct(&mut self) -> Result<(), Self::Error> {
        panic!("the first variant-tag write must fail before the payload")
    }

    fn field_key(&mut self, _key: &str) -> Result<(), Self::Error> {
        panic!("the first variant-tag write must fail before the payload")
    }

    fn end_struct(&mut self) -> Result<(), Self::Error> {
        panic!("the first variant-tag write must fail before the payload")
    }

    fn begin_seq(&mut self) -> Result<(), Self::Error> {
        panic!("the first variant-tag write must fail before the payload")
    }

    fn end_seq(&mut self) -> Result<(), Self::Error> {
        panic!("the first variant-tag write must fail before the payload")
    }

    fn scalar(&mut self, _value: ScalarValue<'_>) -> Result<(), Self::Error> {
        panic!("the first variant-tag write must fail before the payload")
    }
}

#[test]
fn other_tag_lookup_does_not_materialize_an_earlier_flattened_payload() {
    let value = UnknownVariant::Other {
        payload: (0..10_000)
            .map(|index| (CountedKey(index), index))
            .collect(),
        tag: Some("synthetic-unknown".to_owned()),
    };
    let mut backend = RejectingTagBackend {
        field_mode: StructFieldMode::Named,
        tag: None,
    };
    FORMATTED_KEYS.set(0);

    let result = serialize_root(&mut backend, Peek::new(&value));

    assert!(matches!(result, Err(SerializeError::Backend(RejectedTag))));
    assert_eq!(backend.tag.as_deref(), Some("synthetic-unknown"));
    assert_eq!(
        FORMATTED_KEYS.get(),
        0,
        "finding the tag must not traverse the flattened payload first"
    );
}

#[test]
fn named_tag_lookup_preserves_the_tag_fields_skip_predicate() {
    let value = UnknownVariantWithSkippedTag::Other {
        payload: (0..10_000)
            .map(|index| (CountedKey(index), index))
            .collect(),
        tag: Some("synthetic-unknown".to_owned()),
    };
    let mut backend = RejectingTagBackend {
        field_mode: StructFieldMode::Named,
        tag: None,
    };
    FORMATTED_KEYS.set(0);

    let result = serialize_root(&mut backend, Peek::new(&value));

    assert!(matches!(result, Err(SerializeError::Backend(RejectedTag))));
    assert_eq!(backend.tag.as_deref(), Some("Other"));
    assert_eq!(FORMATTED_KEYS.get(), 0);
}

#[test]
fn positional_tag_lookup_ignores_conditional_skip_predicates() {
    let value = UnknownVariantWithSkippedTag::Other {
        payload: (0..10_000)
            .map(|index| (CountedKey(index), index))
            .collect(),
        tag: Some("synthetic-unknown".to_owned()),
    };
    let mut backend = RejectingTagBackend {
        field_mode: StructFieldMode::Unnamed,
        tag: None,
    };
    FORMATTED_KEYS.set(0);

    let result = serialize_root(&mut backend, Peek::new(&value));

    assert!(matches!(result, Err(SerializeError::Backend(RejectedTag))));
    assert_eq!(backend.tag.as_deref(), Some("synthetic-unknown"));
    assert_eq!(FORMATTED_KEYS.get(), 0);
}
