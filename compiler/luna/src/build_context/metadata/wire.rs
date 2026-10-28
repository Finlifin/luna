//! Version 1 immutable metadata transport, independent of diagnostic Display.
//! Little-endian u64 lengths and identities; UTF-8 byte strings; explicit tags.
//! Decoding checks representation and closure. Value admission remains in the VM.
use super::{Entry, Input};
use comptime_runtime::{Budget, Integer};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term as T};

pub const MAX_BYTES: usize = 128 * 1024;
const MAGIC: &[u8] = b"FMD\x01";
const PRIMITIVES: &[P] = &[
    P::Type,
    P::Object,
    P::Bool,
    P::Char,
    P::Str,
    P::Symbol,
    P::Integer,
    P::Float,
    P::Unit,
    P::Void,
    P::Never,
    P::I8,
    P::I16,
    P::I32,
    P::I64,
    P::I128,
    P::U8,
    P::U16,
    P::U32,
    P::U64,
    P::U128,
    P::Isize,
    P::Usize,
    P::F32,
    P::F64,
];
fn failure(message: impl std::fmt::Display) -> String {
    format!("metadata wire: {message}")
}
fn tick(fuel: &mut usize, depth: usize) -> Result<(), String> {
    if depth >= 64 {
        return Err(failure("nesting limit exceeded"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| failure("node budget exceeded"))?;
    Ok(())
}
fn sort_tag(sort: Sort) -> u8 {
    match sort {
        Sort::Type => 0,
        Sort::ComptimeValue => 1,
        Sort::Runtime => 2,
    }
}

/// Canonical hex argument for --package-metadata-v1. No files are consulted.
pub fn encode_hex(inputs: &[Input]) -> Result<String, String> {
    let mut writer = Writer {
        bytes: Vec::new(),
        fuel: 100_000,
        arithmetic: Budget::default(),
    };
    writer.put(MAGIC)?;
    writer.number(inputs.len() as u64)?;
    for input in inputs {
        writer.number(input.identity)?;
        writer.number(input.fields.len() as u64)?;
        for (name, entry) in &input.fields {
            writer.text(name)?;
            writer.term(entry.ty(), 0)?;
            writer.term(entry.value(), 0)?;
        }
    }
    const HEX: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(writer.bytes.len() * 2);
    for byte in writer.bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(result)
}

pub fn decode_hex(text: &str) -> Result<Vec<Input>, String> {
    if text.len() > MAX_BYTES * 2 || text.len() % 2 != 0 {
        return Err(failure("invalid encoded length"));
    }
    let digit = |b| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(failure("invalid hex digit")),
    };
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks_exact(2) {
        bytes.push(digit(pair[0])? * 16 + digit(pair[1])?);
    }
    let mut reader = Reader {
        bytes: &bytes,
        at: 0,
        fuel: 100_000,
        arithmetic: Budget::default(),
        admission_fuel: 100_000,
    };
    if reader.take(4)? != MAGIC {
        return Err(failure("unsupported header or version"));
    }
    let count = reader.count()?;
    let mut inputs = Vec::new();
    for _ in 0..count {
        let identity = reader.number()?;
        let fields_count = reader.count()?;
        let mut fields = Vec::new();
        for _ in 0..fields_count {
            let name = reader.text()?.to_owned();
            let ty = reader.term(0)?;
            let value = reader.term(0)?;
            let entry = Entry::new(ty, value, &mut reader.admission_fuel).map_err(failure)?;
            fields.push((name, entry));
        }
        inputs.push(Input { identity, fields });
    }
    if reader.at != bytes.len() {
        return Err(failure("trailing bytes"));
    }
    Ok(inputs)
}

struct Writer {
    bytes: Vec<u8>,
    fuel: usize,
    arithmetic: Budget,
}
impl Writer {
    fn put(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_BYTES - self.bytes.len() {
            return Err(failure("byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn byte(&mut self, byte: u8) -> Result<(), String> {
        self.put(&[byte])
    }
    fn number(&mut self, value: u64) -> Result<(), String> {
        self.put(&value.to_le_bytes())
    }
    fn text(&mut self, text: &str) -> Result<(), String> {
        self.number(text.len() as u64)?;
        self.put(text.as_bytes())
    }
    fn id(&mut self, id: DeclarationId) -> Result<(), String> {
        self.number(id.package)?;
        self.number(id.local as u64)
    }
    fn terms(&mut self, terms: &[T], depth: usize) -> Result<(), String> {
        self.number(terms.len() as u64)?;
        for term in terms {
            self.term(term, depth)?;
        }
        Ok(())
    }
    fn fields(&mut self, fields: &[(String, T)], depth: usize) -> Result<(), String> {
        self.number(fields.len() as u64)?;
        for (name, term) in fields {
            self.text(name)?;
            self.term(term, depth)?;
        }
        Ok(())
    }
    fn optional(&mut self, value: Option<&T>, depth: usize) -> Result<(), String> {
        self.byte(u8::from(value.is_some()))?;
        if let Some(value) = value {
            self.term(value, depth)?;
        }
        Ok(())
    }
    fn term(&mut self, term: &T, depth: usize) -> Result<(), String> {
        tick(&mut self.fuel, depth)?;
        let d = depth + 1;
        match term {
            T::Primitive(p) => {
                self.byte(0)?;
                if let P::BitVec(width) = p {
                    self.byte(255)?;
                    self.number(*width as u64)?;
                } else {
                    self.byte(
                        PRIMITIVES
                            .iter()
                            .position(|other| other == p)
                            .ok_or_else(|| failure("unsupported primitive"))?
                            as u8,
                    )?;
                }
            }
            T::Unit => self.byte(1)?,
            T::Bool(value) => {
                self.byte(2)?;
                self.byte(u8::from(*value))?;
            }
            T::Char(value) => {
                self.byte(3)?;
                self.number(*value as u64)?;
            }
            T::String(value) => {
                self.byte(4)?;
                self.text(value)?;
            }
            T::Symbol(value) => {
                self.byte(5)?;
                self.text(value)?;
            }
            T::Integer(value) => {
                self.byte(6)?;
                let text = value
                    .to_hex_literal(&mut self.arithmetic)
                    .map_err(failure)?;
                self.text(&text)?;
            }
            T::Float32(value) => {
                self.byte(7)?;
                self.number(*value as u64)?;
            }
            T::Float64(value) => {
                self.byte(8)?;
                self.number(*value)?;
            }
            T::BitVec(value) => {
                self.byte(9)?;
                self.number(value.width() as u64)?;
                self.number(value.words().len() as u64)?;
                for word in value.words() {
                    self.number(*word as u64)?;
                }
            }
            T::Float(value) => {
                self.byte(10)?;
                let text = value.to_literal(&mut self.arithmetic).map_err(failure)?;
                self.text(&text)?;
            }
            T::Nominal {
                declaration,
                arguments,
            } => {
                self.byte(11)?;
                self.id(*declaration)?;
                self.terms(arguments, d)?;
            }
            T::NormalForm {
                head,
                sort,
                arguments,
            } => {
                self.byte(12)?;
                self.id(*head)?;
                self.byte(sort_tag(*sort))?;
                self.terms(arguments, d)?;
            }
            T::Optional(value) => {
                self.byte(13)?;
                self.term(value, d)?;
            }
            T::Pointer(value) => {
                self.byte(14)?;
                self.term(value, d)?;
            }
            T::Tuple(values) => {
                self.byte(15)?;
                self.terms(values, d)?;
            }
            T::Record(fields) => {
                self.byte(16)?;
                self.fields(fields, d)?;
            }
            T::TupleValue(values) => {
                self.byte(17)?;
                self.terms(values, d)?;
            }
            T::RecordValue(fields) => {
                self.byte(18)?;
                self.fields(fields, d)?;
            }
            T::TypeListValue(values) => {
                self.byte(19)?;
                self.terms(values, d)?;
            }
            T::NominalValue {
                ty,
                variant,
                fields,
            } => {
                self.byte(20)?;
                self.term(ty, d)?;
                self.byte(u8::from(variant.is_some()))?;
                if let Some(v) = variant {
                    self.number(*v as u64)?;
                }
                self.terms(fields, d)?;
            }
            T::OptionalValue { ty, value } => {
                self.byte(21)?;
                self.term(ty, d)?;
                self.optional(value.as_deref(), d)?;
            }
            T::ErrorResult { errors, result } => {
                self.byte(22)?;
                self.terms(errors, d)?;
                self.term(result, d)?;
            }
            T::ErrorValue { ty, label, value } => {
                self.byte(23)?;
                self.term(ty, d)?;
                self.optional(label.as_deref(), d)?;
                self.term(value, d)?;
            }
            T::ComptimeFunction(value) => {
                self.byte(24)?;
                self.term(value, d)?;
            }
            T::UnsafeFunction(value) => {
                self.byte(25)?;
                self.term(value, d)?;
            }
            T::Function { parameters, result } => {
                self.byte(26)?;
                self.number(parameters.len() as u64)?;
                for parameter in parameters {
                    self.byte(sort_tag(parameter.sort))?;
                    self.term(&parameter.ty, d)?;
                }
                self.term(result, d)?;
            }
            T::Bound { depth, index, sort } => {
                self.byte(27)?;
                self.number(*depth as u64)?;
                self.number(*index as u64)?;
                self.byte(sort_tag(*sort))?;
            }
            T::Closure {
                environment,
                signature,
            } => {
                self.byte(28)?;
                self.terms(environment, d)?;
                self.term(signature, d)?;
            }
            T::ClosureContextType => self.byte(29)?,
            T::ClosureContextValue(values) => {
                self.byte(30)?;
                self.terms(values, d)?;
            }
            T::ContextualClosure { context, signature } => {
                self.byte(31)?;
                self.term(context, d)?;
                self.term(signature, d)?;
            }
            T::TypeListConcat(values) => {
                self.byte(32)?;
                self.terms(values, d)?;
            }
            T::Computation { effects, result } => {
                self.byte(33)?;
                self.number(effects.len() as u64)?;
                for operation in effects {
                    self.id(operation.declaration)?;
                    self.terms(&operation.arguments, d)?;
                }
                self.term(result, d)?;
            }
            T::AssociatedType {
                origin,
                arguments,
                subject,
            } => {
                self.byte(34)?;
                self.id(origin.interface)?;
                self.id(origin.member)?;
                self.terms(arguments, d)?;
                self.term(subject, d)?;
            }
            _ => {
                return Err(failure(
                    "descriptor is not supported by metadata wire v1; executable and store-backed values are not transportable",
                ));
            }
        }
        Ok(())
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    fuel: usize,
    arithmetic: Budget,
    admission_fuel: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| failure("truncated payload"))?;
        let bytes = &self.bytes[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn number(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn narrow(&mut self) -> Result<u32, String> {
        self.number()?
            .try_into()
            .map_err(|_| failure("u32 overflow"))
    }
    fn index(&mut self) -> Result<usize, String> {
        self.number()?
            .try_into()
            .map_err(|_| failure("index overflow"))
    }
    fn count(&mut self) -> Result<usize, String> {
        let n = self.index()?;
        if n > self.bytes.len() - self.at || n > self.fuel {
            return Err(failure("invalid collection length"));
        }
        self.fuel -= n;
        Ok(n)
    }
    fn text(&mut self) -> Result<&'a str, String> {
        let n = self.index()?;
        std::str::from_utf8(self.take(n)?).map_err(|_| failure("invalid UTF-8"))
    }
    fn flag(&mut self) -> Result<bool, String> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(failure("invalid flag")),
        }
    }
    fn sort(&mut self) -> Result<Sort, String> {
        match self.byte()? {
            0 => Ok(Sort::Type),
            1 => Ok(Sort::ComptimeValue),
            2 => Ok(Sort::Runtime),
            _ => Err(failure("invalid sort")),
        }
    }
    fn id(&mut self) -> Result<DeclarationId, String> {
        Ok(DeclarationId {
            package: self.number()?,
            local: self.narrow()?,
        })
    }
    fn terms(&mut self, d: usize) -> Result<Vec<T>, String> {
        let count = self.count()?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push(self.term(d)?);
        }
        Ok(values)
    }
    fn fields(&mut self, d: usize) -> Result<Vec<(String, T)>, String> {
        let count = self.count()?;
        let mut fields = Vec::new();
        for _ in 0..count {
            fields.push((self.text()?.into(), self.term(d)?));
        }
        Ok(fields)
    }
    fn optional(&mut self, d: usize) -> Result<Option<Box<T>>, String> {
        Ok(if self.flag()? {
            Some(Box::new(self.term(d)?))
        } else {
            None
        })
    }
    fn term(&mut self, depth: usize) -> Result<T, String> {
        tick(&mut self.fuel, depth)?;
        let d = depth + 1;
        Ok(match self.byte()? {
            0 => {
                let tag = self.byte()?;
                T::Primitive(if tag == 255 {
                    P::BitVec(self.index()?)
                } else {
                    *PRIMITIVES
                        .get(tag as usize)
                        .ok_or_else(|| failure("unknown primitive"))?
                })
            }
            1 => T::Unit,
            2 => T::Bool(self.flag()?),
            3 => {
                T::Char(char::from_u32(self.narrow()?).ok_or_else(|| failure("invalid character"))?)
            }
            4 => T::String(self.text()?.into()),
            5 => T::Symbol(self.text()?.into()),
            6 => {
                let text = self.text()?;
                let (negative, magnitude) = text
                    .strip_prefix('-')
                    .map_or((false, text), |value| (true, value));
                let value =
                    Integer::parse_literal(magnitude, &mut self.arithmetic).map_err(failure)?;
                T::Integer(if negative {
                    value.negate(&mut self.arithmetic).map_err(failure)?
                } else {
                    value
                })
            }
            7 => T::Float32(self.narrow()?),
            8 => T::Float64(self.number()?),
            9 => {
                let width = self.index()?;
                let count = self.count()?;
                let mut words = Vec::new();
                for _ in 0..count {
                    words.push(self.narrow()?);
                }
                T::BitVec(
                    comptime_runtime::bitvec::BitVec::from_words(
                        width,
                        &words,
                        &mut self.arithmetic,
                    )
                    .map_err(failure)?,
                )
            }
            10 => {
                let text = self.text()?;
                T::Float(
                    comptime_runtime::decimal::DecimalValue::parse_literal(
                        text,
                        &mut self.arithmetic,
                    )
                    .map_err(failure)?,
                )
            }
            11 => T::Nominal {
                declaration: self.id()?,
                arguments: self.terms(d)?,
            },
            12 => T::NormalForm {
                head: self.id()?,
                sort: self.sort()?,
                arguments: self.terms(d)?,
            },
            13 => T::Optional(Box::new(self.term(d)?)),
            14 => T::Pointer(Box::new(self.term(d)?)),
            15 => T::Tuple(self.terms(d)?),
            16 => T::Record(self.fields(d)?),
            17 => T::TupleValue(self.terms(d)?),
            18 => T::RecordValue(self.fields(d)?),
            19 => T::TypeListValue(self.terms(d)?),
            20 => {
                let ty = Box::new(self.term(d)?);
                let variant = if self.flag()? {
                    Some(self.narrow()?)
                } else {
                    None
                };
                T::NominalValue {
                    ty,
                    variant,
                    fields: self.terms(d)?,
                }
            }
            21 => T::OptionalValue {
                ty: Box::new(self.term(d)?),
                value: self.optional(d)?,
            },
            22 => T::ErrorResult {
                errors: self.terms(d)?,
                result: Box::new(self.term(d)?),
            },
            23 => T::ErrorValue {
                ty: Box::new(self.term(d)?),
                label: self.optional(d)?,
                value: Box::new(self.term(d)?),
            },
            24 => T::ComptimeFunction(Box::new(self.term(d)?)),
            25 => T::UnsafeFunction(Box::new(self.term(d)?)),
            26 => {
                let count = self.count()?;
                let mut parameters = Vec::new();
                for _ in 0..count {
                    parameters.push(Parameter {
                        sort: self.sort()?,
                        ty: self.term(d)?,
                    });
                }
                T::Function {
                    parameters,
                    result: Box::new(self.term(d)?),
                }
            }
            27 => T::Bound {
                depth: self.narrow()?,
                index: self.index()?,
                sort: self.sort()?,
            },
            28 => T::Closure {
                environment: self.terms(d)?,
                signature: Box::new(self.term(d)?),
            },
            29 => T::ClosureContextType,
            30 => T::ClosureContextValue(self.terms(d)?),
            31 => T::ContextualClosure {
                context: Box::new(self.term(d)?),
                signature: Box::new(self.term(d)?),
            },
            32 => T::TypeListConcat(self.terms(d)?),
            33 => {
                let count = self.count()?;
                let mut effects = Vec::new();
                for _ in 0..count {
                    effects.push(nir::effects::Operation {
                        declaration: self.id()?,
                        arguments: self.terms(d)?,
                    });
                }
                T::Computation {
                    effects,
                    result: Box::new(self.term(d)?),
                }
            }
            34 => T::AssociatedType {
                origin: Box::new(nir::AssociatedTypeOrigin {
                    interface: self.id()?,
                    member: self.id()?,
                }),
                arguments: self.terms(d)?,
                subject: Box::new(self.term(d)?),
            },
            _ => return Err(failure("unknown descriptor tag")),
        })
    }
}
