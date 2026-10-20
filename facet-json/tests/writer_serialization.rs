use facet::Facet;
use facet_format::serialize_root;
use facet_json::{
    HexBytesOptions, JsonSerializer, SerializeOptions, to_vec_with_options, to_writer_std,
    to_writer_std_with_options,
};
use facet_reflect::Peek;
use std::cell::RefCell;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::rc::Rc;

#[derive(Default)]
struct Observation {
    bytes: Vec<u8>,
    bytes_at_conversion: Vec<usize>,
    flushes: usize,
}

#[derive(Facet)]
#[facet(opaque, proxy = ObservedProxy)]
struct ObservedValue {
    value: u32,
    observation: Rc<RefCell<Observation>>,
}

#[derive(Facet)]
#[facet(transparent)]
struct ObservedProxy(u32);

impl From<&ObservedValue> for ObservedProxy {
    fn from(value: &ObservedValue) -> Self {
        let mut observation = value.observation.borrow_mut();
        let byte_count = observation.bytes.len();
        observation.bytes_at_conversion.push(byte_count);
        Self(value.value)
    }
}

impl TryFrom<ObservedProxy> for ObservedValue {
    type Error = &'static str;

    fn try_from(_: ObservedProxy) -> Result<Self, Self::Error> {
        Err("serialization-only observation probe")
    }
}

#[derive(Debug)]
struct WriterFailure {
    operation: &'static str,
}

impl fmt::Display for WriterFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "synthetic {} failure", self.operation)
    }
}

impl std::error::Error for WriterFailure {}

struct ObservingWriter {
    observation: Rc<RefCell<Observation>>,
    fail_after: Option<usize>,
    fail_flush: bool,
}

impl Write for ObservingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut observation = self.observation.borrow_mut();
        let remaining = self.fail_after.map_or(usize::MAX, |limit| {
            limit.saturating_sub(observation.bytes.len())
        });
        if remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                WriterFailure { operation: "write" },
            ));
        }
        let written = bytes.len().min(remaining);
        observation.bytes.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.observation.borrow_mut().flushes += 1;
        if self.fail_flush {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                WriterFailure { operation: "flush" },
            ))
        } else {
            Ok(())
        }
    }
}

fn probes(observation: &Rc<RefCell<Observation>>) -> Vec<ObservedValue> {
    [11, 22, 33]
        .into_iter()
        .map(|value| ObservedValue {
            value,
            observation: Rc::clone(observation),
        })
        .collect()
}

fn assert_original_failure(error: &io::Error, kind: io::ErrorKind, operation: &str) {
    assert_eq!(error.kind(), kind);
    let original = error
        .get_ref()
        .and_then(|error| error.downcast_ref::<WriterFailure>())
        .expect("the original writer error payload must remain available");
    assert_eq!(original.operation, operation);
}

#[test]
fn the_writer_observes_a_prefix_before_later_values_are_converted() {
    let observation = Rc::new(RefCell::new(Observation::default()));
    let mut writer = ObservingWriter {
        observation: Rc::clone(&observation),
        fail_after: None,
        fail_flush: false,
    };

    to_writer_std(&mut writer, &probes(&observation)).unwrap();

    let observation = observation.borrow();
    assert_eq!(observation.bytes, b"[11,22,33]");
    assert_eq!(observation.bytes_at_conversion.len(), 3);
    assert!(observation.bytes_at_conversion[1] >= b"[11".len());
    assert!(observation.bytes_at_conversion[2] > observation.bytes_at_conversion[1]);
    assert_eq!(observation.flushes, 0);
}

#[test]
fn a_sink_failure_stops_traversal_and_retains_its_original_error() {
    let observation = Rc::new(RefCell::new(Observation::default()));
    let mut writer = ObservingWriter {
        observation: Rc::clone(&observation),
        fail_after: Some(2),
        fail_flush: false,
    };

    let error = to_writer_std(&mut writer, &probes(&observation)).unwrap_err();

    assert_original_failure(&error, io::ErrorKind::PermissionDenied, "write");
    let observation = observation.borrow();
    assert_eq!(observation.bytes.len(), 2);
    assert!(observation.bytes_at_conversion.len() < 3);
    assert_eq!(observation.flushes, 0);
}

#[derive(Default)]
struct ShortWriter {
    bytes: Vec<u8>,
    limit: usize,
    interruptions: usize,
    calls: usize,
    largest_request: usize,
}

impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        self.largest_request = self.largest_request.max(bytes.len());
        if self.interruptions > 0 {
            self.interruptions -= 1;
            return Err(io::ErrorKind::Interrupted.into());
        }
        let written = bytes.len().min(self.limit);
        self.bytes.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!("serialization must leave flushing to the caller")
    }
}

#[derive(Facet)]
struct Document {
    text: String,
    bytes: Vec<u8>,
    nested: Vec<Vec<String>>,
}

