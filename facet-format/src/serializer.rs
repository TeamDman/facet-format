extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use core::fmt::Debug;
use core::fmt::Write as _;

use facet_core::{Def, DynDateTimeKind, DynValueKind, ScalarType, Shape, StructKind, Type};
use facet_reflect::{
    HasFields as _, Peek, PeekListLike, PeekMap, PeekOption, PeekResult, PeekSet, ReflectError,
};

use crate::ScalarValue;

/// Field ordering preference for serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FieldOrdering {
    /// Fields are serialized in declaration order (default).
    #[default]
    Declaration,
}

/// How struct fields should be serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StructFieldMode {
    /// Serialize fields with names/keys (default for text formats).
    #[default]
    Named,
    /// Serialize fields in declaration order without names (binary formats).
    Unnamed,
}

/// How map-like values should be serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MapEncoding {
    /// Serialize maps as objects/structs with string keys.
    #[default]
    Struct,
    /// Serialize maps as key/value pairs (binary formats).
    Pairs,
}

/// How enum variants should be serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum EnumVariantEncoding {
    /// Serialize enums using tag/field-name strategies (default for text formats).
    #[default]
    Tagged,
    /// Serialize enums using a numeric variant index followed by fields (binary formats).
    Index,
}

/// How dynamic values (e.g. `facet_value::Value`) should be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DynamicValueEncoding {
    /// Use the format's native self-describing encoding (default for JSON, MsgPack, etc.).
    #[default]
    SelfDescribing,
    /// Use an explicit type tag before the dynamic value payload (binary formats).
    Tagged,
}

/// Tag describing the concrete payload type for a dynamic value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DynamicValueTag {
    /// Null value.
    Null,
    /// Boolean value.
    Bool,
    /// Signed 64-bit integer.
    I64,
    /// Unsigned 64-bit integer.
    U64,
    /// 64-bit float.
    F64,
    /// UTF-8 string.
    String,
    /// Raw bytes.
    Bytes,
    /// Sequence/array.
    Array,
    /// Object/map.
    Object,
    /// Date/time value (encoded as string for tagged formats).
    DateTime,
}

/// Low-level serializer interface implemented by each format backend.
///
/// This is intentionally event-ish: the shared serializer logic owns traversal
/// (struct/enum/seq decisions), while formats own representation details.
pub trait FormatSerializer {
    /// Format-specific error type.
    type Error: Debug;

    /// Begin a map/object/struct.
    fn begin_struct(&mut self) -> Result<(), Self::Error>;
    /// Emit a field key within a struct.
    fn field_key(&mut self, key: &str) -> Result<(), Self::Error>;
    /// Emit a rich field key with optional tag and documentation.
    ///
    /// This is called when serializing map keys that have been extracted from
    /// metadata containers (like `ObjectKey` with tag support).
    ///
    /// Default implementation ignores tag and doc, just emits the name.
    /// Formats that support tags (like Styx) should override this.
    fn emit_field_key(&mut self, key: &crate::FieldKey<'_>) -> Result<(), Self::Error> {
        // Default: ignore tag and doc, just emit the name (empty string if None)
        let name = key.name().map(|c| c.as_ref()).unwrap_or("");
        self.field_key(name)
    }
    /// End a map/object/struct.
    fn end_struct(&mut self) -> Result<(), Self::Error>;

    /// Begin a sequence/array.
    fn begin_seq(&mut self) -> Result<(), Self::Error>;
    /// End a sequence/array.
    fn end_seq(&mut self) -> Result<(), Self::Error>;

