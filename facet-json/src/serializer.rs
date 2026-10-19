extern crate alloc;

use alloc::{string::String, vec::Vec};

use facet_core::Facet;
use facet_format::{
    FormatSerializer, ScalarValue, SerializeError, serialize_root, serialize_root_with_cancellation,
};
use facet_reflect::Peek;

/// Options for JSON serialization.
#[derive(Clone)]
pub struct SerializeOptions<'cancel> {
    /// Whether to pretty-print with indentation (default: false)
    pub pretty: bool,

    /// Indentation string for pretty-printing (default: "  ")
    pub indent: &'static str,

    /// How byte sequences (`Vec<u8>`, `[u8; N]`, `bytes::Bytes`, etc.) are serialized.
    pub bytes_format: BytesFormat,

    /// A borrowed cooperative checker: `true` requests cancellation.
    ///
    /// Checks run during traversal, bounded scalar chunks, and writer progress.
    /// No cancellation library or static lifetime is required. Allocations,
    /// user-provided callbacks, UTF-8 validation, and blocked writes cannot be
    /// preempted internally. `None` preserves the existing non-cancellable path.
    pub cancellation: Option<&'cancel (dyn Fn() -> bool + Send + Sync)>,
}

impl core::fmt::Debug for SerializeOptions<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SerializeOptions")
            .field("pretty", &self.pretty)
            .field("indent", &self.indent)
            .field("bytes_format", &self.bytes_format)
            .field("cancellation", &self.cancellation.map(|_| "<callback>"))
            .finish()
    }
}

impl Default for SerializeOptions<'_> {
    fn default() -> Self {
        Self {
            pretty: false,
            indent: "  ",
            bytes_format: BytesFormat::default(),
            cancellation: None,
        }
    }
}

/// Byte serialization format for JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum BytesFormat {
    /// Serialize as a JSON array of numbers (e.g., `[0, 255, 42]`).
    #[default]
    Array,
    /// Serialize as a JSON string containing hex bytes (e.g., `"0x00ff2a"`).
    Hex(HexBytesOptions),
}

/// Options for hex byte serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexBytesOptions {
    /// Truncate when the byte length exceeds this threshold.
    ///
    /// `None` means never truncate.
    pub truncate_above: Option<usize>,
    /// Number of bytes to keep from the start when truncating.
    pub head: usize,
    /// Number of bytes to keep from the end when truncating.
    pub tail: usize,
}

impl Default for HexBytesOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl HexBytesOptions {
    /// Create default hex byte options (no truncation).
    pub const fn new() -> Self {
        Self {
            truncate_above: None,
            head: 32,
            tail: 32,
        }
    }

    /// Truncate when byte length exceeds `truncate_above`.
    pub const fn truncate(mut self, truncate_above: usize) -> Self {
        self.truncate_above = Some(truncate_above);
        self
    }

    /// Set the number of head/tail bytes to keep when truncating.
    pub const fn head_tail(mut self, head: usize, tail: usize) -> Self {
        self.head = head;
        self.tail = tail;
        self
    }
}

impl SerializeOptions<'_> {
    /// Create new default options (compact output).
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach or replace a cooperative cancellation checker.
    ///
    /// This may shorten the callback's borrow when adapting cloned options to a
    /// local execution scope. The checker requires `Send + Sync`, preserving
    /// those auto traits on options and serializers without requiring `'static`.
    pub const fn cancellation<'cancel>(
        self,
        cancellation: &'cancel (dyn Fn() -> bool + Send + Sync),
    ) -> SerializeOptions<'cancel> {
        SerializeOptions {
            pretty: self.pretty,
            indent: self.indent,
            bytes_format: self.bytes_format,
            cancellation: Some(cancellation),
        }
    }

    /// Enable pretty-printing with default indentation.
    pub const fn pretty(mut self) -> Self {
        self.pretty = true;
        self
    }

    /// Set a custom indentation string (implies pretty-printing).
    pub const fn indent(mut self, indent: &'static str) -> Self {
        self.indent = indent;
        self.pretty = true;
        self
    }

    /// Configure how byte sequences are serialized.
    pub const fn bytes_format(mut self, bytes_format: BytesFormat) -> Self {
        self.bytes_format = bytes_format;
        self
    }

    /// Serialize byte sequences as JSON arrays of numbers.
    pub const fn bytes_as_array(mut self) -> Self {
        self.bytes_format = BytesFormat::Array;
        self
    }

    /// Serialize byte sequences as hex strings without truncation.
    pub const fn bytes_as_hex(mut self) -> Self {
        self.bytes_format = BytesFormat::Hex(HexBytesOptions::new());
        self
    }

    /// Serialize byte sequences as hex strings with custom options.
    pub const fn bytes_as_hex_with_options(mut self, options: HexBytesOptions) -> Self {
        self.bytes_format = BytesFormat::Hex(options);
        self
    }
}