#[test]
fn short_writes_and_interruptions_preserve_compact_pretty_utf8_and_byte_output() {
    let document = Document {
        text: "a\\b\"雪💚\n\u{0000}\u{001f}".repeat(1_000),
        bytes: vec![0, 1, 17, 42, 127, 255],
        nested: vec![vec!["雪💚".to_owned(), "a record".to_owned()]],
    };
    for options in [
        SerializeOptions::new(),
        SerializeOptions::new().pretty(),
        SerializeOptions::new().indent("\t"),
        SerializeOptions::new().bytes_as_hex(),
        SerializeOptions::new()
            .pretty()
            .bytes_as_hex_with_options(HexBytesOptions::new().truncate(3).head_tail(2, 1)),
    ] {
        let expected = to_vec_with_options(&document, &options).unwrap();
        let mut writer = ShortWriter {
            limit: 3,
            interruptions: 2,
            ..ShortWriter::default()
        };

        to_writer_std_with_options(&mut writer, &document, &options).unwrap();

        assert_eq!(writer.bytes, expected);
        assert_eq!(writer.interruptions, 0);
        assert!(writer.calls > 2);
    }
}

struct InterruptedThenFailure {
    calls: usize,
}

impl Write for InterruptedThenFailure {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Err(io::Error::other(WriterFailure { operation: "stop" }))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!("serialization must leave flushing to the caller")
    }
}

#[test]
fn a_nonretryable_writer_error_after_an_interruption_keeps_its_payload() {
    let mut writer = InterruptedThenFailure { calls: 0 };

    let error = to_writer_std(&mut writer, &"not emitted").unwrap_err();

    assert_eq!(writer.calls, 2);
    assert_original_failure(&error, io::ErrorKind::Other, "stop");
}

#[test]
fn a_sink_that_cannot_make_progress_returns_write_zero() {
    let mut writer = ShortWriter::default();

    let error = to_writer_std(&mut writer, &[1, 2, 3]).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::WriteZero);
    assert_eq!(writer.calls, 1);
    assert!(writer.bytes.is_empty());
}

#[test]
fn caller_buffering_and_flush_failures_remain_owned_by_the_caller() {
    let observation = Rc::new(RefCell::new(Observation::default()));
    let writer = ObservingWriter {
        observation: Rc::clone(&observation),
        fail_after: None,
        fail_flush: true,
    };
    let mut buffered = BufWriter::with_capacity(4_096, writer);

    to_writer_std(&mut buffered, &[1, 2, 3]).unwrap();

    assert!(observation.borrow().bytes.is_empty());
    assert_eq!(observation.borrow().flushes, 0);
    assert_eq!(buffered.buffer(), b"[1,2,3]");
    let error = buffered.flush().unwrap_err();
    assert_original_failure(&error, io::ErrorKind::BrokenPipe, "flush");
    assert_eq!(observation.borrow().bytes, b"[1,2,3]");
    assert_eq!(observation.borrow().flushes, 1);
}

#[test]
fn finishing_a_generic_serializer_returns_its_sink_without_flushing() {
    let observation = Rc::new(RefCell::new(Observation::default()));
    let writer = ObservingWriter {
        observation: Rc::clone(&observation),
        fail_after: None,
        fail_flush: false,
    };
    let mut serializer = JsonSerializer::from_writer(writer, SerializeOptions::new());
    serialize_root(&mut serializer, Peek::new(&[1, 2, 3])).unwrap();

    let mut writer: ObservingWriter = serializer.finish();

    assert_eq!(observation.borrow().bytes, b"[1,2,3]");
    assert_eq!(observation.borrow().flushes, 0);
    writer.flush().unwrap();
    assert_eq!(observation.borrow().flushes, 1);
}

#[test]
fn large_strings_emit_bounded_runs_with_valid_utf8_and_escaping() {
    for text in [
        "x".repeat(100_000),
        "雪💚é".repeat(25_000),
        "雪\n\"é".repeat(25_000),
    ] {
        let mut writer = ShortWriter {
            limit: usize::MAX,
            ..ShortWriter::default()
        };

        to_writer_std(&mut writer, &text).unwrap();

        assert!(writer.largest_request <= 4096);
        assert!(writer.calls > 2);
        assert_eq!(
            serde_json::from_slice::<String>(&writer.bytes).unwrap(),
            text
        );
    }
}

#[test]
fn del_escaping_is_consistent_across_scan_and_write_boundaries() {
    for prefix_length in [0, 15, 16, 4095, 4096] {
        let padding = "x".repeat(prefix_length);
        let value = format!("{padding}\u{7f}");
        let expected = format!("\"{padding}\\u007f\"");
        assert_eq!(facet_json::to_string(&value).unwrap(), expected);
    }
}
