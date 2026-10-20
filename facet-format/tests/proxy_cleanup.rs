#![forbid(unsafe_code)]

use facet::Facet;
use facet_format::FormatSerializer;
use facet_format::ScalarValue;
use facet_format::SerializeError;
use facet_format::serialize_root;
use facet_reflect::Peek;
use std::cell::Cell;
use std::panic::AssertUnwindSafe;
use std::panic::catch_unwind;

thread_local! {
    static PROXY_CONVERSIONS: Cell<usize> = const { Cell::new(0) };
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
        PROXY_CONVERSIONS.set(PROXY_CONVERSIONS.get() + 1);
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

#[derive(Debug, PartialEq)]
enum BackendError {
    Rejected,
}

enum Outcome {
    Success,
    Error,
    Panic,
}

struct Backend {
    outcome: Outcome,
    scalars: usize,
}

impl FormatSerializer for Backend {
    type Error = BackendError;

    fn begin_struct(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn field_key(&mut self, _key: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end_struct(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn begin_seq(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end_seq(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn scalar(&mut self, _value: ScalarValue<'_>) -> Result<(), Self::Error> {
        self.scalars += 1;
        match self.outcome {
            Outcome::Success => Ok(()),
            Outcome::Error => Err(BackendError::Rejected),
            Outcome::Panic => panic!("synthetic backend panic while consuming the proxy"),
        }
    }
}

#[test]
fn a_successful_proxy_is_released_exactly_once() {
    PROXY_CONVERSIONS.set(0);
    PROXY_DROPS.set(0);
    let mut backend = Backend {
        outcome: Outcome::Success,
        scalars: 0,
    };

    serialize_root(&mut backend, Peek::new(&Proxied(42))).unwrap();

    assert_eq!(backend.scalars, 1);
    assert_eq!(PROXY_CONVERSIONS.get(), 1);
    assert_eq!(PROXY_DROPS.get(), 1);
}

#[test]
fn a_proxy_is_released_exactly_once_after_a_backend_error() {
    PROXY_CONVERSIONS.set(0);
    PROXY_DROPS.set(0);
    let mut backend = Backend {
        outcome: Outcome::Error,
        scalars: 0,
    };

    let result = serialize_root(&mut backend, Peek::new(&Proxied(42)));

    assert!(matches!(
        result,
        Err(SerializeError::Backend(BackendError::Rejected))
    ));
    assert_eq!(backend.scalars, 1);
    assert_eq!(PROXY_CONVERSIONS.get(), 1);
    assert_eq!(PROXY_DROPS.get(), 1);
}

#[test]
fn a_proxy_is_released_exactly_once_when_the_backend_unwinds() {
    PROXY_CONVERSIONS.set(0);
    PROXY_DROPS.set(0);
    let mut backend = Backend {
        outcome: Outcome::Panic,
        scalars: 0,
    };

    let result = catch_unwind(AssertUnwindSafe(|| {
        serialize_root(&mut backend, Peek::new(&Proxied(42)))
    }));

    assert!(result.is_err());
    assert_eq!(backend.scalars, 1);
    assert_eq!(PROXY_CONVERSIONS.get(), 1);
    assert_eq!(PROXY_DROPS.get(), 1);
}