    /// Emit a scalar value.
    fn scalar(&mut self, scalar: ScalarValue<'_>) -> Result<(), Self::Error>;

    /// Optional: Whether to omit a struct field entirely (no key, no value).
    ///
    /// Lets a format drop fields it's configured to skip — e.g. `None` options
    /// when an "omit none" option is set. Called for each named-struct field
    /// before it's written. Default: never omit.
    fn should_omit_field(&self, _field: &facet_reflect::FieldItem, _value: Peek<'_, '_>) -> bool {
        false
    }

    /// Optional: Provide field metadata before field_key is called.
    /// Default implementation does nothing.
    fn field_metadata(&mut self, _field: &facet_reflect::FieldItem) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Optional: Provide field metadata with access to the field value.
    ///
    /// This is called before `field_key` and allows formats to inspect the field value
    /// for metadata. This is particularly useful for metadata containers like `Documented<T>`
    /// where doc comments are stored in the value, not the field definition.
    ///
    /// If this returns `Ok(true)`, the field key has been written and `field_key` will be skipped.
    /// If this returns `Ok(false)`, normal field_key handling continues.
    ///
    /// Default implementation does nothing and returns `Ok(false)`.
    fn field_metadata_with_value(
        &mut self,
        _field: &facet_reflect::FieldItem,
        _value: Peek<'_, '_>,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Optional: Provide struct/enum type metadata when beginning to serialize it.
    /// Default implementation does nothing.
    fn struct_metadata(&mut self, _shape: &facet_core::Shape) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Optional: Provide variant metadata before serializing an enum variant.
    /// Default implementation does nothing.
    fn variant_metadata(
        &mut self,
        _variant: &'static facet_core::Variant,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Serialize a metadata container value.
    ///
    /// Metadata containers (structs with `#[facet(metadata_container)]`) have exactly
    /// one non-metadata field (the actual value) and one or more metadata fields
    /// (like doc comments or source spans).
    ///
    /// Formats that support metadata can override this to emit metadata in the
    /// appropriate position. For example, Styx emits doc comments before the value:
    ///
    /// ```text
    /// /// The port to listen on
    /// port 8080
    /// ```
    ///
    /// The format is responsible for:
    /// 1. Extracting metadata fields (use `field.metadata_kind()` to identify them)
    /// 2. Emitting metadata in the appropriate position
    /// 3. Serializing the non-metadata field value
    ///
    /// Returns `Ok(true)` if handled, `Ok(false)` to fall back to default transparent
    /// serialization (which just serializes the non-metadata field).
    fn serialize_metadata_container(
        &mut self,
        _container: &facet_reflect::PeekStruct<'_, '_>,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Preferred field ordering for this format.
    /// Default is declaration order.
    fn preferred_field_order(&self) -> FieldOrdering {
        FieldOrdering::Declaration
    }

    /// Preferred struct field mode for this format.
    fn struct_field_mode(&self) -> StructFieldMode {
        StructFieldMode::Named
    }

    /// Preferred map encoding for this format.
    fn map_encoding(&self) -> MapEncoding {
        MapEncoding::Struct
    }

    /// Preferred enum variant encoding for this format.
    fn enum_variant_encoding(&self) -> EnumVariantEncoding {
        EnumVariantEncoding::Tagged
    }

    /// Whether this format is self-describing (includes type information).
    ///
    /// Self-describing formats (JSON, YAML, TOML) can deserialize without hints
    /// and treat newtypes transparently. Non-self-describing formats (ASN.1,
    /// postcard, msgpack) require structural hints and wrap newtypes.
    ///
    /// Default is `true` for text-based formats.
    fn is_self_describing(&self) -> bool {
        true
    }

    /// Preferred dynamic value encoding for this format.
    fn dynamic_value_encoding(&self) -> DynamicValueEncoding {
        DynamicValueEncoding::SelfDescribing
    }

    /// Returns the shape of the format's raw capture type for serialization.
    ///
    /// When serializing a value whose shape matches this, the serializer will
    /// extract the inner string and call [`FormatSerializer::raw_scalar`] instead of normal
    /// serialization.
    fn raw_serialize_shape(&self) -> Option<&'static facet_core::Shape> {
        None
    }

    /// Emit a raw scalar value (for RawJson, etc.) without any encoding/escaping.
    ///
    /// The content is the format-specific raw representation that should be
    /// output directly.
    fn raw_scalar(&mut self, content: &str) -> Result<(), Self::Error> {
        // Default: treat as a regular string (formats should override this)
        self.scalar(ScalarValue::Str(Cow::Borrowed(content)))
    }

    /// Serialize an opaque scalar type with a format-specific encoding.
    ///
    /// Returns `Ok(true)` if handled, `Ok(false)` to fall back to standard logic.
    fn serialize_opaque_scalar(
        &mut self,
        _shape: &'static facet_core::Shape,
        _value: Peek<'_, '_>,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Serialize an opaque scalar with optional field context.
    ///
    /// Backends can use field attributes to customize behavior. The default
    /// implementation forwards to `serialize_opaque_scalar` for compatibility.
    fn serialize_opaque_scalar_with_field(
        &mut self,
        _field: Option<&facet_core::Field>,
        shape: &'static facet_core::Shape,
        value: Peek<'_, '_>,
    ) -> Result<bool, Self::Error> {
        self.serialize_opaque_scalar(shape, value)
    }

    /// Emit a dynamic value type tag.
    ///
    /// Formats that use [`DynamicValueEncoding::Tagged`] should override this.
    /// Self-describing formats can ignore it.
    fn dynamic_value_tag(&mut self, _tag: DynamicValueTag) -> Result<(), Self::Error> {
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Binary format support methods
    //
    // The following methods enable proper serialization for binary formats like
    // postcard that need length prefixes, type-precise encoding, and explicit
    // discriminants. All have default implementations for backward compatibility
    // with existing text-format serializers.
    // ─────────────────────────────────────────────────────────────────────────

    /// Begin a sequence with known length.
    ///
    /// Binary formats (postcard, msgpack) can use this to write a length prefix
    /// before the elements. Text formats can ignore the length and just call
    /// `begin_seq()`.
    ///
    /// Default: delegates to `begin_seq()`.
    fn begin_seq_with_len(&mut self, _len: usize) -> Result<(), Self::Error> {
        self.begin_seq()
    }

    /// Begin serializing a map with known length.
    ///
    /// Default: delegates to `begin_struct()` for formats that encode maps as objects.
    fn begin_map_with_len(&mut self, _len: usize) -> Result<(), Self::Error> {
        self.begin_struct()
    }

    /// End a map/object/struct.
    ///
    /// Default: delegates to `end_struct()`.
    fn end_map(&mut self) -> Result<(), Self::Error> {
        self.end_struct()
    }

    /// Serialize a map key in `MapEncoding::Struct` mode.
    ///
    /// This is called for each map key when using struct encoding. The default
    /// implementation converts the key to a string (via `as_str()` or `Display`)
    /// and calls `field_key()`.
    ///
    /// Formats can override this to handle special key types differently.
    /// For example, Styx overrides this to serialize `Option::None` as `@`.
    ///
    /// Returns `Ok(true)` if handled, `Ok(false)` to use the default behavior.
    fn serialize_map_key(&mut self, _key: Peek<'_, '_>) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Serialize a scalar with full type information.
    ///
    /// Binary formats need to encode different integer sizes differently:
    /// - postcard: u8 as raw byte, u16+ as varint, signed use zigzag
    /// - msgpack: different tags for different sizes
    ///
    /// Text formats can ignore the type and use the normalized `ScalarValue`.
    ///
    /// Default: normalizes to `ScalarValue` and calls `scalar()`.
    fn typed_scalar(
        &mut self,
        scalar_type: ScalarType,
        value: Peek<'_, '_>,
    ) -> Result<(), Self::Error> {
        // Default implementation: normalize to ScalarValue and call scalar()
        let scalar = match scalar_type {
            ScalarType::Unit => ScalarValue::Null,
            ScalarType::Bool => ScalarValue::Bool(*value.get::<bool>().unwrap()),
            ScalarType::Char => ScalarValue::Char(*value.get::<char>().unwrap()),
            ScalarType::Str | ScalarType::String | ScalarType::CowStr => {
                ScalarValue::Str(Cow::Borrowed(value.as_str().unwrap()))
            }
            ScalarType::F32 => ScalarValue::F64(*value.get::<f32>().unwrap() as f64),
            ScalarType::F64 => ScalarValue::F64(*value.get::<f64>().unwrap()),
            ScalarType::U8 => ScalarValue::U64(*value.get::<u8>().unwrap() as u64),
            ScalarType::U16 => ScalarValue::U64(*value.get::<u16>().unwrap() as u64),
            ScalarType::U32 => ScalarValue::U64(*value.get::<u32>().unwrap() as u64),
            ScalarType::U64 => ScalarValue::U64(*value.get::<u64>().unwrap()),
            ScalarType::U128 => {
                let n = *value.get::<u128>().unwrap();
                ScalarValue::Str(Cow::Owned(alloc::string::ToString::to_string(&n)))
            }
            ScalarType::USize => ScalarValue::U64(*value.get::<usize>().unwrap() as u64),
            ScalarType::I8 => ScalarValue::I64(*value.get::<i8>().unwrap() as i64),
            ScalarType::I16 => ScalarValue::I64(*value.get::<i16>().unwrap() as i64),
            ScalarType::I32 => ScalarValue::I64(*value.get::<i32>().unwrap() as i64),
            ScalarType::I64 => ScalarValue::I64(*value.get::<i64>().unwrap()),
            ScalarType::I128 => {
                let n = *value.get::<i128>().unwrap();
                ScalarValue::Str(Cow::Owned(alloc::string::ToString::to_string(&n)))
            }
            ScalarType::ISize => ScalarValue::I64(*value.get::<isize>().unwrap() as i64),
            #[cfg(feature = "net")]
            ScalarType::IpAddr => {
                let addr = *value.get::<core::net::IpAddr>().unwrap();
                ScalarValue::Str(Cow::Owned(alloc::string::ToString::to_string(&addr)))
            }
            #[cfg(feature = "net")]
            ScalarType::Ipv4Addr => {
                let addr = *value.get::<core::net::Ipv4Addr>().unwrap();
                ScalarValue::Str(Cow::Owned(alloc::string::ToString::to_string(&addr)))
            }
            #[cfg(feature = "net")]
            ScalarType::Ipv6Addr => {
                let addr = *value.get::<core::net::Ipv6Addr>().unwrap();
                ScalarValue::Str(Cow::Owned(alloc::string::ToString::to_string(&addr)))
            }
            #[cfg(feature = "net")]
            ScalarType::SocketAddr => {
                let addr = *value.get::<core::net::SocketAddr>().unwrap();
                ScalarValue::Str(Cow::Owned(alloc::string::ToString::to_string(&addr)))
            }
            _ => {
                // For unknown scalar types, try to get a string representation
                if let Some(s) = value.as_str() {
                    ScalarValue::Str(Cow::Borrowed(s))
                } else {
                    ScalarValue::Null
                }
            }
        };
        self.scalar(scalar)
    }

    /// Begin serializing `Option::Some(value)`.
    ///
    /// Binary formats like postcard write a `0x01` discriminant byte here.
    /// Text formats typically don't need a prefix (they just serialize the value).
    ///
    /// Default: no-op (text formats).
    fn begin_option_some(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Serialize `Option::None`.
    ///
    /// Binary formats like postcard write a `0x00` discriminant byte.
    /// Text formats typically emit `null`.
    ///
    /// Default: emits `ScalarValue::Null`.
    fn serialize_none(&mut self) -> Result<(), Self::Error> {
        self.scalar(ScalarValue::Null)
    }

    /// Begin an enum variant with its index and name.
    ///
    /// Binary formats like postcard write the variant index as a varint.
    /// Text formats typically use the variant name as a key or value.
    ///
    /// This is called for externally tagged enums before the variant payload.
    /// For untagged enums, this is not called.
    ///
    /// Default: no-op (text formats handle variants via field_key/scalar).
    fn begin_enum_variant(
        &mut self,
        _variant_index: usize,
        _variant_name: &'static str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Write a tag for an externally-tagged enum variant.
    ///
    /// Formats like Styx that use `@tag` syntax for enum variants should override
    /// this to write their tag and return `Ok(true)`. The shared serializer will
    /// then call the appropriate payload serialization method.
    ///
    /// If this returns `Ok(false)` (the default), the shared serializer uses
    /// the standard externally-tagged representation: `{ "variant_name": payload }`.
    ///
    /// When returning `Ok(true)`:
    /// - For unit variants, nothing more is written
    /// - For newtype variants, the payload is serialized directly after
    /// - For struct variants, begin_struct_after_tag is called for the payload
    fn write_variant_tag(&mut self, _variant_name: &str) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Begin a struct directly after a variant tag (no separator).
    ///
    /// Called after `write_variant_tag` returns `Ok(true)` for struct variants.
    /// Formats should write `{` without any preceding space/separator.
    ///
    /// Default: calls `begin_struct()`.
    fn begin_struct_after_tag(&mut self) -> Result<(), Self::Error> {
        self.begin_struct()
    }

    /// Begin a sequence directly after a variant tag (no separator).
    ///
    /// Called after `write_variant_tag` returns `Ok(true)` for tuple variants.
    /// Formats should write `(` or `[` without any preceding space/separator.
    ///
    /// Default: calls `begin_seq()`.
    fn begin_seq_after_tag(&mut self) -> Result<(), Self::Error> {
        self.begin_seq()
    }

    /// Called when a variant tag was written but no payload follows.
    ///
    /// This happens for:
    /// - Unit variants (no fields at all)
    /// - `#[facet(other)]` variants where all fields are metadata/tag fields
    ///
    /// Formats that track state after `write_variant_tag` (like Styx's
    /// `skip_next_before_value` flag) should use this to clear that state,
    /// ensuring the next value gets proper spacing.
    ///
    /// Default: no-op.
    fn finish_variant_tag_unit_payload(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Serialize a byte sequence (`Vec<u8>`, `&[u8]`, etc.) in bulk.
    ///
    /// For binary formats like postcard that store byte sequences as raw bytes
    /// (varint length followed by raw data), this allows bulk writing instead
    /// of element-by-element serialization.
    ///
    /// If the serializer handles this, it should write the bytes directly and
    /// return `Ok(true)`. If it doesn't support this optimization, it should
    /// return `Ok(false)` and the serializer will fall back to element-by-element
    /// serialization.
    ///
    /// Returns `Ok(true)` if handled (bytes were written), `Ok(false)` otherwise.
    fn serialize_byte_sequence(&mut self, _bytes: &[u8]) -> Result<bool, Self::Error> {
        // Default: not supported, fall back to element-by-element
        Ok(false)
    }

    /// Serialize a fixed-size byte array (`[u8; N]`) in bulk.
    ///
    /// Unlike `serialize_byte_sequence`, this does NOT write a length prefix
    /// since the array size is known from the type.
    ///
    /// Returns `Ok(true)` if handled (bytes were written), `Ok(false)` otherwise.
    fn serialize_byte_array(&mut self, _bytes: &[u8]) -> Result<bool, Self::Error> {
        // Default: not supported, fall back to element-by-element
        Ok(false)
    }

    /// Returns the format namespace for format-specific proxy resolution.
    ///
    /// When a field or container has format-specific proxies (e.g., `#[facet(xml::proxy = XmlProxy)]`),
    /// this namespace is used to look up the appropriate proxy. If no namespace is returned,
    /// only the format-agnostic proxy (`#[facet(proxy = ...)]`) is considered.
    ///
    /// Examples:
    /// - XML serializer should return `Some("xml")`
    /// - JSON serializer should return `Some("json")`
    ///
    /// Default: returns `None` (only format-agnostic proxies are used).
    fn format_namespace(&self) -> Option<&'static str> {
        None
    }
}

/// Error produced by the shared serializer.
#[derive(Debug)]
#[non_exhaustive]
pub enum SerializeError<E: Debug> {
    /// Cooperative cancellation was requested before serialization completed.
    Cancelled,
    /// Format backend error.
    Backend(E),
    /// Reflection failed while traversing the value.
    Reflect(ReflectError),
    /// Value can't be represented by the shared serializer.
    Unsupported(Cow<'static, str>),
    /// Internal invariant violation.
    Internal(Cow<'static, str>),
}

impl<E: Debug> core::fmt::Display for SerializeError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SerializeError::Cancelled => f.write_str("serialization cancelled"),
            SerializeError::Backend(_) => f.write_str("format serializer error"),
            SerializeError::Reflect(err) => write!(f, "{err}"),
            SerializeError::Unsupported(msg) => f.write_str(msg.as_ref()),
            SerializeError::Internal(msg) => f.write_str(msg.as_ref()),
        }
    }
}

/// A path segment in the serialization context.
#[derive(Debug, Clone)]
pub enum PathSegment {
    /// A field name (struct field or map key).
    Field(Cow<'static, str>),
    /// An array/list index.
    Index(usize),
    /// An enum variant name.
    Variant(Cow<'static, str>),
}

impl core::fmt::Display for PathSegment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PathSegment::Field(name) => write!(f, ".{}", name),
            PathSegment::Index(idx) => write!(f, "[{}]", idx),
            PathSegment::Variant(name) => write!(f, "::{}", name),
        }
    }
}

/// Context for serialization, tracking the path through the value tree.
///
/// This context is passed through recursive serialization calls to track
/// where we are in the value hierarchy, enabling better error messages.
pub struct SerializeContext<'s, S: FormatSerializer> {
    serializer: &'s mut S,
    path: alloc::vec::Vec<PathSegment>,
    current_field: Option<facet_core::Field>,
    should_cancel: Option<&'s dyn Fn() -> bool>,
}

impl<'s, S: FormatSerializer> SerializeContext<'s, S> {
    /// Create a new serialization context wrapping a format serializer.
    pub fn new(serializer: &'s mut S) -> Self {
        Self {
            serializer,
            path: alloc::vec::Vec::new(),
            current_field: None,
            should_cancel: None,
        }
    }

    /// Create a context that checks a borrowed cancellation predicate while traversing.
    ///
    /// `true` requests cancellation. The callback may borrow caller-owned state and
    /// need not be `Send`, `Sync`, or `'static`. Checks occur between traversal work
    /// and backend calls; a backend, user formatter, or proxy must provide its own
    /// checkpoints to interrupt a long-running call internally. An interrupted
    /// serializer may contain a partial document and should be discarded.
    pub fn with_cancellation(serializer: &'s mut S, should_cancel: &'s dyn Fn() -> bool) -> Self {
        Self {
            serializer,
            path: alloc::vec::Vec::new(),
            current_field: None,
            should_cancel: Some(should_cancel),
        }
    }

    fn check_cancellation(&self) -> Result<(), SerializeError<S::Error>> {
        check_cancellation(self.should_cancel)
    }

    fn collect_fields<I>(
        &self,
        fields: I,
    ) -> Result<alloc::vec::Vec<I::Item>, SerializeError<S::Error>>
    where
        I: Iterator,
    {
        let mut collected = alloc::vec::Vec::new();
        for field in fields {
            self.check_cancellation()?;
            collected.push(field);
        }
        self.check_cancellation()?;
        Ok(collected)
    }

    fn with_field_context<T>(
        &mut self,
        field: Option<facet_core::Field>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let prev = self.current_field;
        self.current_field = field;
        let out = f(self);
        self.current_field = prev;
        out
    }

    /// Push a path segment onto the context.
    fn push(&mut self, segment: PathSegment) {
        self.path.push(segment);
    }

    /// Pop a path segment from the context.
    fn pop(&mut self) {
        self.path.pop();
    }

    /// Get the current path as a string.
    fn path_string(&self) -> String {
        if self.path.is_empty() {
            "<root>".into()
        } else {
            let mut s = String::new();
            for seg in &self.path {
                let _ = write!(s, "{}", seg);
            }
            s
        }
    }

    /// Create an unsupported error with path context.
    fn unsupported_error(&self, shape: &Shape, msg: &str) -> SerializeError<S::Error> {
        SerializeError::Unsupported(Cow::Owned(alloc::format!(
            "{} (type: `{}`, def: {}, path: `{}`)",
            msg,
            shape,
            def_kind_name(&shape.def),
            self.path_string()
        )))
    }

    /// Serialize a value, tracking path for error context.
    pub fn serialize<'mem, 'facet>(
        &mut self,
        value: Peek<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        self.serialize_impl(value)
    }

    fn serialize_impl<'mem, 'facet>(
        &mut self,
        value: Peek<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        self.check_cancellation()?;
        self.serialize_impl_inner(value)?;
        self.check_cancellation()
    }

    fn serialize_impl_inner<'mem, 'facet>(
        &mut self,
        value: Peek<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        // Dereference pointers (Box, Arc, etc.) to get the underlying value
        let value = self.deref_if_pointer(value)?;

        // Check for raw serialization type (e.g., RawJson) BEFORE innermost_peek
        if self.serializer.raw_serialize_shape() == Some(value.shape()) {
            if let Ok(struct_) = value.into_struct()
                && let Some((_field_item, inner_value)) =
                    struct_.fields_for_binary_serialize().next()
                && let Some(s) = inner_value.as_str()
            {
                self.check_cancellation()?;
                self.serializer
                    .raw_scalar(s)
                    .map_err(SerializeError::Backend)?;
                return self.check_cancellation();
            }
            return Err(SerializeError::Unsupported(Cow::Borrowed(
                "raw capture type matched but could not extract inner string",
            )));
        }

        self.check_cancellation()?;
        let handled = self
            .serializer
            .serialize_opaque_scalar_with_field(self.current_field.as_ref(), value.shape(), value)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        if handled {
            return Ok(());
        }

        let value = value.innermost_peek();

        // Check for metadata containers
        if value.shape().is_metadata_container()
            && let Ok(struct_) = value.into_struct()
        {
            self.check_cancellation()?;
            let handled = self
                .serializer
                .serialize_metadata_container(&struct_)
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
            if handled {
                return Ok(());
            }
            for (field, field_value) in struct_.fields() {
                self.check_cancellation()?;
                if !field.is_metadata() {
                    return self.serialize_impl(field_value);
                }
            }
        }

        // Check for container-level proxy
        if let Some(proxy_def) = value
            .shape()
            .effective_proxy(self.serializer.format_namespace())
        {
            return self.serialize_via_proxy(value, proxy_def);
        }

        // Use typed_scalar for scalars
        if let Some(scalar_type) = value.scalar_type() {
            self.check_cancellation()?;
            self.serializer
                .typed_scalar(scalar_type, value)
                .map_err(SerializeError::Backend)?;
            return self.check_cancellation();
        }

        // Fallback for Def::Scalar types with Display trait
        if matches!(value.shape().def, Def::Scalar) && value.shape().vtable.has_display() {
            use alloc::string::ToString;
            let formatted = value.to_string();
            self.check_cancellation()?;
            self.serializer
                .scalar(ScalarValue::Str(Cow::Owned(formatted)))
                .map_err(SerializeError::Backend)?;
            return self.check_cancellation();
        }

        if let Ok(opt) = value.into_option() {
            return self.serialize_option(opt);
        }

        if let Ok(result) = value.into_result() {
            return self.serialize_result(result);
        }

        if let Ok(dynamic) = value.into_dynamic_value() {
            return self.serialize_dynamic_value(dynamic);
        }

        match value.shape().def {
            facet_core::Def::List(_) | facet_core::Def::Array(_) | facet_core::Def::Slice(_) => {
                return self.serialize_list_like(value);
            }
            _ => {}
        }

        if let Ok(map) = value.into_map() {
            return self.serialize_map(map);
        }

        if let Ok(set) = value.into_set() {
            return self.serialize_set(set);
        }

        if let Ok(struct_) = value.into_struct() {
            return self.serialize_struct(value.shape(), struct_);
        }

        if let Ok(enum_) = value.into_enum() {
            return self.serialize_enum(value.shape(), enum_);
        }

        Err(self.unsupported_error(value.shape(), "unsupported value kind for serialization"))
    }

    #[inline(never)]
    fn serialize_option<'mem, 'facet>(
        &mut self,
        opt: PeekOption<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        match opt.value() {
            Some(inner) => {
                self.check_cancellation()?;
                self.serializer
                    .begin_option_some()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                self.serialize_impl(inner)
            }
            None => {
                self.check_cancellation()?;
                self.serializer
                    .serialize_none()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
        }
    }

    #[inline(never)]
    fn serialize_result<'mem, 'facet>(
        &mut self,
        result: PeekResult<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let (variant_index, variant_name, inner) = if result.is_ok() {
            (
                0,
                "Ok",
                result.ok().ok_or(SerializeError::Internal(Cow::Borrowed(
                    "result reported Ok but value was missing",
                )))?,
            )
        } else {
            (
                1,
                "Err",
                result.err().ok_or(SerializeError::Internal(Cow::Borrowed(
                    "result reported Err but value was missing",
                )))?,
            )
        };

        if self.serializer.enum_variant_encoding() == EnumVariantEncoding::Index {
            self.check_cancellation()?;
            self.serializer
                .begin_enum_variant(variant_index, variant_name)
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
            self.push(PathSegment::Variant(Cow::Borrowed(variant_name)));
            let result = self.serialize_impl(inner);
            self.pop();
            return result;
        }

        self.check_cancellation()?;
        self.serializer
            .begin_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        self.serializer
            .field_key(variant_name)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        self.push(PathSegment::Variant(Cow::Borrowed(variant_name)));
        let result = self.serialize_impl(inner);
        self.pop();
        result?;
        self.check_cancellation()?;
        self.serializer
            .end_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        Ok(())
    }

    #[inline(never)]
    fn serialize_list_like<'mem, 'facet>(
        &mut self,
        value: Peek<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let list = value.into_list_like().map_err(SerializeError::Reflect)?;
        self.serialize_list_like_inner(value.shape().def, list)
    }

    #[inline(never)]
    fn serialize_list_like_inner<'mem, 'facet>(
        &mut self,
        def: Def,
        list: PeekListLike<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let len = list.len();

        if let Some(bytes) = list.as_bytes() {
            self.check_cancellation()?;
            let handled = match def {
                facet_core::Def::Array(_) => self.serializer.serialize_byte_array(bytes),
                _ => self.serializer.serialize_byte_sequence(bytes),
            }
            .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
            if handled {
                return Ok(());
            }
        }

        self.check_cancellation()?;
        match def {
            facet_core::Def::Array(_) => self.serializer.begin_seq(),
            _ => self.serializer.begin_seq_with_len(len),
        }
        .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        for (idx, item) in list.iter().enumerate() {
            self.check_cancellation()?;
            self.push(PathSegment::Index(idx));
            self.serialize_impl(item)?;
            self.pop();
        }
        self.check_cancellation()?;
        self.serializer.end_seq().map_err(SerializeError::Backend)?;
        self.check_cancellation()
    }

    #[inline(never)]
    fn serialize_map<'mem, 'facet>(
        &mut self,
        map: PeekMap<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let len = map.len();
        match self.serializer.map_encoding() {
            MapEncoding::Pairs => self.serialize_map_as_pairs(map, len),
            MapEncoding::Struct => self.serialize_map_as_struct(map),
        }
    }

    fn map_key_string<'mem, 'facet>(
        &self,
        key: Peek<'mem, 'facet>,
    ) -> Result<Cow<'mem, str>, SerializeError<S::Error>> {
        self.check_cancellation()?;
        if let Some(proxy_def) = key
            .shape()
            .effective_proxy(self.serializer.format_namespace())
        {
            let proxy = key
                .custom_serialization_with_proxy(proxy_def)
                .map_err(|err| SerializeError::Unsupported(Cow::Owned(alloc::format!("{err}"))))?;
            let proxy_peek = proxy.as_peek().innermost_peek();
            let key = self
                .extract_string_from_peek(proxy_peek)?
                .map(ToString::to_string)
                .unwrap_or_else(|| alloc::format!("{}", proxy_peek));
            self.check_cancellation()?;
            return Ok(Cow::Owned(key));
        }

        let key = self
            .extract_string_from_peek(key)?
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(alloc::format!("{}", key)));
        self.check_cancellation()?;
        Ok(key)
    }

    #[inline(never)]
    fn serialize_map_as_pairs<'mem, 'facet>(
        &mut self,
        map: PeekMap<'mem, 'facet>,
        len: usize,
    ) -> Result<(), SerializeError<S::Error>> {
        self.check_cancellation()?;
        self.serializer
            .begin_map_with_len(len)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        for (key, val) in map.iter() {
            self.check_cancellation()?;
            self.serialize_impl(key)?;
            let key_str = self.map_key_string(key)?.into_owned();
            self.push(PathSegment::Field(Cow::Owned(key_str)));
            self.serialize_impl(val)?;
            self.pop();
        }
        self.check_cancellation()?;
        self.serializer.end_map().map_err(SerializeError::Backend)?;
        self.check_cancellation()
    }

    #[inline(never)]
    fn serialize_map_as_struct<'mem, 'facet>(
        &mut self,
        map: PeekMap<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        self.check_cancellation()?;
        self.serializer
            .begin_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        for (key, val) in map.iter() {
            self.check_cancellation()?;
            let key_str = self.map_key_string(key)?;
            self.check_cancellation()?;
            let key_written = self
                .serializer
                .serialize_map_key(key)
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
            if !key_written {
                self.serializer
                    .field_key(&key_str)
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
            }
            self.push(PathSegment::Field(Cow::Owned(key_str.into_owned())));
            self.serialize_impl(val)?;
            self.pop();
        }
        self.check_cancellation()?;
        self.serializer
            .end_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()
    }

    #[inline(never)]
    fn serialize_set<'mem, 'facet>(
        &mut self,
        set: PeekSet<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let len = set.len();
        self.check_cancellation()?;
        self.serializer
            .begin_seq_with_len(len)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        for (idx, item) in set.iter().enumerate() {
            self.check_cancellation()?;
            self.push(PathSegment::Index(idx));
            self.serialize_impl(item)?;
            self.pop();
        }
        self.check_cancellation()?;
        self.serializer.end_seq().map_err(SerializeError::Backend)?;
        self.check_cancellation()
    }

    fn serialize_field_value<'mem, 'facet>(
        &mut self,
        field_item: &facet_reflect::FieldItem,
        field_value: Peek<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        self.with_field_context(field_item.field, |this| {
            if let Some(proxy_def) = field_item
                .field
                .and_then(|f| f.effective_proxy(this.serializer.format_namespace()))
            {
                this.serialize_via_proxy(field_value, proxy_def)
            } else {
                this.serialize_impl(field_value)
            }
        })
    }

    fn serialize_struct<'mem, 'facet>(
        &mut self,
        shape: &'static Shape,
        struct_: facet_reflect::PeekStruct<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let kind = struct_.ty().kind;
        self.check_cancellation()?;
        self.serializer
            .struct_metadata(shape)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;

        if kind == StructKind::Tuple || kind == StructKind::TupleStruct {
            self.serialize_tuple_struct(shape, struct_)
        } else {
            self.serialize_named_struct(struct_)
        }
    }

    #[inline(never)]
    fn serialize_tuple_struct<'mem, 'facet>(
        &mut self,
        shape: &'static Shape,
        struct_: facet_reflect::PeekStruct<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let fields: alloc::vec::Vec<_> =
            self.collect_fields(struct_.fields_for_binary_serialize())?;
        let is_transparent = shape.is_transparent() && fields.len() == 1;

        if is_transparent {
            let (field_item, field_value) = &fields[0];
            self.serialize_field_value(field_item, *field_value)
        } else {
            self.check_cancellation()?;
            self.serializer
                .begin_seq()
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
            for (idx, (field_item, field_value)) in fields.into_iter().enumerate() {
                self.check_cancellation()?;
                self.push(PathSegment::Index(idx));
                self.serialize_field_value(&field_item, field_value)?;
                self.pop();
            }
            self.check_cancellation()?;
            self.serializer.end_seq().map_err(SerializeError::Backend)?;
            self.check_cancellation()
        }
    }

    #[inline(never)]
    fn serialize_named_struct<'mem, 'facet>(
        &mut self,
        struct_: facet_reflect::PeekStruct<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let field_mode = self.serializer.struct_field_mode();
        self.check_cancellation()?;
        self.serializer
            .begin_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;

        let mut fields: alloc::vec::Vec<_> = if field_mode == StructFieldMode::Unnamed {
            self.collect_fields(struct_.fields_for_binary_serialize())?
        } else {
            self.collect_fields(struct_.fields_for_serialize())?
        };

        sort_fields_if_needed(self.serializer, &mut fields);

        for (field_item, field_value) in fields {
            self.check_cancellation()?;
            if self.serializer.should_omit_field(&field_item, field_value) {
                continue;
            }
            if field_item.flattened {
                self.serialize_flattened_struct_field(field_mode, field_item, field_value)?;
            } else {
                self.serialize_regular_struct_field(field_mode, field_item, field_value)?;
            }
        }

        self.check_cancellation()?;
        self.serializer
            .end_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()
    }

    #[inline(never)]
    fn serialize_regular_struct_field<'mem, 'facet>(
        &mut self,
        field_mode: StructFieldMode,
        field_item: facet_reflect::FieldItem,
        field_value: Peek<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        self.check_cancellation()?;
        let key_written = self
            .serializer
            .field_metadata_with_value(&field_item, field_value)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        if !key_written {
            self.serializer
                .field_metadata(&field_item)
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
            if field_mode == StructFieldMode::Named {
                self.serializer
                    .field_key(field_item.effective_name())
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
            }
        }
        self.push(PathSegment::Field(Cow::Owned(
            field_item.effective_name().to_string(),
        )));
        let result = self.serialize_field_value(&field_item, field_value);
        self.pop();
        result
    }

    #[inline(never)]
    fn serialize_flattened_struct_field<'mem, 'facet>(
        &mut self,
        field_mode: StructFieldMode,
        field_item: facet_reflect::FieldItem,
        field_value: Peek<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        if let Some(field) = field_item.field
            && let shape = field.shape()
            && let Some(tag_key) = shape.get_tag_attr()
            && shape.get_content_attr().is_none()
        {
            let variant_name = field_item.effective_name();

            if field_mode == StructFieldMode::Named {
                self.check_cancellation()?;
                self.serializer
                    .field_key(tag_key)
                    .map_err(SerializeError::Backend)?;
            }
            self.check_cancellation()?;
            self.serializer
                .scalar(ScalarValue::Str(Cow::Borrowed(variant_name)))
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;

            if let Ok(inner_struct) = field_value.into_struct() {
                for (inner_item, inner_value) in inner_struct.fields_for_serialize() {
                    self.check_cancellation()?;
                    if field_mode == StructFieldMode::Named {
                        self.serializer
                            .field_key(inner_item.effective_name())
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                    }
                    self.push(PathSegment::Field(Cow::Owned(
                        inner_item.effective_name().to_string(),
                    )));
                    self.serialize_field_value(&inner_item, inner_value)?;
                    self.pop();
                }
            } else if let Ok(enum_peek) = field_value.into_enum() {
                for (inner_item, inner_value) in enum_peek.fields_for_serialize() {
                    self.check_cancellation()?;
                    if field_mode == StructFieldMode::Named {
                        self.serializer
                            .field_key(inner_item.effective_name())
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                    }
                    self.push(PathSegment::Field(Cow::Owned(
                        inner_item.effective_name().to_string(),
                    )));
                    self.serialize_field_value(&inner_item, inner_value)?;
                    self.pop();
                }
            } else if matches!(field_value.shape().ty, Type::Primitive(_)) {
                return Err(SerializeError::Unsupported(
                    "internally-tagged enum with scalar newtype payload cannot be \
                     flattened; use #[facet(content = \"...\")] for adjacently-tagged \
                     representation"
                        .into(),
                ));
            }
            return Ok(());
        }

        if {
            let shape = field_value.shape();
            shape.get_tag_attr().is_none() && shape.get_content_attr().is_none()
        } && let Ok(enum_peek) = field_value.into_enum()
        {
            if field_mode == StructFieldMode::Named {
                self.check_cancellation()?;
                self.serializer
                    .field_key(field_item.effective_name())
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
            }

            let variant = enum_peek
                .active_variant()
                .map_err(|e| SerializeError::Unsupported(Cow::Owned(e.to_string())))?;

            self.push(PathSegment::Variant(Cow::Owned(
                field_item.effective_name().to_string(),
            )));
            if variant.data.kind == StructKind::Unit {
                self.check_cancellation()?;
                self.serializer
                    .serialize_none()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
            } else {
                self.serialize_variant_after_tag(enum_peek, variant)?;
            }
            self.pop();
            return Ok(());
        }

        self.serialize_regular_struct_field(field_mode, field_item, field_value)
    }

    /// Recursively flattens a newtype variant's inner value into the current
    /// JSON object that already has `begin_struct` and the outer tag written.
    ///
    /// Three cases:
    /// 1. Inner value is a **struct** → emit its fields directly.
    /// 2. Inner value is an **internally-tagged enum** → emit its tag, then
    ///    handle its active variant (which may itself be a newtype, so recurse).
    /// 3. Inner value is a **scalar / unsupported type** → error, because
    ///    scalars cannot be flattened into an object.
    fn serialize_flattened_newtype_value<'mem, 'facet>(
        &mut self,
        value: Peek<'mem, 'facet>,
        used_tag_keys: &[&str],
    ) -> Result<(), SerializeError<S::Error>> {
        let shape = value.shape();
        let field_mode = self.serializer.struct_field_mode();

        // Case 1: plain struct — flatten its fields into the enclosing object
        if let Ok(struct_) = value.into_struct() {
            let mut fields: alloc::vec::Vec<_> = if field_mode == StructFieldMode::Unnamed {
                self.collect_fields(struct_.fields_for_binary_serialize())?
            } else {
                self.collect_fields(struct_.fields_for_serialize())?
            };
            sort_fields_if_needed(self.serializer, &mut fields);
            for (field_item, field_value) in fields {
                self.check_cancellation()?;
                self.serializer
                    .field_metadata(&field_item)
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                if field_mode == StructFieldMode::Named {
                    self.serializer
                        .field_key(field_item.effective_name())
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                }
                self.push(PathSegment::Field(Cow::Owned(
                    field_item.effective_name().to_string(),
                )));
                self.serialize_field_value(&field_item, field_value)?;
                self.pop();
            }
            return Ok(());
        }

        // Case 2: internally-tagged enum — write its tag, then handle its variant
        if let Some(inner_tag) = shape.get_tag_attr()
            && shape.get_content_attr().is_none()
            && let Ok(inner_enum) = value.into_enum()
        {
            // Reject duplicate tag key names across nesting levels —
            // e.g. both outer and inner enum using #[facet(tag = "type")].
            // With a flat JSON object we cannot distinguish which "type"
            // value belongs to which level.
            if used_tag_keys.contains(&inner_tag) {
                return Err(SerializeError::Unsupported(
                    format!(
                        "nested internally-tagged enums use the same tag key \"{}\"; \
                         this is ambiguous when flattened into a single object",
                        inner_tag
                    )
                    .into(),
                ));
            }

            let inner_variant = inner_enum
                .active_variant()
                .map_err(|e| SerializeError::Unsupported(Cow::Owned(e.to_string())))?;

            // Write the inner enum's tag
            self.check_cancellation()?;
            self.serializer
                .field_key(inner_tag)
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
            self.serializer
                .scalar(ScalarValue::Str(Cow::Borrowed(
                    inner_variant.effective_name(),
                )))
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;

            self.push(PathSegment::Variant(Cow::Borrowed(
                inner_variant.effective_name(),
            )));

            match inner_variant.data.kind {
                StructKind::Unit => {}
                StructKind::Struct => {
                    let mut inner_fields: alloc::vec::Vec<_> =
                        if field_mode == StructFieldMode::Unnamed {
                            self.collect_fields(inner_enum.fields_for_binary_serialize())?
                        } else {
                            self.collect_fields(inner_enum.fields_for_serialize())?
                        };
                    sort_fields_if_needed(self.serializer, &mut inner_fields);

                    for (field_item, field_value) in inner_fields {
                        self.check_cancellation()?;
                        self.serializer
                            .field_metadata(&field_item)
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                        if field_mode == StructFieldMode::Named {
                            self.serializer
                                .field_key(field_item.effective_name())
                                .map_err(SerializeError::Backend)?;
                            self.check_cancellation()?;
                        }
                        self.push(PathSegment::Field(Cow::Owned(
                            field_item.effective_name().to_string(),
                        )));
                        self.serialize_field_value(&field_item, field_value)?;
                        self.pop();
                    }
                }
                StructKind::TupleStruct | StructKind::Tuple => {
                    // Inner variant is itself a newtype — recurse
                    if inner_variant.data.fields.len() != 1 {
                        self.pop();
                        return Err(SerializeError::Unsupported(Cow::Borrowed(
                            "internally tagged tuple variants with multiple fields are not supported",
                        )));
                    }
                    let nested_value = inner_enum
                        .field(0)
                        .map_err(|e| SerializeError::Unsupported(Cow::Owned(e.to_string())))?
                        .expect("single-field tuple variant should have field 0");
                    let mut inner_used_tag_keys = alloc::vec::Vec::from(used_tag_keys);
                    inner_used_tag_keys.push(inner_tag);
                    self.serialize_flattened_newtype_value(nested_value, &inner_used_tag_keys)?;
                }
            }

            self.pop();
            return Ok(());
        }

        // Case 3: scalar or other non-flattenable type
        Err(SerializeError::Unsupported(
            "internally-tagged enum with scalar newtype payload cannot be \
             flattened; use #[facet(content = \"...\")] for adjacently-tagged \
             representation"
                .into(),
        ))
    }

    fn serialize_discriminant<'mem, 'facet>(
        &mut self,
        enum_: facet_reflect::PeekEnum<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        match enum_.ty().enum_repr {
            facet_core::EnumRepr::Rust => Err(SerializeError::Internal(Cow::Borrowed(
                "enum does not have an explicit representation",
            ))),
            facet_core::EnumRepr::RustNPO
            | facet_core::EnumRepr::U8
            | facet_core::EnumRepr::U16
            | facet_core::EnumRepr::U32
            | facet_core::EnumRepr::U64
            | facet_core::EnumRepr::USize => {
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::U64(enum_.discriminant() as u64))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            facet_core::EnumRepr::I8
            | facet_core::EnumRepr::I16
            | facet_core::EnumRepr::I32
            | facet_core::EnumRepr::I64
            | facet_core::EnumRepr::ISize => {
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::I64(enum_.discriminant()))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
        }
    }

    fn serialize_enum<'mem, 'facet>(
        &mut self,
        shape: &'static Shape,
        enum_: facet_reflect::PeekEnum<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let variant = enum_.active_variant().map_err(|_| {
            SerializeError::Unsupported(Cow::Borrowed("opaque enum layout is unsupported"))
        })?;

        self.check_cancellation()?;
        self.serializer
            .variant_metadata(variant)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;

        // Cow-like enums serialize transparently
        if shape.is_cow() {
            let inner = enum_
                .field(0)
                .map_err(|_| {
                    SerializeError::Internal(Cow::Borrowed("cow variant field lookup failed"))
                })?
                .ok_or(SerializeError::Internal(Cow::Borrowed(
                    "cow variant has no field",
                )))?;
            return self.serialize_impl(inner);
        }

        if self.serializer.enum_variant_encoding() == EnumVariantEncoding::Index {
            let variant_index = enum_.variant_index().map_err(|_| {
                SerializeError::Unsupported(Cow::Borrowed("opaque enum layout is unsupported"))
            })?;
            self.check_cancellation()?;
            self.serializer
                .begin_enum_variant(variant_index, variant.name)
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;

            self.push(PathSegment::Variant(Cow::Borrowed(variant.name)));
            let result = match variant.data.kind {
                StructKind::Unit => Ok(()),
                StructKind::TupleStruct | StructKind::Tuple | StructKind::Struct => {
                    for (idx, (field_item, field_value)) in
                        enum_.fields_for_binary_serialize().enumerate()
                    {
                        self.check_cancellation()?;
                        self.push(PathSegment::Index(idx));
                        self.serialize_field_value(&field_item, field_value)?;
                        self.pop();
                    }
                    Ok(())
                }
            };
            self.pop();
            return result;
        }

        let numeric = shape.is_numeric();
        let untagged = shape.is_untagged();
        let tag = shape.get_tag_attr();
        let content = shape.get_content_attr();

        if numeric && tag.is_none() {
            return serialize_numeric_enum(self.serializer, variant);
        }
        if untagged || variant.has_builtin_attr("untagged") {
            self.push(PathSegment::Variant(Cow::Borrowed(
                variant.effective_name(),
            )));
            let result = self.serialize_untagged_enum(enum_, variant);
            self.pop();
            return result;
        }

        // #[facet(other)] variants serialize as untagged UNLESS they have a #[facet(tag)] field.
        // When a tag field is present, the captured tag value should be serialized via
        // serialize_externally_tagged_enum, which knows how to extract and use that value.
        if variant.is_other() {
            let mut has_tag_field = false;
            for field in variant.data.fields {
                self.check_cancellation()?;
                if field.is_variant_tag() {
                    has_tag_field = true;
                    break;
                }
            }
            if !has_tag_field {
                self.push(PathSegment::Variant(Cow::Borrowed(
                    variant.effective_name(),
                )));
                let result = self.serialize_untagged_enum(enum_, variant);
                self.pop();
                return result;
            }
        }

        match (tag, content) {
            (Some(tag_key), None) => {
                // Internally tagged
                self.check_cancellation()?;
                self.serializer
                    .begin_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                self.serializer
                    .field_key(tag_key)
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;

                if numeric {
                    self.serialize_discriminant(enum_)?;
                } else {
                    self.check_cancellation()?;
                    self.serializer
                        .scalar(ScalarValue::Str(Cow::Borrowed(variant.effective_name())))
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                }

                self.push(PathSegment::Variant(Cow::Borrowed(
                    variant.effective_name(),
                )));
                let field_mode = self.serializer.struct_field_mode();
                match variant.data.kind {
                    StructKind::Unit => {}
                    StructKind::Struct => {
                        let mut fields: alloc::vec::Vec<_> =
                            if field_mode == StructFieldMode::Unnamed {
                                self.collect_fields(enum_.fields_for_binary_serialize())?
                            } else {
                                self.collect_fields(enum_.fields_for_serialize())?
                            };
                        sort_fields_if_needed(self.serializer, &mut fields);
                        for (field_item, field_value) in fields {
                            self.check_cancellation()?;
                            self.serializer
                                .field_metadata(&field_item)
                                .map_err(SerializeError::Backend)?;
                            self.check_cancellation()?;
                            if field_mode == StructFieldMode::Named {
                                self.serializer
                                    .field_key(field_item.effective_name())
                                    .map_err(SerializeError::Backend)?;
                                self.check_cancellation()?;
                            }
                            self.push(PathSegment::Field(Cow::Owned(
                                field_item.effective_name().to_string(),
                            )));
                            self.serialize_field_value(&field_item, field_value)?;
                            self.pop();
                        }
                    }
                    StructKind::TupleStruct | StructKind::Tuple => {
                        // Single-field tuple (newtype) variants get flattened into the
                        // enclosing tagged object. The inner value may be a struct, an
                        // internally-tagged enum, or a chain of newtype wrappers around
                        // one of those — we handle all cases recursively.
                        if variant.data.fields.len() != 1 {
                            self.pop();
                            return Err(SerializeError::Unsupported(Cow::Borrowed(
                                "internally tagged tuple variants with multiple fields are not supported",
                            )));
                        }

                        let inner_value = enum_
                            .field(0)
                            .map_err(|e| SerializeError::Unsupported(Cow::Owned(e.to_string())))?
                            .expect("single-field tuple variant should have field 0");

                        self.serialize_flattened_newtype_value(inner_value, &[tag_key])?;
                    }
                }
                self.pop();

                self.check_cancellation()?;
                self.serializer
                    .end_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                return Ok(());
            }
            (Some(tag_key), Some(content_key)) => {
                // Adjacently tagged
                return self.serialize_adjacently_tagged_enum(
                    enum_,
                    variant,
                    tag_key,
                    content_key,
                    numeric,
                );
            }
            (None, Some(_)) => {
                return Err(SerializeError::Unsupported(Cow::Borrowed(
                    "adjacent content key set without tag key",
                )));
            }
            (None, None) => {}
        }

        // Externally tagged (default)
        self.serialize_externally_tagged_enum(enum_, variant)
    }

    fn serialize_adjacently_tagged_enum<'mem, 'facet>(
        &mut self,
        enum_: facet_reflect::PeekEnum<'mem, 'facet>,
        variant: &'static facet_core::Variant,
        tag_key: &'static str,
        content_key: &'static str,
        numeric: bool,
    ) -> Result<(), SerializeError<S::Error>> {
        let field_mode = self.serializer.struct_field_mode();
        self.check_cancellation()?;
        self.serializer
            .begin_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        self.serializer
            .field_key(tag_key)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;

        if numeric {
            self.serialize_discriminant(enum_)?;
        } else {
            self.check_cancellation()?;
            self.serializer
                .scalar(ScalarValue::Str(Cow::Borrowed(variant.effective_name())))
                .map_err(SerializeError::Backend)?;
            self.check_cancellation()?;
        }

        self.push(PathSegment::Variant(Cow::Borrowed(
            variant.effective_name(),
        )));

        match variant.data.kind {
            StructKind::Unit => {}
            StructKind::Struct => {
                self.check_cancellation()?;
                self.serializer
                    .field_key(content_key)
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                self.serializer
                    .begin_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                let mut fields: alloc::vec::Vec<_> = if field_mode == StructFieldMode::Unnamed {
                    self.collect_fields(enum_.fields_for_binary_serialize())?
                } else {
                    self.collect_fields(enum_.fields_for_serialize())?
                };
                sort_fields_if_needed(self.serializer, &mut fields);
                for (field_item, field_value) in fields {
                    self.check_cancellation()?;
                    self.serializer
                        .field_metadata(&field_item)
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    if field_mode == StructFieldMode::Named {
                        self.serializer
                            .field_key(field_item.effective_name())
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                    }
                    self.push(PathSegment::Field(Cow::Owned(
                        field_item.effective_name().to_string(),
                    )));
                    self.serialize_field_value(&field_item, field_value)?;
                    self.pop();
                }
                self.check_cancellation()?;
                self.serializer
                    .end_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
            }
            StructKind::TupleStruct | StructKind::Tuple => {
                self.check_cancellation()?;
                self.serializer
                    .field_key(content_key)
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;

                let field_count = variant.data.fields.len();
                if field_count == 1 {
                    let inner = enum_
                        .field(0)
                        .map_err(|_| {
                            SerializeError::Internal(Cow::Borrowed("variant field lookup failed"))
                        })?
                        .ok_or(SerializeError::Internal(Cow::Borrowed(
                            "variant reported 1 field but field(0) returned None",
                        )))?;
                    let field_def = variant.data.fields.first().copied();
                    self.with_field_context(field_def, |this| this.serialize_impl(inner))?;
                } else {
                    self.check_cancellation()?;
                    self.serializer
                        .begin_seq()
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    for idx in 0..field_count {
                        self.check_cancellation()?;
                        let inner = enum_
                            .field(idx)
                            .map_err(|_| {
                                SerializeError::Internal(Cow::Borrowed(
                                    "variant field lookup failed",
                                ))
                            })?
                            .ok_or(SerializeError::Internal(Cow::Borrowed(
                                "variant field missing while iterating tuple fields",
                            )))?;
                        self.push(PathSegment::Index(idx));
                        let field_def = variant.data.fields.get(idx).copied();
                        self.with_field_context(field_def, |this| this.serialize_impl(inner))?;
                        self.pop();
                    }
                    self.check_cancellation()?;
                    self.serializer.end_seq().map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                }
            }
        }

        self.pop();
        self.check_cancellation()?;
        self.serializer
            .end_struct()
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;
        Ok(())
    }

    fn serialize_externally_tagged_enum<'mem, 'facet>(
        &mut self,
        enum_: facet_reflect::PeekEnum<'mem, 'facet>,
        variant: &'static facet_core::Variant,
    ) -> Result<(), SerializeError<S::Error>> {
        let field_mode = self.serializer.struct_field_mode();

        // For #[facet(other)] variants with a #[facet(metadata = "tag")] field,
        // use the field's value as the tag name
        let tag_name: Cow<'_, str> = if variant.is_other() {
            let mut tag_value: Option<Cow<'_, str>> = None;
            let fields_iter: alloc::boxed::Box<
                dyn Iterator<Item = (facet_reflect::FieldItem, facet_reflect::Peek<'_, '_>)>,
            > = if field_mode == StructFieldMode::Unnamed {
                alloc::boxed::Box::new(enum_.fields_for_binary_serialize())
            } else {
                alloc::boxed::Box::new(enum_.fields_for_serialize())
            };
            for (field_item, field_value) in fields_iter {
                self.check_cancellation()?;
                if let Some(field) = field_item.field
                    && field.is_variant_tag()
                {
                    if let Ok(opt) = field_value.into_option()
                        && let Some(inner) = opt.value()
                        && let Some(s) = inner.as_str()
                    {
                        tag_value = Some(Cow::Borrowed(s));
                    }
                    break;
                }
            }
            tag_value.unwrap_or_else(|| Cow::Borrowed(variant.effective_name()))
        } else {
            Cow::Borrowed(variant.effective_name())
        };

        // Check if the format wants to handle this with tag syntax
        self.check_cancellation()?;
        let use_tag_syntax = self
            .serializer
            .write_variant_tag(&tag_name)
            .map_err(SerializeError::Backend)?;
        self.check_cancellation()?;

        self.push(PathSegment::Variant(Cow::Owned(tag_name.to_string())));

        let result = if use_tag_syntax {
            self.serialize_variant_after_tag(enum_, variant)
        } else {
            self.serialize_standard_externally_tagged(enum_, variant)
        };

        self.pop();
        result
    }

    fn serialize_variant_after_tag<'mem, 'facet>(
        &mut self,
        enum_: facet_reflect::PeekEnum<'mem, 'facet>,
        variant: &'static facet_core::Variant,
    ) -> Result<(), SerializeError<S::Error>> {
        let field_mode = self.serializer.struct_field_mode();

        match variant.data.kind {
            StructKind::Unit => {
                self.check_cancellation()?;
                self.serializer
                    .finish_variant_tag_unit_payload()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                Ok(())
            }
            StructKind::TupleStruct | StructKind::Tuple => {
                let field_count = variant.data.fields.len();
                if field_count == 1 {
                    let inner = enum_
                        .field(0)
                        .map_err(|_| {
                            SerializeError::Internal(Cow::Borrowed("variant field lookup failed"))
                        })?
                        .ok_or(SerializeError::Internal(Cow::Borrowed(
                            "variant reported 1 field but field(0) returned None",
                        )))?;
                    if let Some(field_def) = variant.data.fields.first().copied()
                        && let Some(proxy_def) =
                            field_def.effective_proxy(self.serializer.format_namespace())
                    {
                        self.with_field_context(Some(field_def), |this| {
                            this.serialize_via_proxy(inner, proxy_def)
                        })?;
                    } else {
                        self.with_field_context(variant.data.fields.first().copied(), |this| {
                            this.serialize_impl(inner)
                        })?;
                    }
                } else {
                    self.check_cancellation()?;
                    self.serializer
                        .begin_seq_after_tag()
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    for idx in 0..field_count {
                        self.check_cancellation()?;
                        let inner = enum_
                            .field(idx)
                            .map_err(|_| {
                                SerializeError::Internal(Cow::Borrowed(
                                    "variant field lookup failed",
                                ))
                            })?
                            .ok_or(SerializeError::Internal(Cow::Borrowed(
                                "variant field missing while iterating tuple fields",
                            )))?;
                        self.push(PathSegment::Index(idx));
                        if let Some(field_def) = variant.data.fields.get(idx).copied()
                            && let Some(proxy_def) =
                                field_def.effective_proxy(self.serializer.format_namespace())
                        {
                            self.with_field_context(Some(field_def), |this| {
                                this.serialize_via_proxy(inner, proxy_def)
                            })?;
                        } else {
                            self.with_field_context(
                                variant.data.fields.get(idx).copied(),
                                |this| this.serialize_impl(inner),
                            )?;
                        }
                        self.pop();
                    }
                    self.check_cancellation()?;
                    self.serializer.end_seq().map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                }
                Ok(())
            }
            StructKind::Struct => {
                let is_other = variant.is_other();
                let fields_iter: alloc::boxed::Box<
                    dyn Iterator<Item = (facet_reflect::FieldItem, facet_reflect::Peek<'_, '_>)>,
                > = if field_mode == StructFieldMode::Unnamed {
                    alloc::boxed::Box::new(enum_.fields_for_binary_serialize())
                } else {
                    alloc::boxed::Box::new(enum_.fields_for_serialize())
                };
                let mut fields = alloc::vec::Vec::new();
                for (field_item, field_value) in fields_iter {
                    self.check_cancellation()?;
                    if is_other
                        && field_item.field.is_some_and(|field| {
                            field.metadata_kind().is_some() || field.is_variant_tag()
                        })
                    {
                        continue;
                    }
                    fields.push((field_item, field_value));
                }

                if fields.is_empty() {
                    self.check_cancellation()?;
                    self.serializer
                        .finish_variant_tag_unit_payload()
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    return Ok(());
                }

                self.check_cancellation()?;
                self.serializer
                    .begin_struct_after_tag()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                sort_fields_if_needed(self.serializer, &mut fields);
                for (field_item, field_value) in fields {
                    self.check_cancellation()?;
                    self.serializer
                        .field_metadata(&field_item)
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    if field_mode == StructFieldMode::Named {
                        self.serializer
                            .field_key(field_item.effective_name())
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                    }
                    self.push(PathSegment::Field(Cow::Owned(
                        field_item.effective_name().to_string(),
                    )));
                    self.serialize_field_value(&field_item, field_value)?;
                    self.pop();
                }
                self.check_cancellation()?;
                self.serializer
                    .end_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                Ok(())
            }
        }
    }

    fn serialize_standard_externally_tagged<'mem, 'facet>(
        &mut self,
        enum_: facet_reflect::PeekEnum<'mem, 'facet>,
        variant: &'static facet_core::Variant,
    ) -> Result<(), SerializeError<S::Error>> {
        let field_mode = self.serializer.struct_field_mode();

        match variant.data.kind {
            StructKind::Unit => {
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::Str(Cow::Borrowed(variant.effective_name())))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                Ok(())
            }
            StructKind::TupleStruct | StructKind::Tuple => {
                self.check_cancellation()?;
                self.serializer
                    .begin_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                self.serializer
                    .field_key(variant.effective_name())
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;

                let field_count = variant.data.fields.len();
                if field_count == 1 {
                    let inner = enum_
                        .field(0)
                        .map_err(|_| {
                            SerializeError::Internal(Cow::Borrowed("variant field lookup failed"))
                        })?
                        .ok_or(SerializeError::Internal(Cow::Borrowed(
                            "variant reported 1 field but field(0) returned None",
                        )))?;
                    if let Some(field_def) = variant.data.fields.first().copied()
                        && let Some(proxy_def) =
                            field_def.effective_proxy(self.serializer.format_namespace())
                    {
                        self.with_field_context(Some(field_def), |this| {
                            this.serialize_via_proxy(inner, proxy_def)
                        })?;
                    } else {
                        self.with_field_context(variant.data.fields.first().copied(), |this| {
                            this.serialize_impl(inner)
                        })?;
                    }
                } else {
                    self.check_cancellation()?;
                    self.serializer
                        .begin_seq()
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    for idx in 0..field_count {
                        self.check_cancellation()?;
                        let inner = enum_
                            .field(idx)
                            .map_err(|_| {
                                SerializeError::Internal(Cow::Borrowed(
                                    "variant field lookup failed",
                                ))
                            })?
                            .ok_or(SerializeError::Internal(Cow::Borrowed(
                                "variant field missing while iterating tuple fields",
                            )))?;
                        self.push(PathSegment::Index(idx));
                        if let Some(field_def) = variant.data.fields.get(idx).copied()
                            && let Some(proxy_def) =
                                field_def.effective_proxy(self.serializer.format_namespace())
                        {
                            self.with_field_context(Some(field_def), |this| {
                                this.serialize_via_proxy(inner, proxy_def)
                            })?;
                        } else {
                            self.with_field_context(
                                variant.data.fields.get(idx).copied(),
                                |this| this.serialize_impl(inner),
                            )?;
                        }
                        self.pop();
                    }
                    self.check_cancellation()?;
                    self.serializer.end_seq().map_err(SerializeError::Backend)?;
                }

                self.check_cancellation()?;
                self.serializer
                    .end_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                Ok(())
            }
            StructKind::Struct => {
                self.check_cancellation()?;
                self.serializer
                    .begin_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                self.serializer
                    .field_key(variant.effective_name())
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                self.serializer
                    .begin_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                let mut fields: alloc::vec::Vec<_> = if field_mode == StructFieldMode::Unnamed {
                    self.collect_fields(enum_.fields_for_binary_serialize())?
                } else {
                    self.collect_fields(enum_.fields_for_serialize())?
                };
                sort_fields_if_needed(self.serializer, &mut fields);
                for (field_item, field_value) in fields {
                    self.check_cancellation()?;
                    self.serializer
                        .field_metadata(&field_item)
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    if field_mode == StructFieldMode::Named {
                        self.serializer
                            .field_key(field_item.effective_name())
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                    }
                    self.push(PathSegment::Field(Cow::Owned(
                        field_item.effective_name().to_string(),
                    )));
                    self.serialize_field_value(&field_item, field_value)?;
                    self.pop();
                }
                self.check_cancellation()?;
                self.serializer
                    .end_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                self.serializer
                    .end_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                Ok(())
            }
        }
    }

    fn serialize_untagged_enum<'mem, 'facet>(
        &mut self,
        enum_: facet_reflect::PeekEnum<'mem, 'facet>,
        variant: &'static facet_core::Variant,
    ) -> Result<(), SerializeError<S::Error>> {
        let field_mode = self.serializer.struct_field_mode();

        match variant.data.kind {
            StructKind::Unit => {
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::Str(Cow::Borrowed(variant.effective_name())))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            StructKind::TupleStruct | StructKind::Tuple => {
                let field_count = variant.data.fields.len();
                if field_count == 1 {
                    let inner = enum_
                        .field(0)
                        .map_err(|_| {
                            SerializeError::Internal(Cow::Borrowed("variant field lookup failed"))
                        })?
                        .ok_or(SerializeError::Internal(Cow::Borrowed(
                            "variant reported 1 field but field(0) returned None",
                        )))?;
                    let field_def = variant.data.fields.first().copied();
                    self.with_field_context(field_def, |this| this.serialize_impl(inner))
                } else {
                    self.check_cancellation()?;
                    self.serializer
                        .begin_seq()
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    for idx in 0..field_count {
                        self.check_cancellation()?;
                        let inner = enum_
                            .field(idx)
                            .map_err(|_| {
                                SerializeError::Internal(Cow::Borrowed(
                                    "variant field lookup failed",
                                ))
                            })?
                            .ok_or(SerializeError::Internal(Cow::Borrowed(
                                "variant field missing while iterating tuple fields",
                            )))?;
                        self.push(PathSegment::Index(idx));
                        let field_def = variant.data.fields.get(idx).copied();
                        self.with_field_context(field_def, |this| this.serialize_impl(inner))?;
                        self.pop();
                    }
                    self.check_cancellation()?;
                    self.serializer.end_seq().map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    Ok(())
                }
            }
            StructKind::Struct => {
                self.check_cancellation()?;
                self.serializer
                    .begin_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                let mut fields: alloc::vec::Vec<_> = if field_mode == StructFieldMode::Unnamed {
                    self.collect_fields(enum_.fields_for_binary_serialize())?
                } else {
                    self.collect_fields(enum_.fields_for_serialize())?
                };
                sort_fields_if_needed(self.serializer, &mut fields);
                for (field_item, field_value) in fields {
                    self.check_cancellation()?;
                    self.serializer
                        .field_metadata(&field_item)
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                    if field_mode == StructFieldMode::Named {
                        self.serializer
                            .field_key(field_item.effective_name())
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                    }
                    self.push(PathSegment::Field(Cow::Owned(
                        field_item.effective_name().to_string(),
                    )));
                    self.serialize_field_value(&field_item, field_value)?;
                    self.pop();
                }
                self.check_cancellation()?;
                self.serializer
                    .end_struct()
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                Ok(())
            }
        }
    }

    fn serialize_dynamic_value<'mem, 'facet>(
        &mut self,
        dynamic: facet_reflect::PeekDynamicValue<'mem, 'facet>,
    ) -> Result<(), SerializeError<S::Error>> {
        let tagged = self.serializer.dynamic_value_encoding() == DynamicValueEncoding::Tagged;

        match dynamic.kind() {
            DynValueKind::Null => {
                if tagged {
                    self.check_cancellation()?;
                    self.serializer
                        .dynamic_value_tag(DynamicValueTag::Null)
                        .map_err(SerializeError::Backend)?;
                }
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::Null)
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            DynValueKind::Bool => {
                let value = dynamic.as_bool().ok_or_else(|| {
                    SerializeError::Internal(Cow::Borrowed("dynamic bool missing value"))
                })?;
                if tagged {
                    self.check_cancellation()?;
                    self.serializer
                        .dynamic_value_tag(DynamicValueTag::Bool)
                        .map_err(SerializeError::Backend)?;
                }
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::Bool(value))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            DynValueKind::Number => {
                if let Some(n) = dynamic.as_i64() {
                    if tagged {
                        self.check_cancellation()?;
                        self.serializer
                            .dynamic_value_tag(DynamicValueTag::I64)
                            .map_err(SerializeError::Backend)?;
                    }
                    self.check_cancellation()?;
                    self.serializer
                        .scalar(ScalarValue::I64(n))
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()
                } else if let Some(n) = dynamic.as_u64() {
                    if tagged {
                        self.check_cancellation()?;
                        self.serializer
                            .dynamic_value_tag(DynamicValueTag::U64)
                            .map_err(SerializeError::Backend)?;
                    }
                    self.check_cancellation()?;
                    self.serializer
                        .scalar(ScalarValue::U64(n))
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()
                } else if let Some(n) = dynamic.as_f64() {
                    if tagged {
                        self.check_cancellation()?;
                        self.serializer
                            .dynamic_value_tag(DynamicValueTag::F64)
                            .map_err(SerializeError::Backend)?;
                    }
                    self.check_cancellation()?;
                    self.serializer
                        .scalar(ScalarValue::F64(n))
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()
                } else {
                    Err(SerializeError::Unsupported(Cow::Borrowed(
                        "dynamic number not representable",
                    )))
                }
            }
            DynValueKind::String => {
                let value = dynamic.as_str().ok_or_else(|| {
                    SerializeError::Internal(Cow::Borrowed("dynamic string missing value"))
                })?;
                if tagged {
                    self.check_cancellation()?;
                    self.serializer
                        .dynamic_value_tag(DynamicValueTag::String)
                        .map_err(SerializeError::Backend)?;
                }
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::Str(Cow::Borrowed(value)))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            DynValueKind::Bytes => {
                let value = dynamic.as_bytes().ok_or_else(|| {
                    SerializeError::Internal(Cow::Borrowed("dynamic bytes missing value"))
                })?;
                if tagged {
                    self.check_cancellation()?;
                    self.serializer
                        .dynamic_value_tag(DynamicValueTag::Bytes)
                        .map_err(SerializeError::Backend)?;
                }
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::Bytes(Cow::Borrowed(value)))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            DynValueKind::Array => {
                let len = dynamic.array_len().ok_or_else(|| {
                    SerializeError::Internal(Cow::Borrowed("dynamic array missing length"))
                })?;
                if tagged {
                    self.check_cancellation()?;
                    self.serializer
                        .dynamic_value_tag(DynamicValueTag::Array)
                        .map_err(SerializeError::Backend)?;
                }
                self.check_cancellation()?;
                self.serializer
                    .begin_seq_with_len(len)
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()?;
                if let Some(iter) = dynamic.array_iter() {
                    for (idx, item) in iter.enumerate() {
                        self.check_cancellation()?;
                        self.push(PathSegment::Index(idx));
                        self.serialize_impl(item)?;
                        self.pop();
                    }
                }
                self.check_cancellation()?;
                self.serializer.end_seq().map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            DynValueKind::Object => {
                let len = dynamic.object_len().ok_or_else(|| {
                    SerializeError::Internal(Cow::Borrowed("dynamic object missing length"))
                })?;
                if tagged {
                    self.check_cancellation()?;
                    self.serializer
                        .dynamic_value_tag(DynamicValueTag::Object)
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                }
                match self.serializer.map_encoding() {
                    MapEncoding::Pairs => {
                        self.check_cancellation()?;
                        self.serializer
                            .begin_map_with_len(len)
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                        if let Some(iter) = dynamic.object_iter() {
                            for (key, value) in iter {
                                self.check_cancellation()?;
                                self.serializer
                                    .scalar(ScalarValue::Str(Cow::Borrowed(key)))
                                    .map_err(SerializeError::Backend)?;
                                self.check_cancellation()?;
                                self.push(PathSegment::Field(Cow::Owned(key.to_string())));
                                self.serialize_impl(value)?;
                                self.pop();
                            }
                        }
                        self.check_cancellation()?;
                        self.serializer.end_map().map_err(SerializeError::Backend)?;
                        self.check_cancellation()
                    }
                    MapEncoding::Struct => {
                        self.check_cancellation()?;
                        self.serializer
                            .begin_struct()
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()?;
                        if let Some(iter) = dynamic.object_iter() {
                            for (key, value) in iter {
                                self.check_cancellation()?;
                                self.serializer
                                    .field_key(key)
                                    .map_err(SerializeError::Backend)?;
                                self.check_cancellation()?;
                                self.push(PathSegment::Field(Cow::Owned(key.to_string())));
                                self.serialize_impl(value)?;
                                self.pop();
                            }
                        }
                        self.check_cancellation()?;
                        self.serializer
                            .end_struct()
                            .map_err(SerializeError::Backend)?;
                        self.check_cancellation()
                    }
                }
            }
            DynValueKind::DateTime => {
                let dt = dynamic.as_datetime().ok_or_else(|| {
                    SerializeError::Internal(Cow::Borrowed("dynamic datetime missing value"))
                })?;
                if tagged {
                    self.check_cancellation()?;
                    self.serializer
                        .dynamic_value_tag(DynamicValueTag::DateTime)
                        .map_err(SerializeError::Backend)?;
                    self.check_cancellation()?;
                }
                let s = format_dyn_datetime(dt);
                self.check_cancellation()?;
                self.serializer
                    .scalar(ScalarValue::Str(Cow::Owned(s)))
                    .map_err(SerializeError::Backend)?;
                self.check_cancellation()
            }
            DynValueKind::QName | DynValueKind::Uuid => Err(SerializeError::Unsupported(
                Cow::Borrowed("dynamic QName/Uuid serialization is not supported"),
            )),
            // A dynamic value kind added since this match was written.
            _ => Err(SerializeError::Internal(Cow::Borrowed(
                "unsupported dynamic value kind",
            ))),
        }
    }

    fn serialize_via_proxy<'mem, 'facet>(
        &mut self,
        value: Peek<'mem, 'facet>,
        proxy_def: &'static facet_core::ProxyDef,
    ) -> Result<(), SerializeError<S::Error>> {
        self.check_cancellation()?;
        proxy_def.shape.layout.sized_layout().map_err(|_| {
            SerializeError::Unsupported(Cow::Borrowed("proxy type must be sized for serialization"))
        })?;

        // OwnedPeek releases the converted value and allocation even when a
        // backend or cancellation predicate unwinds during traversal. The
        // reflection boundary also owns deallocation after a failed conversion.
        let proxy = value
            .custom_serialization_with_proxy(proxy_def)
            .map_err(|error| SerializeError::Unsupported(Cow::Owned(error.to_string())))?;
        self.serialize_impl(proxy.as_peek())
    }

    fn deref_if_pointer<'mem, 'facet>(
        &self,
        mut value: Peek<'mem, 'facet>,
    ) -> Result<Peek<'mem, 'facet>, SerializeError<S::Error>> {
        loop {
            self.check_cancellation()?;
            if let Ok(pointer) = value.into_pointer()
                && let Some(target) = pointer.borrow_inner()
            {
                value = target;
            } else {
                return Ok(value);
            }
        }
    }

    fn extract_string_from_peek<'mem, 'facet>(
        &self,
        value: Peek<'mem, 'facet>,
    ) -> Result<Option<&'mem str>, SerializeError<S::Error>> {
        self.check_cancellation()?;
        if let Some(value) = value.as_str() {
            return Ok(Some(value));
        }
        if value.shape().is_metadata_container()
            && let Ok(container) = value.into_struct()
        {
            for (field, value) in container.fields() {
                self.check_cancellation()?;
                if !field.is_metadata() {
                    return self.extract_string_from_peek(value);
                }
            }
        }
        Ok(None)
    }
}

impl<E: Debug> std::error::Error for SerializeError<E> {}

/// Get a human-readable name for a Def variant.
fn def_kind_name(def: &Def) -> &'static str {
    match def {
        Def::Undefined => "Undefined",
        Def::Scalar => "Scalar",
        Def::Map(_) => "Map",
        Def::Set(_) => "Set",
        Def::List(_) => "List",
        Def::Array(_) => "Array",
        Def::NdArray(_) => "NdArray",
        Def::Slice(_) => "Slice",
        Def::Option(_) => "Option",
        Def::Result(_) => "Result",
        Def::DynamicValue(_) => "DynamicValue",
        Def::Pointer(_) => "Pointer",
        _ => "Unknown",
    }
}