#[derive(Debug)]
pub struct JsonSerializeError {
    msg: &'static str,
    cancelled: bool,
}

impl JsonSerializeError {
    /// Whether a low-level JSON scalar operation stopped at a cancellation checkpoint.
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    const fn cancelled() -> Self {
        Self {
            msg: "JSON serialization cancelled",
            cancelled: true,
        }
    }
}

impl core::fmt::Display for JsonSerializeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.msg)
    }
}

impl std::error::Error for JsonSerializeError {}

#[derive(Debug, Clone, Copy)]
enum Ctx {
    Struct { first: bool },
    Seq { first: bool },
}

/// JSON serializer with configurable formatting options.
pub struct JsonSerializer<'cancel> {
    out: Vec<u8>,
    stack: Vec<Ctx>,
    options: SerializeOptions<'cancel>,
}

impl<'cancel> JsonSerializer<'cancel> {
    /// Create a new JSON serializer with default (compact) options.
    pub fn new() -> Self {
        Self::with_options(SerializeOptions::default())
    }

    /// Create a new JSON serializer with the given options.
    pub const fn with_options(options: SerializeOptions<'cancel>) -> Self {
        Self {
            out: Vec::new(),
            stack: Vec::new(),
            options,
        }
    }
}

/// Input chunk budget between scalar cancellation checkpoints.
const CANCELLATION_CHUNK_SIZE: usize = 4096;

