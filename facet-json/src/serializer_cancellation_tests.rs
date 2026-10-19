use super::*;
use alloc::borrow::Cow;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn cancellation_after(checks: &AtomicUsize, limit: usize) -> impl Fn() -> bool + Send + Sync + '_ {
    move || checks.fetch_add(1, Ordering::Relaxed) + 1 > limit
}

#[test]
fn large_ascii_and_escaped_utf8_strings_stop_inside_one_scalar() {
    for text in ["x".repeat(100_000), "雪\n\"é".repeat(25_000)] {
        let checks = AtomicUsize::new(0);
        let checker = cancellation_after(&checks, 4);
        let mut serializer =
            JsonSerializer::with_options(SerializeOptions::new().cancellation(&checker));
        let error = serializer
            .scalar(ScalarValue::Str(Cow::Borrowed(&text)))
            .unwrap_err();
        assert!(error.is_cancelled());
        assert!(!serializer.out.is_empty());
        assert!(serializer.out.len() < text.len());
    }
}

#[test]
fn large_byte_arrays_and_hex_stop_inside_one_scalar() {
    let bytes = vec![255; 100_000];
    for options in [
        SerializeOptions::new(),
        SerializeOptions::new().bytes_as_hex(),
        SerializeOptions::new().bytes_as_hex_with_options(
            HexBytesOptions::new().truncate(1).head_tail(50_000, 40_000),
        ),
    ] {
        let checks = AtomicUsize::new(0);
        let checker = cancellation_after(&checks, 4);
        let mut serializer = JsonSerializer::with_options(options.cancellation(&checker));
        let error = serializer.serialize_byte_sequence(&bytes).unwrap_err();
        assert!(error.is_cancelled());
        assert!(serializer.out.len() < bytes.len());
    }
}

#[test]
fn raw_json_and_custom_indentation_copy_in_interruptible_chunks() {
    let raw = " ".repeat(100_000);
    let checks = AtomicUsize::new(0);
    let checker = cancellation_after(&checks, 4);
    let mut serializer =
        JsonSerializer::with_options(SerializeOptions::new().cancellation(&checker));
    assert!(serializer.raw_scalar(&raw).unwrap_err().is_cancelled());
    assert!(serializer.out.len() < raw.len());

    // Deep nesting also makes indentation one large scalar-style copy loop.
    let checks = AtomicUsize::new(0);
    let checker = cancellation_after(&checks, 4);
    let mut pretty =
        JsonSerializer::with_options(SerializeOptions::new().pretty().cancellation(&checker));
    pretty.stack.resize(10_000, Ctx::Seq { first: true });
    assert!(pretty.write_indent().unwrap_err().is_cancelled());
    assert!(pretty.out.len() < 10_000);
}

#[test]
fn high_level_helpers_report_shared_cancellation_and_borrow_the_checker() {
    let value = "many characters".repeat(10_000);
    let checks = AtomicUsize::new(0);
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SerializeOptions<'static>>();
    assert_send_sync::<JsonSerializer<'static>>();
    let original: SerializeOptions<'static> = SerializeOptions::new();
    let checker = cancellation_after(&checks, 5);
    // Cloning options can attach a new checker with a shorter caller-owned borrow.
    let options = original.clone().cancellation(&checker);
    assert!(matches!(
        to_string_with_options(&value, &options),
        Err(SerializeError::Cancelled)
    ));
    assert!(checks.load(Ordering::Relaxed) > 5);
}

#[test]
fn a_false_checker_preserves_compact_pretty_bytes_and_hex_output() {
    let checks = AtomicUsize::new(0);
    let checker = || {
        checks.fetch_add(1, Ordering::Relaxed);
        false
    };
    let text = vec!["simple", "雪\"\n", "é💚"];
    for options in [
        SerializeOptions::new(),
        SerializeOptions::new().pretty(),
        SerializeOptions::new().indent("\t"),
    ] {
        assert_eq!(
            to_vec_with_options(&text, &options).unwrap(),
            to_vec_with_options(&text, &options.cancellation(&checker)).unwrap()
        );
    }
    let bytes = vec![0_u8, 255, 42, 17];
    for options in [
        SerializeOptions::new(),
        SerializeOptions::new().bytes_as_hex(),
        SerializeOptions::new()
            .bytes_as_hex_with_options(HexBytesOptions::new().truncate(2).head_tail(1, 1)),
    ] {
        assert_eq!(
            to_vec_with_options(&bytes, &options).unwrap(),
            to_vec_with_options(&bytes, &options.cancellation(&checker)).unwrap()
        );
    }
    assert!(checks.load(Ordering::Relaxed) > 0);
}

enum WriteOutcome {
    Short,
    Interrupted,
    Failure,
}

struct CancellingWriter<'a> {
    cancelled: &'a AtomicBool,
    outcome: WriteOutcome,
    bytes: Vec<u8>,
    calls: usize,
}

impl Write for CancellingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        self.cancelled.store(true, Ordering::Relaxed);
        match self.outcome {
            WriteOutcome::Short => {
                let len = bytes.len().min(4);
                self.bytes.extend_from_slice(&bytes[..len]);
                Ok(len)
            }
            WriteOutcome::Interrupted => Err(io::ErrorKind::Interrupted.into()),
            WriteOutcome::Failure => Err(io::ErrorKind::PermissionDenied.into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn standard_writer_cancellation_retains_a_typed_nonretryable_error() {
    for outcome in [WriteOutcome::Short, WriteOutcome::Interrupted] {
        let cancelled = AtomicBool::new(false);
        let mut writer = CancellingWriter {
            cancelled: &cancelled,
            outcome,
            bytes: Vec::new(),
            calls: 0,
        };
        let checker = || cancelled.load(Ordering::Relaxed);
        let options = SerializeOptions::new().cancellation(&checker);
        let error = to_writer_std_with_options(&mut writer, &"record".repeat(10_000), &options)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(matches!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<SerializeError<JsonSerializeError>>(),
            Some(SerializeError::Cancelled)
        ));
        assert_eq!(writer.calls, 1);
        assert!(writer.bytes.len() <= 4);
    }
}

#[test]
fn writer_failure_wins_over_cancellation_requested_during_the_failed_write() {
    let cancelled = AtomicBool::new(false);
    let mut writer = CancellingWriter {
        cancelled: &cancelled,
        outcome: WriteOutcome::Failure,
        bytes: Vec::new(),
        calls: 0,
    };
    let checker = || cancelled.load(Ordering::Relaxed);
    let options = SerializeOptions::new().cancellation(&checker);
    let error = to_writer_std_with_options(&mut writer, &"record", &options).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(writer.calls, 1);
}

#[test]
fn invalid_low_level_backend_use_keeps_its_original_error() {
    let checker = || true;
    let mut serializer =
        JsonSerializer::with_options(SerializeOptions::new().cancellation(&checker));
    let error = serializer.field_key("outside a struct").unwrap_err();
    assert!(!error.is_cancelled());
    assert_eq!(error.to_string(), "field_key called outside of a struct");
}

#[test]
fn bounded_writes_preserve_pretty_utf8_output_across_chunk_boundaries() {
    let values = vec!["雪💚\n".repeat(5_000), "another record".to_owned()];
    let expected = to_vec_pretty(&values).unwrap();
    let mut actual = Vec::new();
    let checker = || false;
    let options = SerializeOptions::new().pretty().cancellation(&checker);
    to_writer_std_with_options(&mut actual, &values, &options).unwrap();
    assert_eq!(actual, expected);
}