/// Serialize a root value using the shared traversal logic.
pub fn serialize_root<'mem, 'facet, S>(
    serializer: &mut S,
    value: Peek<'mem, 'facet>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    let mut ctx = SerializeContext::new(serializer);
    ctx.serialize(value)
}

fn check_cancellation<E: Debug>(
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<E>> {
    if should_cancel.is_some_and(|should_cancel| should_cancel()) {
        Err(SerializeError::Cancelled)
    } else {
        Ok(())
    }
}

/// Serialize a root value, stopping when a borrowed predicate returns `true`.
///
/// The predicate is checked between reflected values, fields, and backend calls.
/// It may borrow non-thread-safe local state. This is cooperative cancellation:
/// a long-running backend or user-provided formatter/proxy can only be stopped
/// after its call returns unless it performs additional checks itself. A backend
/// may have emitted a partial document when cancellation is returned; discard it.
pub fn serialize_root_with_cancellation<'mem, 'facet, S>(
    serializer: &mut S,
    value: Peek<'mem, 'facet>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    SerializeContext::with_cancellation(serializer, should_cancel).serialize(value)
}

/// Helper to sort fields according to format preference (currently a no-op).
fn sort_fields_if_needed<'mem, 'facet, S>(
    _serializer: &S,
    _fields: &mut alloc::vec::Vec<(facet_reflect::FieldItem, Peek<'mem, 'facet>)>,
) where
    S: FormatSerializer,
{
    // Currently only Declaration order is supported, which preserves the original order.
}

fn format_dyn_datetime(
    (year, month, day, hour, minute, second, nanos, kind): (
        i32,
        u8,
        u8,
        u8,
        u8,
        u8,
        u32,
        DynDateTimeKind,
    ),
) -> String {
    let mut out = String::new();
    match kind {
        DynDateTimeKind::Offset { offset_minutes } => {
            let _ = write!(
                out,
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                year, month, day, hour, minute, second
            );
            if nanos > 0 {
                let _ = write!(out, ".{:09}", nanos);
            }
            if offset_minutes == 0 {
                out.push('Z');
            } else {
                let sign = if offset_minutes >= 0 { '+' } else { '-' };
                let abs = offset_minutes.unsigned_abs();
                let _ = write!(out, "{}{:02}:{:02}", sign, abs / 60, abs % 60);
            }
        }
        DynDateTimeKind::LocalDateTime => {
            let _ = write!(
                out,
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                year, month, day, hour, minute, second
            );
            if nanos > 0 {
                let _ = write!(out, ".{:09}", nanos);
            }
        }
        DynDateTimeKind::LocalDate => {
            let _ = write!(out, "{:04}-{:02}-{:02}", year, month, day);
        }
        DynDateTimeKind::LocalTime => {
            let _ = write!(out, "{:02}:{:02}:{:02}", hour, minute, second);
            if nanos > 0 {
                let _ = write!(out, ".{:09}", nanos);
            }
        }
        // A datetime kind added since this match was written: best-effort
        // date-and-time without zone information.
        _ => {
            let _ = write!(
                out,
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                year, month, day, hour, minute, second
            );
            if nanos > 0 {
                let _ = write!(out, ".{:09}", nanos);
            }
        }
    }
    out
}

fn serialize_numeric_enum<S>(
    serializer: &mut S,
    variant: &'static facet_core::Variant,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    let discriminant = variant
        .discriminant
        .ok_or(SerializeError::Unsupported(Cow::Borrowed(
            "Enum without a discriminant",
        )))?;
    serializer
        .scalar(ScalarValue::I64(discriminant))
        .map_err(SerializeError::Backend)
}

// ─────────────────────────────────────────────────────────────────────────────
// Shape-guided serialization of dynamic values
// ─────────────────────────────────────────────────────────────────────────────

/// Serialize a dynamic value (like `facet_value::Value`) according to a target shape.
///
/// This is the inverse of `FormatDeserializer::deserialize_with_shape`. It allows serializing
/// a `Value` as if it were a typed value matching the shape, without the dynamic value's
/// type discriminants.
///
/// This is useful for non-self-describing formats like postcard where you want to:
/// 1. Parse JSON into a `Value`
/// 2. Serialize it to postcard bytes matching a typed schema
///
/// # Arguments
///
/// * `serializer` - The format serializer to use
/// * `value` - A `Peek` into a dynamic value type (like `facet_value::Value`)
/// * `target_shape` - The shape describing the expected wire format
///
/// # Errors
///
/// Returns an error if:
/// - The value is not a dynamic value type
/// - The value's structure doesn't match the target shape
pub fn serialize_value_with_shape<S>(
    serializer: &mut S,
    value: Peek<'_, '_>,
    target_shape: &'static Shape,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    let dynamic = value.into_dynamic_value().map_err(|_| {
        SerializeError::Unsupported(Cow::Borrowed(
            "serialize_value_with_shape requires a DynamicValue type",
        ))
    })?;

    serialize_dynamic_with_shape(serializer, dynamic, target_shape, value.shape(), None)
}