impl JsonSerializer<'_> {
    fn check_cancellation(&self) -> Result<(), JsonSerializeError> {
        if self.options.cancellation.is_some_and(|check| check()) {
            Err(JsonSerializeError::cancelled())
        } else {
            Ok(())
        }
    }

    fn write_raw_bytes(&mut self, bytes: &[u8]) -> Result<(), JsonSerializeError> {
        if self.options.cancellation.is_none() {
            self.out.extend_from_slice(bytes);
            return Ok(());
        }
        for chunk in bytes.chunks(CANCELLATION_CHUNK_SIZE) {
            self.check_cancellation()?;
            self.out.extend_from_slice(chunk);
        }
        self.check_cancellation()
    }

    /// Consume the serializer and return the output bytes.
    pub fn finish(self) -> Vec<u8> {
        self.out
    }

    /// Current nesting depth (for indentation).
    const fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Write a newline and indentation if in pretty mode.
    fn write_indent(&mut self) -> Result<(), JsonSerializeError> {
        if self.options.pretty {
            self.out.push(b'\n');
            for _ in 0..self.depth() {
                self.write_raw_bytes(self.options.indent.as_bytes())?;
            }
        }
        Ok(())
    }

    fn before_value(&mut self) -> Result<(), JsonSerializeError> {
        self.check_cancellation()?;
        match self.stack.last_mut() {
            Some(Ctx::Seq { first }) => {
                if !*first {
                    self.out.push(b',');
                }
                *first = false;
                self.write_indent()?;
            }
            Some(Ctx::Struct { .. }) => {
                // struct values are separated by `field_key`
            }
            None => {}
        }
        Ok(())
    }

    /// Optimized JSON string writing with SIMD-like 16-byte fast path.
    ///
    /// For ASCII strings without special characters, processes 16 bytes at a time.
    /// Falls back to character-by-character escaping when needed.
    fn write_json_string(&mut self, s: &str) -> Result<(), JsonSerializeError> {
        const STEP_SIZE: usize = 16; // u128 = 16 bytes
        type Chunk = [u8; STEP_SIZE];

        self.check_cancellation()?;
        self.out.push(b'"');

        let mut unchecked = 0;
        let mut s = s;
        while let Some(Ok(chunk)) = s.as_bytes().get(..STEP_SIZE).map(Chunk::try_from) {
            if unchecked >= CANCELLATION_CHUNK_SIZE {
                self.check_cancellation()?;
                unchecked = 0;
            }
            let previous_len = s.len();
            let window = u128::from_ne_bytes(chunk);
            // Check all 16 bytes at once:
            // 1. All ASCII (high bit clear): window & 0x80...80 == 0
            // 2. No quotes (0x22): !contains_byte(window, 0x22)
            // 3. No backslashes (0x5c): !contains_byte(window, 0x5c)
            // 4. No control chars (< 0x20): top 3 bits set for all bytes
            let completely_ascii = window & 0x80808080808080808080808080808080 == 0;
            let quote_free = !contains_byte(window, 0x22);
            let backslash_free = !contains_byte(window, 0x5c);
            let control_char_free = no_control_chars(window);

            if completely_ascii && quote_free && backslash_free && control_char_free {
                // Fast path: copy 16 bytes directly
                self.out.extend_from_slice(&chunk);
                s = &s[STEP_SIZE..];
            } else {
                // Slow path: escape character by character for this chunk
                let mut chars = s.chars();
                let mut count = STEP_SIZE;
                for c in &mut chars {
                    self.write_json_escaped_char(c);
                    count = count.saturating_sub(c.len_utf8());
                    if count == 0 {
                        break;
                    }
                }
                s = chars.as_str();
            }
            unchecked += previous_len - s.len();
        }

        // Handle remaining bytes (< 16)
        for c in s.chars() {
            self.write_json_escaped_char(c);
        }

        self.out.push(b'"');
        self.check_cancellation()
    }

    #[inline]
    fn write_json_escaped_char(&mut self, c: char) {
        match c {
            '"' => self.out.extend_from_slice(b"\\\""),
            '\\' => self.out.extend_from_slice(b"\\\\"),
            '\n' => self.out.extend_from_slice(b"\\n"),
            '\r' => self.out.extend_from_slice(b"\\r"),
            '\t' => self.out.extend_from_slice(b"\\t"),
            '\u{08}' => self.out.extend_from_slice(b"\\b"),
            '\u{0C}' => self.out.extend_from_slice(b"\\f"),
            c if c.is_ascii_control() => {
                let code_point = c as u32;
                let to_hex = |d: u32| {
                    if d < 10 {
                        b'0' + d as u8
                    } else {
                        b'a' + (d - 10) as u8
                    }
                };
                let buf = [
                    b'\\',
                    b'u',
                    to_hex((code_point >> 12) & 0xF),
                    to_hex((code_point >> 8) & 0xF),
                    to_hex((code_point >> 4) & 0xF),
                    to_hex(code_point & 0xF),
                ];
                self.out.extend_from_slice(&buf);
            }
            c if c.is_ascii() => {
                self.out.push(c as u8);
            }
            c => {
                let mut buf = [0u8; 4];
                let len = c.encode_utf8(&mut buf).len();
                self.out.extend_from_slice(&buf[..len]);
            }
        }
    }

    #[inline]
    fn write_hex_byte(&mut self, byte: u8) {
        let hi = byte >> 4;
        let lo = byte & 0x0f;
        self.out
            .push(if hi < 10 { b'0' + hi } else { b'a' + (hi - 10) });
        self.out
            .push(if lo < 10 { b'0' + lo } else { b'a' + (lo - 10) });
    }

    #[inline]
    fn write_u8_decimal(&mut self, value: u8) {
        #[cfg(feature = "fast")]
        self.out
            .extend_from_slice(itoa::Buffer::new().format(value).as_bytes());
        #[cfg(not(feature = "fast"))]
        self.out.extend_from_slice(value.to_string().as_bytes());
    }

    fn write_bytes_array(&mut self, bytes: &[u8]) -> Result<(), JsonSerializeError> {
        self.out.push(b'[');
        for (index, byte) in bytes.iter().copied().enumerate() {
            if index % 1024 == 0 {
                self.check_cancellation()?;
            }
            if index != 0 {
                self.out.push(b',');
            }
            self.write_u8_decimal(byte);
        }
        self.out.push(b']');
        self.check_cancellation()
    }

    fn write_bytes_hex(
        &mut self,
        bytes: &[u8],
        options: HexBytesOptions,
    ) -> Result<(), JsonSerializeError> {
        self.out.push(b'"');
        self.out.extend_from_slice(b"0x");

        let should_truncate = options
            .truncate_above
            .is_some_and(|max_len| bytes.len() > max_len);
        if !should_truncate {
            self.write_hex_bytes(bytes)?;
            self.out.push(b'"');
            return self.check_cancellation();
        }

        let head = options.head.min(bytes.len());
        let max_tail = bytes.len().saturating_sub(head);
        let tail = options.tail.min(max_tail);

        self.write_hex_bytes(&bytes[..head])?;

        let omitted = bytes.len().saturating_sub(head + tail);
        self.out.extend_from_slice(b"..<");
        #[cfg(feature = "fast")]
        self.out
            .extend_from_slice(itoa::Buffer::new().format(omitted).as_bytes());
        #[cfg(not(feature = "fast"))]
        self.out.extend_from_slice(omitted.to_string().as_bytes());
        self.out.extend_from_slice(b" bytes>..");

        if tail != 0 {
            self.write_hex_bytes(&bytes[bytes.len() - tail..])?;
        }

        self.out.push(b'"');
        self.check_cancellation()
    }

    fn write_hex_bytes(&mut self, bytes: &[u8]) -> Result<(), JsonSerializeError> {
        for chunk in bytes.chunks(1024) {
            self.check_cancellation()?;
            for byte in chunk {
                self.write_hex_byte(*byte);
            }
        }
        Ok(())
    }

    fn write_bytes_with_options(&mut self, bytes: &[u8]) -> Result<(), JsonSerializeError> {
        match self.options.bytes_format {
            BytesFormat::Array => self.write_bytes_array(bytes),
            BytesFormat::Hex(options) => self.write_bytes_hex(bytes, options),
        }
    }
}

