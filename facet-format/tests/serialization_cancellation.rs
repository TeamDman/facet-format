use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use facet::Facet;
use facet_format::{
    FormatSerializer, ScalarValue, SerializeError, serialize_root, serialize_root_with_cancellation,
};
use facet_reflect::{FieldItem, Peek};

#[derive(Debug, PartialEq)]
enum BackendError {
    Rejected,
}

struct Recorder<'a> {
    events: Vec<String>,
    scalars: &'a Cell<usize>,
    omitted_fields: Option<&'a Cell<usize>>,
    fail_and_cancel: Option<&'a Cell<bool>>,
}

impl<'a> Recorder<'a> {
    fn new(scalars: &'a Cell<usize>) -> Self {
        Self {
            events: Vec::new(),
            scalars,
            omitted_fields: None,
            fail_and_cancel: None,
        }
    }
}

impl FormatSerializer for Recorder<'_> {
    type Error = BackendError;

    fn begin_struct(&mut self) -> Result<(), Self::Error> {
        self.events.push("{".into());
        Ok(())
    }

    fn field_key(&mut self, key: &str) -> Result<(), Self::Error> {
        self.events.push(key.into());
        Ok(())
    }

    fn end_struct(&mut self) -> Result<(), Self::Error> {
        self.events.push("}".into());
        Ok(())
    }

    fn begin_seq(&mut self) -> Result<(), Self::Error> {
        self.events.push("[".into());
        Ok(())
    }

    fn end_seq(&mut self) -> Result<(), Self::Error> {
        self.events.push("]".into());
        Ok(())
    }

    fn scalar(&mut self, value: ScalarValue<'_>) -> Result<(), Self::Error> {
        self.scalars.set(self.scalars.get() + 1);
        if let Some(cancelled) = self.fail_and_cancel {
            cancelled.set(true);
            return Err(BackendError::Rejected);
        }
        self.events.push(format!("{value:?}"));
        Ok(())
    }

    fn should_omit_field(&self, _: &FieldItem, _: Peek<'_, '_>) -> bool {
        if let Some(count) = self.omitted_fields {
            count.set(count.get() + 1);
            true
        } else {
            false
        }
    }
}

#[test]
fn an_existing_request_prevents_backend_work() {
    let scalars = Cell::new(0);
    let mut backend = Recorder::new(&scalars);
    let result =
        serialize_root_with_cancellation(&mut backend, Peek::new(&vec![1, 2, 3]), &|| true);

    assert!(matches!(result, Err(SerializeError::Cancelled)));
    assert!(backend.events.is_empty());
}

#[test]
fn a_borrowed_predicate_stops_a_large_container_before_remaining_elements() {
    let scalars = Cell::new(0);
    let mut backend = Recorder::new(&scalars);
    let values: Vec<u32> = (0..10_000).collect();
    let should_cancel = || scalars.get() >= 3;

    let result = serialize_root_with_cancellation(&mut backend, Peek::new(&values), &should_cancel);

    assert!(matches!(result, Err(SerializeError::Cancelled)));
    assert_eq!(scalars.get(), 3);
    assert_ne!(backend.events.last().map(String::as_str), Some("]"));
}

#[test]
fn a_false_predicate_preserves_nested_and_flattened_output() {
    #[derive(Facet)]
    struct Details {
        owner: String,
        tags: Vec<String>,
    }
    #[derive(Facet)]
    struct Record {
        label: String,
        optional: Option<u32>,
        #[facet(flatten)]
        details: Details,
    }

    let value = Record {
        label: "example".into(),
        optional: Some(42),
        details: Details {
            owner: "local".into(),
            tags: vec!["a".into(), "b".into()],
        },
    };
    let scalars = Cell::new(0);
    let mut ordinary = Recorder::new(&scalars);
    let mut cancellable = Recorder::new(&scalars);
    serialize_root(&mut ordinary, Peek::new(&value)).unwrap();
    serialize_root_with_cancellation(&mut cancellable, Peek::new(&value), &|| false).unwrap();

    assert_eq!(ordinary.events, cancellable.events);
}

#[test]
fn omitted_fields_still_offer_cancellation_checkpoints() {
    #[derive(Facet)]
    struct Record {
        first: u32,
        second: u32,
        third: u32,
        fourth: u32,
    }
    let value = Record {
        first: 1,
        second: 2,
        third: 3,
        fourth: 4,
    };
    let scalars = Cell::new(0);
    let omitted = Cell::new(0);
    let mut backend = Recorder::new(&scalars);
    backend.omitted_fields = Some(&omitted);

    let result =
        serialize_root_with_cancellation(&mut backend, Peek::new(&value), &|| omitted.get() >= 2);

    assert!(matches!(result, Err(SerializeError::Cancelled)));
    assert_eq!(omitted.get(), 2);
    assert_eq!(scalars.get(), 0);
}

#[test]
fn a_backend_failure_is_preserved_when_it_also_requests_cancellation() {
    let scalars = Cell::new(0);
    let cancelled = Cell::new(false);
    let mut backend = Recorder::new(&scalars);
    backend.fail_and_cancel = Some(&cancelled);

    let result =
        serialize_root_with_cancellation(&mut backend, Peek::new(&42), &|| cancelled.get());

    assert!(matches!(
        result,
        Err(SerializeError::Backend(BackendError::Rejected))
    ));
}

thread_local! {
    static PROXY_CREATED: Cell<bool> = const { Cell::new(false) };
    static PROXY_DROPS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Facet)]
#[facet(proxy = DropProxy)]
struct Proxied(u32);

#[derive(Facet)]
struct DropProxy {
    value: u32,
}

impl From<&Proxied> for DropProxy {
    fn from(value: &Proxied) -> Self {
        PROXY_CREATED.set(true);
        Self { value: value.0 }
    }
}

impl From<DropProxy> for Proxied {
    fn from(value: DropProxy) -> Self {
        Self(value.value)
    }
}

impl Drop for DropProxy {
    fn drop(&mut self) {
        PROXY_DROPS.set(PROXY_DROPS.get() + 1);
    }
}

#[test]
fn a_proxy_is_dropped_when_cancellation_arrives_during_conversion() {
    PROXY_CREATED.set(false);
    PROXY_DROPS.set(0);
    let scalars = Cell::new(0);
    let mut backend = Recorder::new(&scalars);

    let result = serialize_root_with_cancellation(&mut backend, Peek::new(&Proxied(42)), &|| {
        PROXY_CREATED.get()
    });

    assert!(matches!(result, Err(SerializeError::Cancelled)));
    assert_eq!(PROXY_DROPS.get(), 1);
    assert_eq!(scalars.get(), 0);
}

#[test]
fn a_proxy_is_dropped_when_the_cancellation_predicate_unwinds_after_conversion() {
    PROXY_CREATED.set(false);
    PROXY_DROPS.set(0);
    let scalars = Cell::new(0);
    let mut backend = Recorder::new(&scalars);

    let result = catch_unwind(AssertUnwindSafe(|| {
        serialize_root_with_cancellation(&mut backend, Peek::new(&Proxied(42)), &|| {
            assert!(!PROXY_CREATED.get(), "synthetic cancellation checker panic");
            false
        })
    }));

    assert!(result.is_err());
    assert_eq!(PROXY_DROPS.get(), 1);
    assert_eq!(scalars.get(), 0);
}