/// Serialize a dynamic value with a target shape and a borrowed cancellation predicate.
///
/// Cancellation and partial-output behavior match [`serialize_root_with_cancellation`].
pub fn serialize_value_with_shape_with_cancellation<S>(
    serializer: &mut S,
    value: Peek<'_, '_>,
    target_shape: &'static Shape,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    check_cancellation(Some(should_cancel))?;
    let dynamic = value.into_dynamic_value().map_err(|_| {
        SerializeError::Unsupported(Cow::Borrowed(
            "serialize_value_with_shape requires a DynamicValue type",
        ))
    })?;
    serialize_dynamic_with_shape(
        serializer,
        dynamic,
        target_shape,
        value.shape(),
        Some(should_cancel),
    )
}

fn serialize_dynamic_with_shape<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    target_shape: &'static Shape,
    value_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    check_cancellation(should_cancel)?;
    use facet_core::{ListDef, OptionDef, ScalarType as CoreScalarType, Type, UserType};

    // Handle smart pointers - unwrap to the inner shape
    if let Def::Pointer(ptr_def) = target_shape.def
        && let Some(pointee) = ptr_def.pointee
    {
        return serialize_dynamic_with_shape(
            serializer,
            dynamic,
            pointee,
            value_shape,
            should_cancel,
        );
    }

    // Handle transparent wrappers via .inner
    if let Some(inner_shape) = target_shape.inner {
        // Skip collection types that have .inner for variance but aren't transparent wrappers
        if !matches!(
            target_shape.def,
            Def::List(_) | Def::Map(_) | Def::Set(_) | Def::Array(_)
        ) {
            return serialize_dynamic_with_shape(
                serializer,
                dynamic,
                inner_shape,
                value_shape,
                should_cancel,
            );
        }
    }

    // Handle Option<T>
    if let Def::Option(OptionDef { t: inner_shape, .. }) = target_shape.def {
        return serialize_option_from_dynamic(
            serializer,
            dynamic,
            inner_shape,
            value_shape,
            should_cancel,
        );
    }

    // Handle List/Vec
    if let Def::List(ListDef { t: item_shape, .. }) = target_shape.def {
        return serialize_list_from_dynamic(
            serializer,
            dynamic,
            item_shape,
            value_shape,
            should_cancel,
        );
    }

    // Handle Array [T; N]
    if let Def::Array(array_def) = target_shape.def {
        return serialize_array_from_dynamic(
            serializer,
            dynamic,
            array_def.t,
            value_shape,
            should_cancel,
        );
    }

    // Handle Map
    if let Def::Map(map_def) = target_shape.def {
        return serialize_map_from_dynamic(
            serializer,
            dynamic,
            map_def.k,
            map_def.v,
            value_shape,
            should_cancel,
        );
    }

    // Handle scalars
    if let Some(scalar_type) = CoreScalarType::try_from_shape(target_shape) {
        return serialize_scalar_from_dynamic(serializer, dynamic, scalar_type, should_cancel);
    }

    // Handle structs and enums by Type
    match target_shape.ty {
        Type::User(UserType::Struct(struct_def)) => serialize_struct_from_dynamic(
            serializer,
            dynamic,
            struct_def,
            value_shape,
            should_cancel,
        ),
        Type::User(UserType::Enum(enum_def)) => serialize_enum_from_dynamic(
            serializer,
            dynamic,
            enum_def,
            target_shape,
            value_shape,
            should_cancel,
        ),
        _ => Err(SerializeError::Unsupported(Cow::Owned(alloc::format!(
            "unsupported target shape for serialize_value_with_shape: {}",
            target_shape
        )))),
    }
}