/// Check if any byte in the u128 equals the target byte.
/// Uses the SWAR (SIMD Within A Register) technique.
#[inline]
const fn contains_byte(val: u128, byte: u8) -> bool {
    let mask = 0x01010101010101010101010101010101u128 * (byte as u128);
    let xor_result = val ^ mask;
    let has_zero = (xor_result.wrapping_sub(0x01010101010101010101010101010101))
        & !xor_result
        & 0x80808080808080808080808080808080;
    has_zero != 0
}

/// Check that all bytes have at least one of the top 3 bits set (i.e., >= 0x20).
/// This means no control characters (0x00-0x1F).
#[inline]
const fn no_control_chars(value: u128) -> bool {
    let masked = value & 0xe0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0;
    let has_zero = (masked.wrapping_sub(0x01010101010101010101010101010101))
        & !masked
        & 0x80808080808080808080808080808080;
    has_zero == 0
}

impl Default for JsonSerializer<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl FormatSerializer for JsonSerializer<'_> {
    type Error = JsonSerializeError;

    fn begin_struct(&mut self) -> Result<(), Self::Error> {
        self.before_value()?;
        self.out.push(b'{');
        self.stack.push(Ctx::Struct { first: true });
        Ok(())
    }

    fn field_key(&mut self, key: &str) -> Result<(), Self::Error> {
        match self.stack.last_mut() {
            Some(Ctx::Struct { first }) => {
                if !*first {
                    self.out.push(b',');
                }
                *first = false;
                self.write_indent()?;
                self.write_json_string(key)?;
                self.out.push(b':');
                if self.options.pretty {
                    self.out.push(b' ');
                }
                Ok(())
            }
            _ => Err(JsonSerializeError {
                msg: "field_key called outside of a struct",
                cancelled: false,
            }),
        }
    }

    fn end_struct(&mut self) -> Result<(), Self::Error> {
        match self.stack.pop() {
            Some(Ctx::Struct { first }) => {
                // Only add newline/indent before closing brace if struct was non-empty
                if !first {
                    self.write_indent()?;
                }
                self.check_cancellation()?;
                self.out.push(b'}');
                Ok(())
            }
            _ => Err(JsonSerializeError {
                msg: "end_struct called without matching begin_struct",
                cancelled: false,
            }),
        }
    }

    fn begin_seq(&mut self) -> Result<(), Self::Error> {
        self.before_value()?;
        self.out.push(b'[');
        self.stack.push(Ctx::Seq { first: true });
        Ok(())
    }

    fn end_seq(&mut self) -> Result<(), Self::Error> {
        match self.stack.pop() {
            Some(Ctx::Seq { first }) => {
                // Only add newline/indent before closing bracket if seq was non-empty
                if !first {
                    self.write_indent()?;
                }
                self.check_cancellation()?;
                self.out.push(b']');
                Ok(())
            }
            _ => Err(JsonSerializeError {
                msg: "end_seq called without matching begin_seq",
                cancelled: false,
            }),
        }
    }

    fn scalar(&mut self, scalar: ScalarValue<'_>) -> Result<(), Self::Error> {
        self.before_value()?;
        match scalar {
            ScalarValue::Null | ScalarValue::Unit => self.out.extend_from_slice(b"null"),
            ScalarValue::Bool(v) => {
                if v {
                    self.out.extend_from_slice(b"true")
                } else {
                    self.out.extend_from_slice(b"false")
                }
            }
            ScalarValue::Char(c) => {
                self.out.push(b'"');
                self.write_json_escaped_char(c);
                self.out.push(b'"');
            }
            ScalarValue::I64(v) => {
                #[cfg(feature = "fast")]
                self.out
                    .extend_from_slice(itoa::Buffer::new().format(v).as_bytes());
                #[cfg(not(feature = "fast"))]
                self.out.extend_from_slice(v.to_string().as_bytes());
            }
            ScalarValue::U64(v) => {
                #[cfg(feature = "fast")]
                self.out
                    .extend_from_slice(itoa::Buffer::new().format(v).as_bytes());
                #[cfg(not(feature = "fast"))]
                self.out.extend_from_slice(v.to_string().as_bytes());
            }
            ScalarValue::I128(v) => {
                #[cfg(feature = "fast")]
                self.out
                    .extend_from_slice(itoa::Buffer::new().format(v).as_bytes());
                #[cfg(not(feature = "fast"))]
                self.out.extend_from_slice(v.to_string().as_bytes());
            }
            ScalarValue::U128(v) => {
                #[cfg(feature = "fast")]
                self.out
                    .extend_from_slice(itoa::Buffer::new().format(v).as_bytes());
                #[cfg(not(feature = "fast"))]
                self.out.extend_from_slice(v.to_string().as_bytes());
            }
            ScalarValue::F64(v) => {
                if v.is_nan() || v.is_infinite() {
                    self.out.extend_from_slice(b"null");
                } else {
                    #[cfg(feature = "fast")]
                    self.out
                        .extend_from_slice(zmij::Buffer::new().format(v).as_bytes());
                    #[cfg(not(feature = "fast"))]
                    self.out.extend_from_slice(v.to_string().as_bytes());
                }
            }
            ScalarValue::Str(s) => self.write_json_string(&s)?,
            ScalarValue::Bytes(bytes) => self.write_bytes_with_options(bytes.as_ref())?,
            _ => {
                return Err(JsonSerializeError {
                    msg: "unsupported scalar value kind",
                    cancelled: false,
                });
            }
        }
        Ok(())
    }

    fn serialize_byte_sequence(&mut self, bytes: &[u8]) -> Result<bool, Self::Error> {
        self.before_value()?;
        self.write_bytes_with_options(bytes)?;
        Ok(true)
    }

    fn serialize_byte_array(&mut self, bytes: &[u8]) -> Result<bool, Self::Error> {
        self.serialize_byte_sequence(bytes)
    }

    fn raw_serialize_shape(&self) -> Option<&'static facet_core::Shape> {
        Some(crate::RawJson::SHAPE)
    }

    fn raw_scalar(&mut self, content: &str) -> Result<(), Self::Error> {
        // For RawJson, output the content directly without escaping
        self.before_value()?;
        self.write_raw_bytes(content.as_bytes())
    }

    fn format_namespace(&self) -> Option<&'static str> {
        Some("json")
    }
}

