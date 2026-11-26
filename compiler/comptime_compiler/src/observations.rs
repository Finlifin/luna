//! Typed metadata/text observations with admission before operand evaluation.
use super::*;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    NewtypeRepresentation,
    DefinitionAttributes,
    EnumName,
    StructFields { names: bool },
    RecordFields { names: bool },
    CallableReflection,
    TypeId,
    EnumStructure(nir::program::EnumStructureQuery),
    SymbolName,
    Text(nir::program::TextQueryKind),
    TypeLayout(nir::TypeLayoutQuery),
    ImplicitQuery(nir::program::ImplicitQueryKind),
    StructFieldSpans,
    RecordFieldSpans,
    DeclarationSpan,
    EnumVariantSpans,
    EnumVariantFieldSpans,
    ComptimeEnumVariantIndex,
    ComptimeEnumVariantFieldValues,
    TupleLen,
    TupleElem,
    BoundedStackBytes,
    CompileError,
    EmitDiagnostic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldOp {
    TypeHasField,
    TypeFieldType,
    StructFieldType,
    FieldValue,
    ComptimeFieldEntry,
    NamedField(std::sync::Arc<str>),
    ConstructStruct,
    ConstructEnum,
    TupleAppend,
}

impl fmt::Display for FieldOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeHasField => f.write_str("type-has-field"),
            Self::TypeFieldType => f.write_str("type-field-type"),
            Self::StructFieldType => f.write_str("struct-field-type"),
            Self::FieldValue => f.write_str("field-value"),
            Self::ComptimeFieldEntry => f.write_str("comptime-field-entry"),
            Self::NamedField(name) => write!(f, "named-field {name:?}"),
            Self::ConstructStruct => f.write_str("construct-struct"),
            Self::ConstructEnum => f.write_str("construct-enum"),
            Self::TupleAppend => f.write_str("tuple-append"),
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StructFields { names } => {
                write!(f, "struct-field-{}", if *names { "names" } else { "types" })
            }
            Self::RecordFields { names } => {
                write!(f, "record-field-{}", if *names { "names" } else { "types" })
            }
            Self::CallableReflection => write!(f, "reflect-callable"),
            Self::TypeId => write!(f, "type-id"),
            Self::EnumStructure(query) => write!(f, "(enum-structure {query})"),
            Self::NewtypeRepresentation => write!(f, "newtype-representation"),
            Self::DefinitionAttributes => write!(f, "definition-attributes"),
            Self::EnumName => write!(f, "enum-name"),
            Self::SymbolName => write!(f, "symbol-name"),
            Self::Text(query) => write!(f, "(text {query})"),
            Self::TypeLayout(query) => write!(f, "(type-layout {query})"),
            Self::ImplicitQuery(query) => write!(f, "(implicit-query {query})"),
            Self::StructFieldSpans => f.write_str("struct-field-spans"),
            Self::RecordFieldSpans => f.write_str("record-field-spans"),
            Self::DeclarationSpan => f.write_str("declaration-span"),
            Self::EnumVariantSpans => f.write_str("enum-variant-spans"),
            Self::EnumVariantFieldSpans => f.write_str("enum-variant-field-spans"),
            Self::ComptimeEnumVariantIndex => f.write_str("comptime-enum-variant-index"),
            Self::ComptimeEnumVariantFieldValues => {
                f.write_str("comptime-enum-variant-field-values")
            }
            Self::TupleLen => write!(f, "tuple-len"),
            Self::TupleElem => write!(f, "tuple-elem"),
            Self::BoundedStackBytes => write!(f, "bounded-stack-bytes"),
            Self::CompileError => write!(f, "compile-error"),
            Self::EmitDiagnostic => write!(f, "emit-diagnostic"),
        }
    }
}

#[derive(Debug)]
pub struct Observation<T = Term> {
    pub kind: Kind,
    pub input: T,
    pub output: T,
    pub operand_source: SourceRange,
}

impl<T: fmt::Display> fmt::Display for Observation<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(observation {} (input {}) (output {}) (operand-source {} {}))",
            self.kind, self.input, self.output, self.operand_source.start, self.operand_source.end
        )
    }
}