fn serialize_option_from_dynamic<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    inner_shape: &'static Shape,
    value_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    if dynamic.kind() == DynValueKind::Null {
        check_cancellation(should_cancel)?;
        serializer
            .serialize_none()
            .map_err(SerializeError::Backend)?;
        check_cancellation(should_cancel)
    } else {
        check_cancellation(should_cancel)?;
        serializer
            .begin_option_some()
            .map_err(SerializeError::Backend)?;
        check_cancellation(should_cancel)?;
        serialize_dynamic_with_shape(serializer, dynamic, inner_shape, value_shape, should_cancel)
    }
}

fn serialize_list_from_dynamic<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    item_shape: &'static Shape,
    value_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    let len = dynamic.array_len().ok_or_else(|| {
        SerializeError::Unsupported(Cow::Borrowed(
            "expected array value for list/vec target shape",
        ))
    })?;

    check_cancellation(should_cancel)?;
    serializer
        .begin_seq_with_len(len)
        .map_err(SerializeError::Backend)?;
    check_cancellation(should_cancel)?;

    if let Some(iter) = dynamic.array_iter() {
        for elem in iter {
            check_cancellation(should_cancel)?;
            let elem_dyn = elem.into_dynamic_value().map_err(|_| {
                SerializeError::Internal(Cow::Borrowed("array element is not a dynamic value"))
            })?;
            serialize_dynamic_with_shape(
                serializer,
                elem_dyn,
                item_shape,
                value_shape,
                should_cancel,
            )?;
        }
    }

    check_cancellation(should_cancel)?;
    serializer.end_seq().map_err(SerializeError::Backend)?;
    check_cancellation(should_cancel)
}