/// Serialize a value to JSON bytes.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::to_vec;
///
/// #[derive(Facet)]
/// struct Point { x: i32, y: i32 }
///
/// let point = Point { x: 10, y: 20 };
/// let bytes = to_vec(&point).unwrap();
/// assert_eq!(bytes, br#"{"x":10,"y":20}"#);
/// ```
pub fn to_vec<'facet, T>(value: &'_ T) -> Result<Vec<u8>, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    to_vec_with_options(value, &SerializeOptions::default())
}

/// Serialize a value to pretty-printed JSON bytes.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::to_vec_pretty;
///
/// #[derive(Facet)]
/// struct Point { x: i32, y: i32 }
///
/// let point = Point { x: 10, y: 20 };
/// let bytes = to_vec_pretty(&point).unwrap();
/// assert!(bytes.contains(&b'\n'));
/// ```
pub fn to_vec_pretty<'facet, T>(value: &'_ T) -> Result<Vec<u8>, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    to_vec_with_options(value, &SerializeOptions::default().pretty())
}

/// Serialize a value to JSON bytes with custom options.
///
/// A configured [`SerializeOptions::cancellation`] checker stops traversal and
/// bounded scalar work with [`SerializeError::Cancelled`]. The checker may
/// borrow caller state; allocations and user callbacks cannot be preempted.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::{to_vec_with_options, SerializeOptions};
///
/// #[derive(Facet)]
/// struct Point { x: i32, y: i32 }
///
/// let point = Point { x: 10, y: 20 };
///
/// // Compact output
/// let bytes = to_vec_with_options(&point, &SerializeOptions::default()).unwrap();
/// assert_eq!(bytes, br#"{"x":10,"y":20}"#);
///
/// // Pretty output with tabs
/// let bytes = to_vec_with_options(&point, &SerializeOptions::default().indent("\t")).unwrap();
/// assert!(bytes.contains(&b'\n'));
/// ```
pub fn to_vec_with_options<'facet, T>(
    value: &'_ T,
    options: &SerializeOptions<'_>,
) -> Result<Vec<u8>, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    let mut serializer = JsonSerializer::with_options(options.clone());
    if let Some(checker) = options.cancellation {
        serialize_root_with_cancellation(&mut serializer, Peek::new(value), checker)
            .map_err(normalize_cancellation)?;
    } else {
        serialize_root(&mut serializer, Peek::new(value))?;
    }
    Ok(serializer.finish())
}

