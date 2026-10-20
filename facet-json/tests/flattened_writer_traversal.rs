#![forbid(unsafe_code)]

use facet::Facet;
use facet_json::to_string;
use facet_json::to_writer_std;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::io::{self};

thread_local! {
    static FORMATTED_KEYS: Cell<usize> = const { Cell::new(0) };
}

// A flattened map materializes its field names through Display while advancing
// the reflection iterator. Counting value proxies would not detect eager field
// collection: collecting fields does not yet serialize their values.
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
struct FlattenedMap {
    #[facet(flatten)]
    entries: BTreeMap<CountedKey, u32>,
}

struct PrefixWriter {
    bytes: Vec<u8>,
    remaining: usize,
}

impl Write for PrefixWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "synthetic reader stopped after the object prefix",
            ));
        }
        let written = bytes.len().min(self.remaining);
        self.bytes.extend_from_slice(&bytes[..written]);
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!("serialization must leave flushing to the caller")
    }
}

#[test]
fn flattened_map_key_work_stops_at_the_first_failed_field_write() {
    let document = FlattenedMap {
        entries: (0..10_000)
            .map(|index| (CountedKey(index), index))
            .collect(),
    };
    let mut writer = PrefixWriter {
        bytes: Vec::new(),
        remaining: 1,
    };
    FORMATTED_KEYS.set(0);

    let error = to_writer_std(&mut writer, &document).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(writer.bytes, b"{");
    assert_eq!(
        FORMATTED_KEYS.get(),
        1,
        "the failing first field must not materialize the remaining 9,999 keys"
    );
}

#[derive(Facet)]
struct OrderedFields {
    before: u32,
    #[facet(flatten)]
    entries: BTreeMap<CountedKey, u32>,
    after: u32,
}

#[derive(Facet)]
struct NestedFields {
    #[facet(flatten)]
    fields: OrderedFields,
}

#[test]
fn streamed_flattened_fields_keep_declaration_and_map_iteration_order() {
    let document = NestedFields {
        fields: OrderedFields {
            before: 7,
            entries: BTreeMap::from([(CountedKey(2), 22), (CountedKey(1), 11)]),
            after: 9,
        },
    };
    FORMATTED_KEYS.set(0);

    let json = to_string(&document).unwrap();

    assert_eq!(json, r#"{"before":7,"key-1":11,"key-2":22,"after":9}"#);
    assert_eq!(FORMATTED_KEYS.get(), 2);
}

#[derive(Facet)]
#[repr(u8)]
#[allow(dead_code, reason = "fixture fields are read through Facet reflection")]
enum FlattenedChild {
    #[facet(rename = "alpha")]
    First(u32),
    Second(u32),
}

#[derive(Facet)]
struct FlattenedChildren {
    before: u32,
    #[facet(flatten)]
    children: Vec<FlattenedChild>,
    after: u32,
}

#[test]
fn streamed_flattened_enum_lists_keep_item_order_and_variant_names() {
    let document = FlattenedChildren {
        before: 7,
        children: vec![FlattenedChild::First(11), FlattenedChild::Second(22)],
        after: 9,
    };

    let json = to_string(&document).unwrap();

    assert_eq!(json, r#"{"before":7,"alpha":11,"Second":22,"after":9}"#);
}