fn serialize_array_from_dynamic<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    item_shape: &'static Shape,
    value_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    // Arrays don't have length prefix in postcard
    check_cancellation(should_cancel)?;
    serializer.begin_seq().map_err(SerializeError::Backend)?;
    check_cancellation(should_cancel)?;

    if let Some(iter) = dynamic.array_iter() {
        for elem in iter {
            check_cancellation(should_cancel)?;
            let elem_dyn = elem.into_dynamic_value().map_err(|_| {
                SerializeError::Internal(Cow::Borrowed("array element is not a dynamic value"))
            })?;
            serialize_dynamic_with_shape(
                serializer,
                elem_dyn,
                item_shape,
                value_shape,
                should_cancel,
            )?;
        }
    }

    check_cancellation(should_cancel)?;
    serializer.end_seq().map_err(SerializeError::Backend)?;
    check_cancellation(should_cancel)
}

fn serialize_map_from_dynamic<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    key_shape: &'static Shape,
    value_shape_inner: &'static Shape,
    value_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    let len = dynamic.object_len().ok_or_else(|| {
        SerializeError::Unsupported(Cow::Borrowed("expected object value for map target shape"))
    })?;

    match serializer.map_encoding() {
        MapEncoding::Pairs => {
            check_cancellation(should_cancel)?;
            serializer
                .begin_map_with_len(len)
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)?;

            if let Some(iter) = dynamic.object_iter() {
                for (key, val) in iter {
                    check_cancellation(should_cancel)?;
                    // Serialize key according to key_shape
                    serialize_string_as_scalar(serializer, key, key_shape, should_cancel)?;
                    // Serialize value
                    let val_dyn = val.into_dynamic_value().map_err(|_| {
                        SerializeError::Internal(Cow::Borrowed(
                            "object value is not a dynamic value",
                        ))
                    })?;
                    serialize_dynamic_with_shape(
                        serializer,
                        val_dyn,
                        value_shape_inner,
                        value_shape,
                        should_cancel,
                    )?;
                }
            }

            check_cancellation(should_cancel)?;
            serializer.end_map().map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        MapEncoding::Struct => {
            check_cancellation(should_cancel)?;
            serializer.begin_struct().map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)?;

            if let Some(iter) = dynamic.object_iter() {
                for (key, val) in iter {
                    check_cancellation(should_cancel)?;
                    serializer.field_key(key).map_err(SerializeError::Backend)?;
                    check_cancellation(should_cancel)?;
                    let val_dyn = val.into_dynamic_value().map_err(|_| {
                        SerializeError::Internal(Cow::Borrowed(
                            "object value is not a dynamic value",
                        ))
                    })?;
                    serialize_dynamic_with_shape(
                        serializer,
                        val_dyn,
                        value_shape_inner,
                        value_shape,
                        should_cancel,
                    )?;
                }
            }

            check_cancellation(should_cancel)?;
            serializer.end_struct().map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
    }
}