/// Serialize a value to a JSON string.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::to_string;
///
/// #[derive(Facet)]
/// struct Person { name: String, age: u32 }
///
/// let person = Person { name: "Alice".into(), age: 30 };
/// let json = to_string(&person).unwrap();
/// assert_eq!(json, r#"{"name":"Alice","age":30}"#);
/// ```
pub fn to_string<'facet, T>(value: &'_ T) -> Result<String, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    to_string_with_options(value, &SerializeOptions::default())
}

/// Serialize a value to a pretty-printed JSON string.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::to_string_pretty;
///
/// #[derive(Facet)]
/// struct Person { name: String, age: u32 }
///
/// let person = Person { name: "Alice".into(), age: 30 };
/// let json = to_string_pretty(&person).unwrap();
/// assert!(json.contains('\n'));
/// ```
pub fn to_string_pretty<'facet, T>(
    value: &'_ T,
) -> Result<String, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    to_string_with_options(value, &SerializeOptions::default().pretty())
}

/// Serialize a value to a JSON string with custom options.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::{to_string_with_options, SerializeOptions};
///
/// #[derive(Facet)]
/// struct Person { name: String, age: u32 }
///
/// let person = Person { name: "Alice".into(), age: 30 };
///
/// // Compact output
/// let json = to_string_with_options(&person, &SerializeOptions::default()).unwrap();
/// assert_eq!(json, r#"{"name":"Alice","age":30}"#);
///
/// // Pretty output with tabs
/// let json = to_string_with_options(&person, &SerializeOptions::default().indent("\t")).unwrap();
/// assert!(json.contains('\n'));
/// ```
pub fn to_string_with_options<'facet, T>(
    value: &'_ T,
    options: &SerializeOptions<'_>,
) -> Result<String, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    let bytes = to_vec_with_options(value, options)?;
    finish_json_string(bytes, options.cancellation)
}

// ── Peek-based serialization ──

/// Serialize a `Peek` instance to a JSON string.
///
/// This allows serializing values without requiring ownership, useful when
/// you already have a `Peek` from reflection operations.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_reflect::Peek;
/// use facet_json::peek_to_string;
///
/// #[derive(Facet)]
/// struct Point { x: i32, y: i32 }
///
/// let point = Point { x: 10, y: 20 };
/// let json = peek_to_string(Peek::new(&point)).unwrap();
/// assert_eq!(json, r#"{"x":10,"y":20}"#);
/// ```
pub fn peek_to_string<'input, 'facet>(
    peek: Peek<'input, 'facet>,
) -> Result<String, SerializeError<JsonSerializeError>> {
    peek_to_string_with_options(peek, &SerializeOptions::default())
}

/// Serialize a `Peek` instance to a pretty-printed JSON string.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_reflect::Peek;
/// use facet_json::peek_to_string_pretty;
///
/// #[derive(Facet)]
/// struct Point { x: i32, y: i32 }
///
/// let point = Point { x: 10, y: 20 };
/// let json = peek_to_string_pretty(Peek::new(&point)).unwrap();
/// assert!(json.contains('\n'));
/// ```
pub fn peek_to_string_pretty<'input, 'facet>(
    peek: Peek<'input, 'facet>,
) -> Result<String, SerializeError<JsonSerializeError>> {
    peek_to_string_with_options(peek, &SerializeOptions::default().pretty())
}

