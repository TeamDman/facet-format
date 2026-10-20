extern crate alloc;

use alloc::{string::String, vec::Vec};

use facet_core::Facet;
use facet_format::{FormatSerializer, ScalarValue, SerializeError, serialize_root};
use facet_reflect::Peek;

/// Options for JSON serialization.
#[derive(Debug, Clone)]
pub struct SerializeOptions {
    /// Whether to pretty-print with indentation (default: false)
    pub pretty: bool,

    /// Indentation string for pretty-printing (default: "  ")
    pub indent: &'static str,

    /// How byte sequences (`Vec<u8>`, `[u8; N]`, `bytes::Bytes`, etc.) are serialized.
    pub bytes_format: BytesFormat,
}

impl Default for SerializeOptions {
    fn default() -> Self {
        Self {
            pretty: false,
            indent: "  ",
            bytes_format: BytesFormat::default(),
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

impl SerializeOptions {
    /// Create new default options (compact output).
    pub fn new() -> Self {
        Self::default()
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

/// A JSON encoding or destination-writing failure.
///
/// Low-level I/O failures expose the original writer error through
/// [`std::error::Error::source`]. Standard writer helpers return that original
/// error directly, preserving its kind and custom payload.
#[derive(Debug)]
pub struct JsonSerializeError {
    msg: &'static str,
    io_error: Option<std::io::Error>,
}

impl JsonSerializeError {
    fn from_io(error: std::io::Error) -> Self {
        Self {
            msg: "JSON output write failed",
            io_error: Some(error),
        }
    }
}

impl core::fmt::Display for JsonSerializeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(error) = &self.io_error {
            error.fmt(f)
        } else {
            f.write_str(self.msg)
        }
    }
}

impl std::error::Error for JsonSerializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error
            .as_ref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

#[derive(Debug, Clone, Copy)]
enum Ctx {
    Struct { first: bool },
    Seq { first: bool },
}

/// JSON serializer with configurable formatting and a caller-owned output sink.
///
/// [`Self::new`] and [`Self::with_options`] collect bytes in the default `Vec`.
/// [`Self::from_writer`] emits directly into its writer, without an internal
/// output buffer or implicit flush. A failed operation may leave a JSON prefix.
pub struct JsonSerializer<W = Vec<u8>> {
    out: W,
    stack: Vec<Ctx>,
    options: SerializeOptions,
}

impl JsonSerializer {
    /// Create a new JSON serializer with default (compact) options.
    pub fn new() -> Self {
        Self::with_options(SerializeOptions::default())
    }

    /// Create a new JSON serializer with the given options.
    pub const fn with_options(options: SerializeOptions) -> Self {
        Self {
            out: Vec::new(),
            stack: Vec::new(),
            options,
        }
    }
}

/// Maximum borrowed string run emitted before scanning the next run.
const STRING_CHUNK_SIZE: usize = 4096;

impl<W: std::io::Write> JsonSerializer<W> {
    /// Serialize directly into a caller-supplied writer with existing options.
    ///
    /// No output buffer is installed and writes may be small. Callers choose
    /// buffering explicitly, usually by passing `&mut BufWriter<_>`. Neither
    /// serialization nor [`Self::finish`] calls the writer's `flush` method.
    pub const fn from_writer(writer: W, options: SerializeOptions) -> Self {
        Self {
            out: writer,
            stack: Vec::new(),
            options,
        }
    }

    fn write_raw_bytes(&mut self, bytes: &[u8]) -> Result<(), JsonSerializeError> {
        self.out
            .write_all(bytes)
            .map_err(JsonSerializeError::from_io)
    }

    fn write_byte(&mut self, byte: u8) -> Result<(), JsonSerializeError> {
        self.write_raw_bytes(&[byte])
    }

    /// Return the output sink without flushing it.
    ///
    /// The default sink is a `Vec<u8>`. A caller-supplied writer is returned as-is;
    /// the caller retains responsibility for buffering, flushing, and persistence.
    pub fn finish(self) -> W {
        self.out
    }

    /// Current nesting depth (for indentation).
    const fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Write a newline and indentation if in pretty mode.
    fn write_indent(&mut self) -> Result<(), JsonSerializeError> {
        if self.options.pretty {
            self.write_byte(b'\n')?;
            for _ in 0..self.depth() {
                self.write_raw_bytes(self.options.indent.as_bytes())?;
            }
        }
        Ok(())
    }

    fn before_value(&mut self) -> Result<(), JsonSerializeError> {
        match self.stack.last_mut() {
            Some(Ctx::Seq { first }) => {
                let needs_comma = !*first;
                *first = false;
                if needs_comma {
                    self.write_byte(b',')?;
                }
                self.write_indent()?;
            }
            Some(Ctx::Struct { .. }) => {
                // struct values are separated by `field_key`
            }
            None => {}
        }
        Ok(())
    }

    /// Scan and write borrowed unescaped runs without buffering the full string.
    ///
    /// JSON escapes are ASCII bytes. Non-ASCII UTF-8 bytes can pass through
    /// unchanged, including when a writer splits a run inside a code point.
    fn write_json_string(&mut self, s: &str) -> Result<(), JsonSerializeError> {
        const STEP_SIZE: usize = 16; // u128 = 16 bytes
        type Chunk = [u8; STEP_SIZE];
        self.write_byte(b'"')?;

        let bytes = s.as_bytes();
        let mut run_start = 0_usize;
        let mut cursor = 0;
        while cursor < bytes.len() {
            let run_end = run_start.saturating_add(STRING_CHUNK_SIZE).min(bytes.len());
            while let Some(Ok(chunk)) = bytes[cursor..run_end].get(..STEP_SIZE).map(Chunk::try_from)
            {
                let window = u128::from_ne_bytes(chunk);
                if contains_byte(window, b'"')
                    || contains_byte(window, b'\\')
                    || contains_byte(window, 0x7f)
                    || !no_control_chars(window)
                {
                    break;
                }
                cursor += STEP_SIZE;
            }
            // Finish a short window or find the exact escape within a SWAR window.
            while cursor < run_end
                && bytes[cursor] != b'"'
                && bytes[cursor] != b'\\'
                && bytes[cursor] != 0x7f
                && bytes[cursor] >= 0x20
            {
                cursor += 1;
            }

            if cursor > run_start {
                self.write_raw_bytes(&bytes[run_start..cursor])?;
            }
            if cursor < run_end {
                // Only ASCII quote, backslash, and control bytes reach this branch.
                self.write_json_escaped_char(char::from(bytes[cursor]))?;
                cursor += 1;
            }
            run_start = cursor;
        }

        self.write_byte(b'"')?;
        Ok(())
    }

    #[inline]
    fn write_json_escaped_char(&mut self, c: char) -> Result<(), JsonSerializeError> {
        match c {
            '"' => self.write_raw_bytes(b"\\\"")?,
            '\\' => self.write_raw_bytes(b"\\\\")?,
            '\n' => self.write_raw_bytes(b"\\n")?,
            '\r' => self.write_raw_bytes(b"\\r")?,
            '\t' => self.write_raw_bytes(b"\\t")?,
            '\u{08}' => self.write_raw_bytes(b"\\b")?,
            '\u{0C}' => self.write_raw_bytes(b"\\f")?,
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
                self.write_raw_bytes(&buf)?;
            }
            c if c.is_ascii() => {
                self.write_byte(c as u8)?;
            }
            c => {
                let mut buf = [0u8; 4];
                let len = c.encode_utf8(&mut buf).len();
                self.write_raw_bytes(&buf[..len])?;
            }
        }
        Ok(())
    }

    #[inline]
    fn write_hex_byte(&mut self, byte: u8) -> Result<(), JsonSerializeError> {
        let hi = byte >> 4;
        let lo = byte & 0x0f;
        self.write_byte(if hi < 10 { b'0' + hi } else { b'a' + (hi - 10) })?;
        self.write_byte(if lo < 10 { b'0' + lo } else { b'a' + (lo - 10) })?;
        Ok(())
    }

    #[inline]
    fn write_u8_decimal(&mut self, value: u8) -> Result<(), JsonSerializeError> {
        #[cfg(feature = "fast")]
        self.write_raw_bytes(itoa::Buffer::new().format(value).as_bytes())?;
        #[cfg(not(feature = "fast"))]
        self.write_raw_bytes(value.to_string().as_bytes())?;
        Ok(())
    }

    fn write_bytes_array(&mut self, bytes: &[u8]) -> Result<(), JsonSerializeError> {
        self.write_byte(b'[')?;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if index != 0 {
                self.write_byte(b',')?;
            }
            self.write_u8_decimal(byte)?;
        }
        self.write_byte(b']')?;
        Ok(())
    }

    fn write_bytes_hex(
        &mut self,
        bytes: &[u8],
        options: HexBytesOptions,
    ) -> Result<(), JsonSerializeError> {
        self.write_byte(b'"')?;
        self.write_raw_bytes(b"0x")?;

        let should_truncate = options
            .truncate_above
            .is_some_and(|max_len| bytes.len() > max_len);
        if !should_truncate {
            self.write_hex_bytes(bytes)?;
            self.write_byte(b'"')?;
            return Ok(());
        }

        let head = options.head.min(bytes.len());
        let max_tail = bytes.len().saturating_sub(head);
        let tail = options.tail.min(max_tail);

        self.write_hex_bytes(&bytes[..head])?;

        let omitted = bytes.len().saturating_sub(head + tail);
        self.write_raw_bytes(b"..<")?;
        #[cfg(feature = "fast")]
        self.write_raw_bytes(itoa::Buffer::new().format(omitted).as_bytes())?;
        #[cfg(not(feature = "fast"))]
        self.write_raw_bytes(omitted.to_string().as_bytes())?;
        self.write_raw_bytes(b" bytes>..")?;

        if tail != 0 {
            self.write_hex_bytes(&bytes[bytes.len() - tail..])?;
        }

        self.write_byte(b'"')?;
        Ok(())
    }

    fn write_hex_bytes(&mut self, bytes: &[u8]) -> Result<(), JsonSerializeError> {
        for byte in bytes {
            self.write_hex_byte(*byte)?;
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

impl Default for JsonSerializer {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: std::io::Write> FormatSerializer for JsonSerializer<W> {
    type Error = JsonSerializeError;

    fn begin_struct(&mut self) -> Result<(), Self::Error> {
        self.before_value()?;
        self.write_byte(b'{')?;
        self.stack.push(Ctx::Struct { first: true });
        Ok(())
    }

    fn field_key(&mut self, key: &str) -> Result<(), Self::Error> {
        match self.stack.last_mut() {
            Some(Ctx::Struct { first }) => {
                let needs_comma = !*first;
                *first = false;
                if needs_comma {
                    self.write_byte(b',')?;
                }
                self.write_indent()?;
                self.write_json_string(key)?;
                self.write_byte(b':')?;
                if self.options.pretty {
                    self.write_byte(b' ')?;
                }
                Ok(())
            }
            _ => Err(JsonSerializeError {
                msg: "field_key called outside of a struct",
                io_error: None,
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
                self.write_byte(b'}')?;
                Ok(())
            }
            _ => Err(JsonSerializeError {
                msg: "end_struct called without matching begin_struct",
                io_error: None,
            }),
        }
    }

    fn begin_seq(&mut self) -> Result<(), Self::Error> {
        self.before_value()?;
        self.write_byte(b'[')?;
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
                self.write_byte(b']')?;
                Ok(())
            }
            _ => Err(JsonSerializeError {
                msg: "end_seq called without matching begin_seq",
                io_error: None,
            }),
        }
    }

    fn scalar(&mut self, scalar: ScalarValue<'_>) -> Result<(), Self::Error> {
        self.before_value()?;
        match scalar {
            ScalarValue::Null | ScalarValue::Unit => self.write_raw_bytes(b"null")?,
            ScalarValue::Bool(v) => {
                if v {
                    self.write_raw_bytes(b"true")?
                } else {
                    self.write_raw_bytes(b"false")?
                }
            }
            ScalarValue::Char(c) => {
                self.write_byte(b'"')?;
                self.write_json_escaped_char(c)?;
                self.write_byte(b'"')?;
            }
            ScalarValue::I64(v) => {
                #[cfg(feature = "fast")]
                self.write_raw_bytes(itoa::Buffer::new().format(v).as_bytes())?;
                #[cfg(not(feature = "fast"))]
                self.write_raw_bytes(v.to_string().as_bytes())?;
            }
            ScalarValue::U64(v) => {
                #[cfg(feature = "fast")]
                self.write_raw_bytes(itoa::Buffer::new().format(v).as_bytes())?;
                #[cfg(not(feature = "fast"))]
                self.write_raw_bytes(v.to_string().as_bytes())?;
            }
            ScalarValue::I128(v) => {
                #[cfg(feature = "fast")]
                self.write_raw_bytes(itoa::Buffer::new().format(v).as_bytes())?;
                #[cfg(not(feature = "fast"))]
                self.write_raw_bytes(v.to_string().as_bytes())?;
            }
            ScalarValue::U128(v) => {
                #[cfg(feature = "fast")]
                self.write_raw_bytes(itoa::Buffer::new().format(v).as_bytes())?;
                #[cfg(not(feature = "fast"))]
                self.write_raw_bytes(v.to_string().as_bytes())?;
            }
            ScalarValue::F64(v) => {
                if v.is_nan() || v.is_infinite() {
                    self.write_raw_bytes(b"null")?;
                } else {
                    #[cfg(feature = "fast")]
                    self.write_raw_bytes(zmij::Buffer::new().format(v).as_bytes())?;
                    #[cfg(not(feature = "fast"))]
                    self.write_raw_bytes(v.to_string().as_bytes())?;
                }
            }
            ScalarValue::Str(s) => self.write_json_string(&s)?,
            ScalarValue::Bytes(bytes) => self.write_bytes_with_options(bytes.as_ref())?,
            _ => {
                return Err(JsonSerializeError {
                    msg: "unsupported scalar value kind",
                    io_error: None,
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
    options: &SerializeOptions,
) -> Result<Vec<u8>, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    let mut serializer = JsonSerializer::with_options(options.clone());
    serialize_root(&mut serializer, Peek::new(value))?;
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
    options: &SerializeOptions,
) -> Result<String, SerializeError<JsonSerializeError>>
where
    T: Facet<'facet> + ?Sized,
{
    let bytes = to_vec_with_options(value, options)?;
    Ok(String::from_utf8(bytes).expect("JSON output should always be valid UTF-8"))
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
    options: &SerializeOptions,
) -> Result<String, SerializeError<JsonSerializeError>> {
    let mut serializer = JsonSerializer::with_options(options.clone());
    serialize_root(&mut serializer, peek)?;
    Ok(String::from_utf8(serializer.finish()).expect("JSON output should always be valid UTF-8"))
}

// ── Writer-based serialization (std::io::Write) ──

/// Serialize a value to JSON and write it to a `std::io::Write` writer.
///
/// Writes directly without installing a buffer or calling `flush`. Callers may
/// pass a borrowed `BufWriter` and flush it explicitly. See
/// [`peek_to_writer_std_with_options`] for partial-output and error behavior.
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
/// Writes directly without installing a buffer or calling `flush`. Callers may
/// pass a borrowed `BufWriter` and flush it explicitly. See
/// [`peek_to_writer_std_with_options`] for partial-output and error behavior.
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
/// Writes directly without installing a buffer or calling `flush`. Callers may
/// pass a borrowed `BufWriter` and flush it explicitly. See
/// [`peek_to_writer_std_with_options`] for partial-output and error behavior.
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
    options: &SerializeOptions,
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
/// Serialization writes directly to the caller's sink, with no internal output
/// buffer or implicit flush. Writes may be small; callers choose buffering and
/// flush it explicitly. Serialization or I/O failure may leave a partial JSON
/// prefix. Original writer error kinds and typed payloads are preserved.
/// Writer adapters may enforce cancellation by returning a nonretryable error.
/// Serialization cannot interrupt work between writer calls or a blocked write.
pub fn peek_to_writer_std_with_options<'input, 'facet, W>(
    writer: W,
    peek: Peek<'input, 'facet>,
    options: &SerializeOptions,
) -> std::io::Result<()>
where
    W: std::io::Write,
{
    let mut serializer = JsonSerializer::from_writer(writer, options.clone());
    serialize_root(&mut serializer, peek).map_err(serialization_io_error)
}

fn serialization_io_error(error: SerializeError<JsonSerializeError>) -> std::io::Error {
    match error {
        SerializeError::Backend(mut backend) => match backend.io_error.take() {
            Some(error) => error,
            None => std::io::Error::other(SerializeError::Backend(backend)),
        },
        other => std::io::Error::other(other),
    }
}

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