fn serialize_string_as_scalar<S>(
    serializer: &mut S,
    s: &str,
    _key_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    // For now, serialize string keys directly
    // TODO: Handle non-string key types if needed
    check_cancellation(should_cancel)?;
    serializer
        .scalar(ScalarValue::Str(Cow::Borrowed(s)))
        .map_err(SerializeError::Backend)?;
    check_cancellation(should_cancel)
}

fn serialize_scalar_from_dynamic<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    scalar_type: facet_core::ScalarType,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    use facet_core::ScalarType as ST;

    match scalar_type {
        ST::Unit => {
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::Null)
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::Bool => {
            let v = dynamic
                .as_bool()
                .ok_or_else(|| SerializeError::Unsupported(Cow::Borrowed("expected bool value")))?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::Bool(v))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::Char => {
            let s = dynamic.as_str().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected string value for char"))
            })?;
            let c = s.chars().next().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected non-empty string for char"))
            })?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::Char(c))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::Str | ST::String | ST::CowStr => {
            let s = dynamic.as_str().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected string value"))
            })?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::Str(Cow::Borrowed(s)))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::U8 | ST::U16 | ST::U32 | ST::U64 | ST::USize => {
            let n = dynamic.as_u64().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected unsigned integer value"))
            })?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::U64(n))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::U128 => {
            let n = dynamic.as_u64().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected unsigned integer value"))
            })?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::U128(n as u128))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::I8 | ST::I16 | ST::I32 | ST::I64 | ST::ISize => {
            let n = dynamic.as_i64().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected signed integer value"))
            })?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::I64(n))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::I128 => {
            let n = dynamic.as_i64().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected signed integer value"))
            })?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::I128(n as i128))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        ST::F32 | ST::F64 => {
            let n = dynamic.as_f64().ok_or_else(|| {
                SerializeError::Unsupported(Cow::Borrowed("expected float value"))
            })?;
            check_cancellation(should_cancel)?;
            serializer
                .scalar(ScalarValue::F64(n))
                .map_err(SerializeError::Backend)?;
            check_cancellation(should_cancel)
        }
        _ => Err(SerializeError::Unsupported(Cow::Owned(alloc::format!(
            "unsupported scalar type: {:?}",
            scalar_type
        )))),
    }
}