/// Serialize a `Peek` instance to a JSON string with custom options.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_reflect::Peek;
/// use facet_json::{peek_to_string_with_options, SerializeOptions};
///
/// #[derive(Facet)]
/// struct Point { x: i32, y: i32 }
///
/// let point = Point { x: 10, y: 20 };
/// let json = peek_to_string_with_options(
///     Peek::new(&point),
///     &SerializeOptions::default().indent("\t"),
/// ).unwrap();
/// assert!(json.contains('\n'));
/// ```
pub fn peek_to_string_with_options<'input, 'facet>(
    peek: Peek<'input, 'facet>,
    options: &SerializeOptions<'_>,
) -> Result<String, SerializeError<JsonSerializeError>> {
    let mut serializer = JsonSerializer::with_options(options.clone());
    if let Some(checker) = options.cancellation {
        serialize_root_with_cancellation(&mut serializer, peek, checker)
            .map_err(normalize_cancellation)?;
    } else {
        serialize_root(&mut serializer, peek)?;
    }
    finish_json_string(serializer.finish(), options.cancellation)
}

// ── Writer-based serialization (std::io::Write) ──

/// Serialize a value to JSON and write it to a `std::io::Write` writer.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::to_writer_std;
///
/// #[derive(Facet)]
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// let person = Person { name: "Alice".into(), age: 30 };
/// let mut buffer = Vec::new();
/// to_writer_std(&mut buffer, &person).unwrap();
/// assert_eq!(buffer, br#"{"name":"Alice","age":30}"#);
/// ```
pub fn to_writer_std<'facet, W, T>(writer: W, value: &T) -> std::io::Result<()>
where
    W: std::io::Write,
    T: Facet<'facet> + ?Sized,
{
    peek_to_writer_std(writer, Peek::new(value))
}

/// Serialize a value to pretty-printed JSON and write it to a `std::io::Write` writer.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::to_writer_std_pretty;
///
/// #[derive(Facet)]
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// let person = Person { name: "Alice".into(), age: 30 };
/// let mut buffer = Vec::new();
/// to_writer_std_pretty(&mut buffer, &person).unwrap();
/// assert!(String::from_utf8_lossy(&buffer).contains('\n'));
/// ```
pub fn to_writer_std_pretty<'facet, W, T>(writer: W, value: &T) -> std::io::Result<()>
where
    W: std::io::Write,
    T: Facet<'facet> + ?Sized,
{
    peek_to_writer_std_pretty(writer, Peek::new(value))
}

/// Serialize a value to JSON with custom options and write it to a `std::io::Write` writer.
///
/// # Example
///
/// ```
/// use facet::Facet;
/// use facet_json::{to_writer_std_with_options, SerializeOptions};
///
/// #[derive(Facet)]
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// let person = Person { name: "Alice".into(), age: 30 };
///
/// // Compact output
/// let mut buffer = Vec::new();
/// to_writer_std_with_options(&mut buffer, &person, &SerializeOptions::default()).unwrap();
/// assert_eq!(buffer, br#"{"name":"Alice","age":30}"#);
///
/// // Pretty output with tabs
/// let mut buffer = Vec::new();
/// to_writer_std_with_options(&mut buffer, &person, &SerializeOptions::default().indent("\t")).unwrap();
/// assert!(String::from_utf8_lossy(&buffer).contains('\n'));
/// ```
pub fn to_writer_std_with_options<'facet, W, T>(
    writer: W,
    value: &T,
    options: &SerializeOptions<'_>,
) -> std::io::Result<()>
where
    W: std::io::Write,
    T: Facet<'facet> + ?Sized,
{
    peek_to_writer_std_with_options(writer, Peek::new(value), options)
}

/// Serialize a `Peek` instance to JSON and write it to a `std::io::Write` writer.
pub fn peek_to_writer_std<'input, 'facet, W>(
    writer: W,
    peek: Peek<'input, 'facet>,
) -> std::io::Result<()>
where
    W: std::io::Write,
{
    peek_to_writer_std_with_options(writer, peek, &SerializeOptions::default())
}

/// Serialize a `Peek` instance to pretty-printed JSON and write it to a `std::io::Write` writer.
pub fn peek_to_writer_std_pretty<'input, 'facet, W>(
    writer: W,
    peek: Peek<'input, 'facet>,
) -> std::io::Result<()>
where
    W: std::io::Write,
{
    peek_to_writer_std_with_options(writer, peek, &SerializeOptions::default().pretty())
}

/// Serialize a `Peek` instance to JSON with custom options and write it to a `std::io::Write` writer.
///
/// JSON is fully buffered before writing. With a cancellation checker, bounded
/// writes and `Interrupted` retries observe cancellation; an individual blocked
/// write cannot be preempted. Cancellation is an `Other` I/O error containing
/// [`SerializeError::Cancelled`], rather than a retryable `Interrupted` error.
/// A failed writer may retain a partial JSON prefix. Existing I/O failures win
/// over cancellation requested concurrently with the failed write.
pub fn peek_to_writer_std_with_options<'input, 'facet, W>(
    mut writer: W,
    peek: Peek<'input, 'facet>,
    options: &SerializeOptions<'_>,
) -> std::io::Result<()>
where
    W: std::io::Write,
{
    // Serialize to bytes first, then write
    // This is simpler and avoids the complexity of incremental writes
    let mut serializer = JsonSerializer::with_options(options.clone());
    if let Some(checker) = options.cancellation {
        serialize_root_with_cancellation(&mut serializer, peek, checker)
            .map_err(normalize_cancellation)
            .map_err(std::io::Error::other)?;
        write_json_bytes_with_cancellation(&mut writer, &serializer.finish(), checker)
    } else {
        serialize_root(&mut serializer, peek)
            .map_err(|e| std::io::Error::other(alloc::format!("{:?}", e)))?;
        writer.write_all(&serializer.finish())
    }
}

fn normalize_cancellation(
    error: SerializeError<JsonSerializeError>,
) -> SerializeError<JsonSerializeError> {
    match error {
        SerializeError::Backend(error) if error.is_cancelled() => SerializeError::Cancelled,
        other => other,
    }
}

fn finish_json_string(
    bytes: Vec<u8>,
    cancellation: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<String, SerializeError<JsonSerializeError>> {
    if cancellation.is_some_and(|check| check()) {
        return Err(SerializeError::Cancelled);
    }
    // The standard library validates UTF-8 synchronously; it cannot call the
    // checker during that scan, so observe cancellation at both boundaries.
    let json = String::from_utf8(bytes).expect("JSON output should always be valid UTF-8");
    if cancellation.is_some_and(|check| check()) {
        return Err(SerializeError::Cancelled);
    }
    Ok(json)
}

fn write_json_bytes_with_cancellation<W: std::io::Write>(
    writer: &mut W,
    bytes: &[u8],
    should_cancel: &(dyn Fn() -> bool + Send + Sync),
) -> std::io::Result<()> {
    for chunk in bytes.chunks(CANCELLATION_CHUNK_SIZE) {
        let mut remaining = chunk;
        while !remaining.is_empty() {
            check_write_cancellation(should_cancel)?;
            match writer.write(remaining) {
                Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
                Ok(written) => remaining = &remaining[written..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }
    check_write_cancellation(should_cancel)
}

fn check_write_cancellation(should_cancel: &dyn Fn() -> bool) -> std::io::Result<()> {
    if should_cancel() {
        Err(std::io::Error::other(
            SerializeError::<JsonSerializeError>::Cancelled,
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "serializer_cancellation_tests.rs"]
mod cancellation_tests;

#[cfg(test)]
mod tests {
    use facet::Facet;

    use super::{BytesFormat, HexBytesOptions, SerializeOptions, to_string_with_options};

    #[derive(Facet)]
    struct BytesVec {
        data: Vec<u8>,
    }

    #[derive(Facet)]
    struct BytesArray {
        data: [u8; 4],
    }

    #[test]
    fn bytes_default_to_json_array() {
        let value = BytesVec {
            data: vec![0, 127, 255],
        };

        let json = to_string_with_options(&value, &SerializeOptions::default()).unwrap();
        assert_eq!(json, r#"{"data":[0,127,255]}"#);
    }

    #[test]
    fn bytes_can_serialize_as_hex_string() {
        let value = BytesVec {
            data: vec![0x00, 0x7f, 0xff],
        };

        let json =
            to_string_with_options(&value, &SerializeOptions::default().bytes_as_hex()).unwrap();
        assert_eq!(json, r#"{"data":"0x007fff"}"#);
    }

    #[test]
    fn bytes_can_serialize_as_truncated_hex_string() {
        let value = BytesVec {
            data: (0u8..10).collect(),
        };

        let options = SerializeOptions::default()
            .bytes_as_hex_with_options(HexBytesOptions::new().truncate(6).head_tail(2, 2));
        let json = to_string_with_options(&value, &options).unwrap();
        assert_eq!(json, r#"{"data":"0x0001..<6 bytes>..0809"}"#);
    }

    #[test]
    fn byte_arrays_use_same_hex_mode() {
        let value = BytesArray {
            data: [0xaa, 0xbb, 0xcc, 0xdd],
        };

        let options =
            SerializeOptions::default().bytes_format(BytesFormat::Hex(HexBytesOptions::new()));
        let json = to_string_with_options(&value, &options).unwrap();
        assert_eq!(json, r#"{"data":"0xaabbccdd"}"#);
    }
}