fn serialize_struct_from_dynamic<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    struct_def: facet_core::StructType,
    value_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    let is_tuple = matches!(struct_def.kind, StructKind::Tuple | StructKind::TupleStruct);

    if is_tuple {
        // For tuples, expect an array value
        check_cancellation(should_cancel)?;
        serializer.begin_seq().map_err(SerializeError::Backend)?;
        check_cancellation(should_cancel)?;

        let iter = dynamic.array_iter().ok_or_else(|| {
            SerializeError::Unsupported(Cow::Borrowed("expected array value for tuple"))
        })?;

        for (field, elem) in struct_def.fields.iter().zip(iter) {
            check_cancellation(should_cancel)?;
            let elem_dyn = elem.into_dynamic_value().map_err(|_| {
                SerializeError::Internal(Cow::Borrowed("tuple element is not a dynamic value"))
            })?;
            serialize_dynamic_with_shape(
                serializer,
                elem_dyn,
                field.shape(),
                value_shape,
                should_cancel,
            )?;
        }

        check_cancellation(should_cancel)?;
        serializer.end_seq().map_err(SerializeError::Backend)?;
        check_cancellation(should_cancel)
    } else {
        // For named structs, expect an object value
        let field_mode = serializer.struct_field_mode();

        check_cancellation(should_cancel)?;
        serializer.begin_struct().map_err(SerializeError::Backend)?;
        check_cancellation(should_cancel)?;

        for field in struct_def.fields {
            check_cancellation(should_cancel)?;
            // Skip metadata fields
            if field.is_metadata() {
                continue;
            }

            let field_name = field.name;
            let field_value = dynamic.object_get(field_name).ok_or_else(|| {
                SerializeError::Unsupported(Cow::Owned(alloc::format!(
                    "missing field '{}' in object",
                    field_name
                )))
            })?;

            if field_mode == StructFieldMode::Named {
                check_cancellation(should_cancel)?;
                serializer
                    .field_key(field_name)
                    .map_err(SerializeError::Backend)?;
                check_cancellation(should_cancel)?;
            }

            let field_dyn = field_value.into_dynamic_value().map_err(|_| {
                SerializeError::Internal(Cow::Borrowed("field value is not a dynamic value"))
            })?;
            serialize_dynamic_with_shape(
                serializer,
                field_dyn,
                field.shape(),
                value_shape,
                should_cancel,
            )?;
        }

        check_cancellation(should_cancel)?;
        serializer.end_struct().map_err(SerializeError::Backend)?;
        check_cancellation(should_cancel)
    }
}

fn find_variant_with_cancellation<E: Debug>(
    enum_def: facet_core::EnumType,
    should_cancel: Option<&dyn Fn() -> bool>,
    matches: impl Fn(&facet_core::Variant) -> bool,
) -> Result<Option<(usize, &'static facet_core::Variant)>, SerializeError<E>> {
    for (index, variant) in enum_def.variants.iter().enumerate() {
        check_cancellation(should_cancel)?;
        if matches(variant) {
            return Ok(Some((index, variant)));
        }
    }
    Ok(None)
}

fn serialize_enum_from_dynamic<S>(
    serializer: &mut S,
    dynamic: facet_reflect::PeekDynamicValue<'_, '_>,
    enum_def: facet_core::EnumType,
    target_shape: &'static Shape,
    value_shape: &'static Shape,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Result<(), SerializeError<S::Error>>
where
    S: FormatSerializer,
{
    // For index-based encoding (postcard), we need to:
    // 1. Determine the variant from the Value
    // 2. Emit the variant index
    // 3. Serialize the variant's payload

    let use_index = serializer.enum_variant_encoding() == EnumVariantEncoding::Index;

    match dynamic.kind() {
        // Unit variant represented as a string
        DynValueKind::String => {
            let variant_name = dynamic.as_str().ok_or_else(|| {
                SerializeError::Internal(Cow::Borrowed("expected string for unit variant"))
            })?;

            let (variant_index, variant) =
                find_variant_with_cancellation::<S::Error>(enum_def, should_cancel, |variant| {
                    variant.effective_name() == variant_name
                })?
                .ok_or_else(|| {
                    SerializeError::Unsupported(Cow::Owned(alloc::format!(
                        "unknown variant '{}'",
                        variant_name
                    )))
                })?;

            if use_index {
                check_cancellation(should_cancel)?;
                serializer
                    .begin_enum_variant(variant_index, variant.effective_name())
                    .map_err(SerializeError::Backend)?;
                check_cancellation(should_cancel)?;
                // Unit variant has no payload
                Ok(())
            } else {
                check_cancellation(should_cancel)?;
                serializer
                    .scalar(ScalarValue::Str(Cow::Borrowed(variant.effective_name())))
                    .map_err(SerializeError::Backend)?;
                check_cancellation(should_cancel)
            }
        }

        // Variant with payload represented as object { "VariantName": payload }
        DynValueKind::Object => {
            // For externally tagged enums, the object has a single key = variant name
            let obj_len = dynamic.object_len().unwrap_or(0);
            if obj_len != 1 {
                return Err(SerializeError::Unsupported(Cow::Owned(alloc::format!(
                    "expected single-key object for enum variant, got {} keys",
                    obj_len
                ))));
            }

            let (variant_name, payload) = dynamic.object_get_entry(0).ok_or_else(|| {
                SerializeError::Internal(Cow::Borrowed("expected object entry for enum variant"))
            })?;

            let (variant_index, variant) =
                find_variant_with_cancellation::<S::Error>(enum_def, should_cancel, |variant| {
                    variant.effective_name() == variant_name
                })?
                .ok_or_else(|| {
                    SerializeError::Unsupported(Cow::Owned(alloc::format!(
                        "unknown variant '{}'",
                        variant_name
                    )))
                })?;

            let payload_dyn = payload.into_dynamic_value().map_err(|_| {
                SerializeError::Internal(Cow::Borrowed("variant payload is not a dynamic value"))
            })?;

            if use_index {
                check_cancellation(should_cancel)?;
                serializer
                    .begin_enum_variant(variant_index, variant.effective_name())
                    .map_err(SerializeError::Backend)?;
                check_cancellation(should_cancel)?;

                // Serialize payload based on variant kind
                match variant.data.kind {
                    StructKind::Unit => {
                        // No payload to serialize
                    }
                    StructKind::TupleStruct | StructKind::Tuple => {
                        if variant.data.fields.len() == 1 {
                            // Newtype variant - serialize the single field directly
                            serialize_dynamic_with_shape(
                                serializer,
                                payload_dyn,
                                variant.data.fields[0].shape(),
                                value_shape,
                                should_cancel,
                            )?;
                        } else {
                            // Multi-field tuple variant - expect array
                            let iter = payload_dyn.array_iter().ok_or_else(|| {
                                SerializeError::Unsupported(Cow::Borrowed(
                                    "expected array for tuple variant payload",
                                ))
                            })?;

                            for (field, elem) in variant.data.fields.iter().zip(iter) {
                                check_cancellation(should_cancel)?;
                                let elem_dyn = elem.into_dynamic_value().map_err(|_| {
                                    SerializeError::Internal(Cow::Borrowed(
                                        "tuple element is not a dynamic value",
                                    ))
                                })?;
                                serialize_dynamic_with_shape(
                                    serializer,
                                    elem_dyn,
                                    field.shape(),
                                    value_shape,
                                    should_cancel,
                                )?;
                            }
                        }
                    }
                    StructKind::Struct => {
                        // Struct variant - expect object
                        for field in variant.data.fields {
                            check_cancellation(should_cancel)?;
                            let field_value =
                                payload_dyn.object_get(field.name).ok_or_else(|| {
                                    SerializeError::Unsupported(Cow::Owned(alloc::format!(
                                        "missing field '{}' in struct variant",
                                        field.name
                                    )))
                                })?;
                            let field_dyn = field_value.into_dynamic_value().map_err(|_| {
                                SerializeError::Internal(Cow::Borrowed(
                                    "field value is not a dynamic value",
                                ))
                            })?;
                            serialize_dynamic_with_shape(
                                serializer,
                                field_dyn,
                                field.shape(),
                                value_shape,
                                should_cancel,
                            )?;
                        }
                    }
                }

                Ok(())
            } else {
                // Externally tagged representation
                check_cancellation(should_cancel)?;
                serializer.begin_struct().map_err(SerializeError::Backend)?;
                check_cancellation(should_cancel)?;

                serializer
                    .field_key(variant.effective_name())
                    .map_err(SerializeError::Backend)?;
                check_cancellation(should_cancel)?;

                match variant.data.kind {
                    StructKind::Unit => {
                        check_cancellation(should_cancel)?;
                        serializer
                            .scalar(ScalarValue::Null)
                            .map_err(SerializeError::Backend)?;
                        check_cancellation(should_cancel)?;
                    }
                    StructKind::TupleStruct | StructKind::Tuple => {
                        if variant.data.fields.len() == 1 {
                            serialize_dynamic_with_shape(
                                serializer,
                                payload_dyn,
                                variant.data.fields[0].shape(),
                                value_shape,
                                should_cancel,
                            )?;
                        } else {
                            check_cancellation(should_cancel)?;
                            serializer.begin_seq().map_err(SerializeError::Backend)?;
                            check_cancellation(should_cancel)?;
                            let iter = payload_dyn.array_iter().ok_or_else(|| {
                                SerializeError::Unsupported(Cow::Borrowed(
                                    "expected array for tuple variant",
                                ))
                            })?;
                            for (field, elem) in variant.data.fields.iter().zip(iter) {
                                check_cancellation(should_cancel)?;
                                let elem_dyn = elem.into_dynamic_value().map_err(|_| {
                                    SerializeError::Internal(Cow::Borrowed(
                                        "element is not a dynamic value",
                                    ))
                                })?;
                                serialize_dynamic_with_shape(
                                    serializer,
                                    elem_dyn,
                                    field.shape(),
                                    value_shape,
                                    should_cancel,
                                )?;
                            }
                            check_cancellation(should_cancel)?;
                            serializer.end_seq().map_err(SerializeError::Backend)?;
                            check_cancellation(should_cancel)?;
                        }
                    }
                    StructKind::Struct => {
                        check_cancellation(should_cancel)?;
                        serializer.begin_struct().map_err(SerializeError::Backend)?;
                        check_cancellation(should_cancel)?;
                        for field in variant.data.fields {
                            check_cancellation(should_cancel)?;
                            let field_value =
                                payload_dyn.object_get(field.name).ok_or_else(|| {
                                    SerializeError::Unsupported(Cow::Owned(alloc::format!(
                                        "missing field '{}'",
                                        field.name
                                    )))
                                })?;
                            check_cancellation(should_cancel)?;
                            serializer
                                .field_key(field.name)
                                .map_err(SerializeError::Backend)?;
                            check_cancellation(should_cancel)?;
                            let field_dyn = field_value.into_dynamic_value().map_err(|_| {
                                SerializeError::Internal(Cow::Borrowed(
                                    "field is not a dynamic value",
                                ))
                            })?;
                            serialize_dynamic_with_shape(
                                serializer,
                                field_dyn,
                                field.shape(),
                                value_shape,
                                should_cancel,
                            )?;
                        }
                        check_cancellation(should_cancel)?;
                        serializer.end_struct().map_err(SerializeError::Backend)?;
                    }
                }

                check_cancellation(should_cancel)?;
                serializer.end_struct().map_err(SerializeError::Backend)?;
                check_cancellation(should_cancel)
            }
        }

        // Null could be a unit variant named "Null" (untagged representation)
        DynValueKind::Null => {
            // Check if there's a Null variant or fallback for Option-like enums
            // Note: we match against the Rust name (v.name) since these are well-known Rust identifiers
            if let Some((variant_index, variant)) =
                find_variant_with_cancellation::<S::Error>(enum_def, should_cancel, |variant| {
                    variant.name.eq_ignore_ascii_case("null") || variant.name == "None"
                })?
            {
                if use_index {
                    check_cancellation(should_cancel)?;
                    serializer
                        .begin_enum_variant(variant_index, variant.effective_name())
                        .map_err(SerializeError::Backend)?;
                    check_cancellation(should_cancel)?;
                    Ok(())
                } else {
                    check_cancellation(should_cancel)?;
                    serializer
                        .scalar(ScalarValue::Str(Cow::Borrowed(variant.effective_name())))
                        .map_err(SerializeError::Backend)?;
                    check_cancellation(should_cancel)
                }
            } else {
                Err(SerializeError::Unsupported(Cow::Borrowed(
                    "null value for enum without null/None variant",
                )))
            }
        }

        _ => {
            // For untagged enums, we might need to try matching variants
            // This is a simplified implementation - could be extended
            let _ = target_shape; // Suppress unused warning
            Err(SerializeError::Unsupported(Cow::Owned(alloc::format!(
                "unsupported dynamic value kind {:?} for enum serialization",
                dynamic.kind()
            ))))
        }
    }
}
