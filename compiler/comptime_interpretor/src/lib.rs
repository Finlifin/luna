//! Concrete execution of the supported NIR value/control-flow fragment.
//!
//! The caller must authorize phase and settle semantic/resource obligations.
//! This engine is not a stage checker and is not automatically invoked by the
//! frontend. `for_comptime` additionally checks the supported function-phase
//! boundary; it does not replace resource verification. No external
//! IO or normal-form unfolding are implemented. Direct handlers execute through
//! NIR and validated IIR occurrence environments; scheduled effect cleanup is
//! not yet admitted. Limits are resource outcomes, not language type errors.
mod admission;
mod aggregate_values;
mod bytecode;
mod cleanup;
mod closure_identity;
mod code_values;

#[cfg(test)]
mod execution_test_support;
mod representation;

pub use representation::ImmutableSize;

mod literal_values;
mod type_descriptions;
mod value_types;

pub use closure_identity::ClosureIdentity;

mod scoped_restore;

pub use code_values::CodeValue;

mod bit_operations;
mod bit_representation;
mod bit_writeback;
mod callable_domains;
mod comptime_domains;
mod comptime_graph;
mod constructor_premises;
mod default_code;
mod dynamic_instances;
mod environment;
mod field_access;
mod function_inputs;
mod function_instances;

pub use function_inputs::{FunctionInputRejection, InputAdmissionStage};

mod iir_execution;

use iir_execution::ConcreteNominal;

mod expression_invocation;
mod host_budget;

pub use host_budget::checked as with_host_selection_budget;

mod call_operands;
mod immutable_storage;
mod input_bodies;
mod input_collection;
mod input_dependencies;
mod input_domains;
mod invocation;

pub use invocation::definition_host::{
    DefinitionHostRequest, DefinitionHostResponse, RejectedDefinitionResponse,
};
pub use invocation::host::{
    AssociatedHostRequest, AssociatedHostResponse, DeliveredOperand, InvocationProgress,
    InvocationResult, InvocationSession, OperandCall, OperandProgress, RejectedHostResponse,
};
pub use invocation::method_host::{MethodHostRequest, MethodHostResponse, RejectedMethodResponse};
pub use invocation::{AdmittedCall, SourceCallPrefix, SourceOperandScope, SourceOperandStep};

mod declaration_inputs;
mod declaration_plan;
mod deterministic;
mod invocation_contracts;
mod list_append;
mod list_index;
mod list_operations;
mod memory;
mod native_depth;
mod normal_inputs;
mod request_identity;
mod retained_contracts;

use normal_inputs::InputKey;

mod input_bindings;
mod normal_plan;
mod normal_terms;
mod normal_types;
mod normal_values;
mod object_edits;
mod object_queries;

#[cfg(test)]
mod object_values;
mod objects;
mod scalar_math;
mod staged_bytecode;
mod trait_methods;
mod value_reference;

pub use objects::ObjectProperty;
pub use value_reference::ValueReference;

mod definition_installation;
mod source_values;

pub use source_values::{AttributeSource, NominalBatch, SourcedValue};

mod generated_newtypes;

pub use generated_newtypes::NewtypePublicationError;

mod generated_enums;

pub use generated_enums::EnumPublicationError;

mod generated_structs;

pub use generated_structs::{
    DefinitionAdmissionSession, PreparedEnumAdmission, PreparedNewtypeAdmission,
    PreparedStructAdmission, StructAdmissionSession, StructPublicationError,
};

mod definition_diagnostics;

pub use definition_diagnostics::DefinitionBuildError;

mod enum_build;

pub use enum_build::{EnumBuildRequest, PreparedEnumBuild};

mod definition_build;

pub use definition_build::{
    DefinitionBuildRequest, DefinitionBuildService, PreparedDefinitionBuild,
};

mod newtype_build;

pub use newtype_build::{NewtypeBuildRequest, PreparedNewtypeBuild};

mod struct_build;

pub use struct_build::{PreparedStructBuild, StructBuildRequest};

mod reflection;

pub use reflection::{
    CallableReflection, EnumReflection, NewtypeReflection, ReflectedField, ReflectedVariant,
    StructReflection,
};

mod float_operations;
mod leaf_inputs;
mod scalar_operations;

#[cfg(test)]
mod scoped_work;
mod selected_types;

pub use comptime_runtime::object::{Entry as ObjectEntry, Object};
pub use memory::{OwnedGraphCheckpoint, PointerValue, Storage};

use deterministic::{DeterministicMap, DeterministicSet};
use nir::program::InterfaceInstance;
use nir::program::{Expr, ExprKind as K, Literal, LocalId, Package, SourceRange};
use nir::{DeclarationId, Primitive, Term};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use environment::Environment;

// Invocation-local contextual interpretation choices; no active state is stored
// in the owned NIR or reused across calls.
enum PreparedNormalInput {
    Value(Value),
    ListType(Term),
}

/// Invocation-only input transport. Live graphs retain their storage identity;
/// normal terms are reconstructed in the invocation's checked environment.
/// This transport is neither a type identity nor a serializable IR node.
pub enum InvocationValue {
    Normal(Term),
    Live(Value),
}

pub use normal_terms::validate_snapshot_term;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Completed contextual code, retained only in invocation-local storage.
    /// It has no ordinary Term data domain or normal-term export.
    Code(CodeValue),
    /// One optional layer. An absent outer value is distinct from a present absent payload.
    Optional {
        ty: Term,
        value: Option<Box<Value>>,
    },
    /// Tagged result data, with no continuation. None is the success branch.
    ErrorResult {
        ty: Term,
        error: Option<usize>,
        value: Box<Value>,
    },
    Pointer(PointerValue),
    /// Interpreter-internal addressable local storage; never a source value.
    Storage(Storage),
    /// Invocation-only comptime alias, including a checked structural field path.
    ComptimeReference(ValueReference),
    /// Concrete or scoped type identity.
    Type(Term),
    /// Completed opaque non-Type normal-form application result, retaining its
    /// comptime-value term and declared output domain.
    NormalForm {
        term: Term,
        ty: Term,
    },
    /// Completed gradual Type data without a concrete type identity.
    Description(nir::program::DescriptionLiteral),
    /// Opaque declaration Type input, admitted only by a comptime scope.
    TypeParameter(DeclarationId),
    /// Invocation-scoped opaque data, never a completed runtime value.
    AbstractValue {
        declaration: DeclarationId,
        ty: Term,
    },
    /// Immutable homogeneous comptime data with its original list type identity.
    List {
        ty: Term,
        values: ListValues,
    },
    Integer(comptime_runtime::Integer),
    BitVec(comptime_runtime::bitvec::BitVec),
    Float(comptime_runtime::decimal::DecimalValue),
    Float16(u16),
    BFloat16(u16),
    Float32(u32),
    Float64(u64),
    SignedInteger(i128),
    UnsignedInteger(u128),
    Unit,
    Variant {
        arguments: Vec<Term>,
        declaration: DeclarationId,
        index: usize,
        fields: Vec<Value>,
    },
    Bool(bool),
    Char(char),
    String(Arc<str>),
    Symbol(Arc<str>),
    Tuple(Vec<Value>),
    /// A named structural product; labels survive host calls and storage paths.
    AnonymousRecord(Vec<(String, Value)>),
    /// Invocation-local ordered metadata, distinct from an anonymous record.
    /// No normal-term encoding or runtime representation is admitted yet.
    Object(Object<ObjectValue>),
    Record {
        arguments: Vec<Term>,
        declaration: DeclarationId,
        fields: Vec<Value>,
    },
    Newtype {
        ty: Term,
        value: Box<Value>,
    },
    Function(DeclarationId),
    /// A named instance identity without an executable body. Its declaration
    /// supplies phase and invocation authority; transporting it grants neither.
    FunctionInstance {
        declaration: DeclarationId,
        arguments: Vec<Term>,
    },
    /// Executable instance admitted by one VM over an immutable graph.
    IirCallable(Arc<IirCallable>),
    RuntimeTraitMethod(Box<TraitMethodIdentity>),
    Closure(Arc<Closure>),
}

/// Preserve the original entry domain, including opaque normal-form identity.
/// Constructing this host representation does not establish its validity.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectValue {
    pub ty: Term,
    pub value: Value,
}

impl fmt::Display for ObjectValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(object-value (type {}) (value {}))",
            self.ty, self.value
        )
    }
}

pub use nir::TraitMethodIdentity;

impl Value {
    /// An ordinary binding can carry this Type, but a structural pattern
    /// cannot select a branch from its representation.
    fn lacks_pattern_shape(&self) -> bool {
        matches!(
            self,
            Self::Description(_) | Self::TypeParameter(_) | Self::Type(Term::Rigid { .. })
        )
    }

    /// Named identity carried by data or a retained concrete callable. This
    /// observation grants no phase, formation or executable-body authority.
    pub fn declaration_instance(&self) -> Option<(DeclarationId, &[Term])> {
        match self {
            Self::FunctionInstance {
                declaration,
                arguments,
            } => Some((*declaration, arguments)),
            Self::IirCallable(handle) => Some((handle.key.declaration, &handle.key.arguments)),
            _ => None,
        }
    }

    /// Inspect the owned invocation graph without copying storage or executing
    /// callbacks. Shared closures and storage are visited once. Weak pointers
    /// are leaves: inspecting a value does not dereference borrowed memory.
    pub fn visit_owned_graph(
        &self,
        fuel: &mut u64,
        visitor: &mut impl FnMut(&Value, &mut u64) -> Result<(), ErrorKind>,
    ) -> Result<(), ErrorKind> {
        fn visit(
            value: &Value,
            fuel: &mut u64,
            depth: usize,
            closures: &mut BTreeSet<usize>,
            storage: &mut BTreeSet<usize>,
            visitor: &mut impl FnMut(&Value, &mut u64) -> Result<(), ErrorKind>,
        ) -> Result<(), ErrorKind> {
            *fuel = fuel.checked_sub(1).ok_or(ErrorKind::StepsExhausted)?;
            if depth >= 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            match value {
                Value::Closure(closure) if !closures.insert(Arc::as_ptr(closure) as usize) => {
                    return Ok(());
                }
                Value::Storage(cell) if !storage.insert(Arc::as_ptr(&cell.0) as usize) => {
                    return Ok(());
                }
                _ => {}
            }
            visitor(value, fuel)?;
            match value {
                Value::ComptimeReference(reference) => {
                    *fuel = fuel
                        .checked_sub(reference.metadata_work() as u64)
                        .ok_or(ErrorKind::StepsExhausted)?;
                    visit(
                        &Value::Storage(reference.storage.clone()),
                        fuel,
                        depth + 1,
                        closures,
                        storage,
                        visitor,
                    )?;
                }
                Value::Storage(cell) => {
                    let payload = cell.0.try_lock().map_err(|_| ErrorKind::InvalidPointer)?;
                    visit(&payload, fuel, depth + 1, closures, storage, visitor)?;
                }
                Value::Closure(closure) => {
                    for child in closure.environment.values() {
                        visit(child, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::List { values: fields, .. } => {
                    for child in fields {
                        visit(child, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::Tuple(fields)
                | Value::Record { fields, .. }
                | Value::Variant { fields, .. } => {
                    for child in fields {
                        visit(child, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::AnonymousRecord(fields) => {
                    for (_, child) in fields {
                        visit(child, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::Object(object) => {
                    for entry in object.entries() {
                        visit(
                            &entry.value().value,
                            fuel,
                            depth + 1,
                            closures,
                            storage,
                            visitor,
                        )?;
                    }
                }
                Value::Optional { value, .. } => {
                    if let Some(value) = value {
                        visit(value, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::ErrorResult { value, .. } | Value::Newtype { value, .. } => {
                    visit(value, fuel, depth + 1, closures, storage, visitor)?
                }
                _ => {}
            }
            Ok(())
        }
        visit(
            self,
            fuel,
            0,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            visitor,
        )
    }

    /// Freeze supported immutable scalars and aggregates. Nominal values retain
    /// their declared type and shape; reconstruction checks declaration layouts.
    /// Known-head metadata and source formation contracts are checked separately.
    pub fn into_normal_term(
        self,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<Term, ErrorKind> {
        self.into_scoped_normal_term(budget, &BTreeSet::new())
    }

    /// Freeze an invocation input, retaining named callable declaration identities
    /// inside immutable aggregates. Captured environments remain live values;
    /// this path never serializes their storage or execution state.
    pub fn into_scoped_invocation_term(
        self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
    ) -> Result<Term, ErrorKind> {
        static EMPTY_DOMAINS: std::sync::OnceLock<BTreeMap<nir::RuntimeId, Term>> =
            std::sync::OnceLock::new();
        self.into_scoped_term(
            budget,
            parameters,
            &BTreeMap::new(),
            EMPTY_DOMAINS.get_or_init(BTreeMap::new),
            true,
        )
    }

    /// Validate immutable callable declaration inputs for capture snapshots.
    /// This checks data shape only; the receiving VM still checks invocation authority.
    pub fn validate_callable_identity_snapshot(
        &self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
    ) -> Result<(), ErrorKind> {
        match self {
            Self::RuntimeTraitMethod(identity) => {
                budget.consume(1).map_err(|_| ErrorKind::StepsExhausted)?;
                for term in identity.terms() {
                    validate_snapshot_term(term, budget, parameters)?;
                }
            }
            _ => {
                let (_, arguments) = self.declaration_instance().ok_or(ErrorKind::Type)?;
                budget.consume(1).map_err(|_| ErrorKind::StepsExhausted)?;
                for term in arguments {
                    validate_snapshot_term(term, budget, parameters)?;
                }
            }
        }
        Ok(())
    }

    /// Freeze a substitution value within an explicitly supplied Type scope.
    /// Immutable export must use `into_normal_term`, which remains closed.
    pub fn into_scoped_normal_term(
        self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
    ) -> Result<Term, ErrorKind> {
        static EMPTY_DOMAINS: std::sync::OnceLock<BTreeMap<nir::RuntimeId, Term>> =
            std::sync::OnceLock::new();
        self.into_scoped_term(
            budget,
            parameters,
            &BTreeMap::new(),
            EMPTY_DOMAINS.get_or_init(BTreeMap::new),
            false,
        )
    }

    /// Reify a residual substitution under explicit declaration-domain authority.
    /// This is not a completed constant or cache-key export.
    pub fn into_abstract_invocation_term(
        self,
        budget: &mut comptime_runtime::Budget,
        type_parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
    ) -> Result<Term, ErrorKind> {
        static EMPTY_DOMAINS: std::sync::OnceLock<BTreeMap<nir::RuntimeId, Term>> =
            std::sync::OnceLock::new();
        self.into_scoped_term(
            budget,
            type_parameters,
            value_parameters,
            EMPTY_DOMAINS.get_or_init(BTreeMap::new),
            true,
        )
    }
}

/// A graph position is meaningful only in the admitting VM. Frozen data
/// carries the declaration and arguments, never the position or local handle.
#[derive(Debug, PartialEq)]
pub struct IirCallable {
    position: usize,
    key: iir::InstanceKey,
    ty: Term,
}

impl IirCallable {
    pub fn key(&self) -> &iir::InstanceKey {
        &self.key
    }

    pub fn ty(&self) -> &Term {
        &self.ty
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Closure {
    definition_source: Option<SourceRange>,
    parameters: Vec<LocalId>,
    environment: Environment,
    capture_order: Vec<LocalId>,
    body: Arc<Expr>,
    ty: Term,
}

impl Closure {
    pub fn parameters(&self) -> &[LocalId] {
        &self.parameters
    }

    pub fn body(&self) -> &Arc<Expr> {
        &self.body
    }

    /// The checked callable type, including capture domains and phase.
    pub fn ty(&self) -> &Term {
        &self.ty
    }

    /// Capture slots in semantic field order, independent of caller locals.
    /// Slots may be Storage cells: constant materializers must snapshot their
    /// current values rather than persist these interpreter handles.
    pub fn captured_values(&self) -> impl ExactSizeIterator<Item = (LocalId, &Value)> {
        self.capture_order
            .iter()
            .map(|id| (*id, &self.environment[id]))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub steps: u64,
    /// Maximum active invocations. Native evaluation and recursive value
    /// inspection additionally obey a conservative host-stack depth bound.
    pub depth: usize,
}

impl Budget {
    /// Default VM action allowance, shared with the public comptime command.
    pub const DEFAULT_STEPS: u64 = 50_000_000;
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            steps: Self::DEFAULT_STEPS,
            depth: 128,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    FunctionInputRejected(Box<FunctionInputRejection>),
    InvalidDefinition(DefinitionBuildError),
    BuildMetadataUnavailable {
        owner: u64,
        provider: Option<u64>,
        alias: String,
    },
    DivisionByZero,
    IndexOutOfBounds,
    MachineOverflow,
    PhaseMismatch,
    InvalidPhase(nir::phases::Error),
    InvalidBitWidth,
    FloatLiteralOverflow(Primitive),
    EnumVariant(nir::program::UnitVariantError),
    ProgramContractRejected,
    TypeContractRejected,
    TypeContractUnresolved,
    InvalidTypeContract(String),
    AbstractValueUnresolved,
    PendingDefinitionAttributes(DeclarationId),
    StepsExhausted,
    RepresentationSizeLimit,
    DepthExhausted,
    UnknownFunction,
    MissingBinding,
    InvalidCallPlan,
    ExpiredPointer,
    InvalidPointer,
    InvalidValueReference,
    UnmaterializedDispatch,
    AssociatedSourceFailure,
    MissingAssociatedSource(Box<nir::associated_initializers::AssociatedSourceDemand>),
    Arity,
    Type,
    Unsupported,
    InvalidControlFlow,
    CyclicAssociatedInitializer(DeclarationId),
    NonExhaustiveMatch,
    InvalidEffectEnvironment,
    EffectControlTransfer,
    EffectValueConsumed,
}

impl From<comptime_runtime::Error> for ErrorKind {
    fn from(cause: comptime_runtime::Error) -> Self {
        use comptime_runtime::Error as E;
        match cause {
            E::BudgetExhausted => Self::StepsExhausted,
            E::SizeLimit => Self::RepresentationSizeLimit,
            E::DivisionByZero => Self::DivisionByZero,
            E::InvalidLiteral | E::OutOfRange => Self::Type,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub source: Option<SourceRange>,
}

impl Error {
    fn phase(error: nir::phases::Error) -> Self {
        Self {
            source: Some(error.source),
            kind: match error.kind {
                nir::phases::ErrorKind::Budget => ErrorKind::StepsExhausted,
                nir::phases::ErrorKind::Depth => ErrorKind::DepthExhausted,
                nir::phases::ErrorKind::Invalid => ErrorKind::InvalidPhase(error),
            },
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::MissingAssociatedSource(demand) => {
                write!(f, "missing declaration source: {demand}")?
            }
            ErrorKind::PendingDefinitionAttributes(target) => write!(
                f,
                "definition attributes for {target} are pending publication"
            )?,
            ErrorKind::CyclicAssociatedInitializer(member) => {
                write!(f, "cyclic associated initializer for member {member}")?
            }
            ErrorKind::InvalidDefinition(reason) => write!(f, "interpretation {reason}")?,
            ErrorKind::FloatLiteralOverflow(domain) => write!(
                f,
                "finite float literal exceeds the selected {domain} representation"
            )?,
            ErrorKind::EnumVariant(reason) => write!(f, "{reason}")?,
            ErrorKind::InvalidTypeContract(reason) => write!(f, "invalid type contract: {reason}")?,
            ErrorKind::InvalidPhase(reason) => {
                write!(f, "interpretation phase: {}", reason.message)?
            }
            ErrorKind::FunctionInputRejected(reason) => write!(f, "interpretation {reason}")?,
            _ => write!(f, "interpretation {:?}", self.kind)?,
        }
        if self.kind == ErrorKind::StepsExhausted {
            f.write_str(" (step budget exhausted)")?;
        }
        if let Some(source) = self.source {
            write!(f, " at {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    CheckPredicate {
        source: SourceRange,
        passed: bool,
    },
    InstantiateFunction {
        declaration: DeclarationId,
        arguments: Vec<Term>,
    },
    EnterFunction(DeclarationId),
    CreateClosure {
        source: SourceRange,
        captures: usize,
    },
}

#[derive(Clone, Debug)]
enum Flow {
    Value(Value),
    Return(Value),
    Break(LocalId),
    Continue(LocalId),
    EffectResume(u64, Value),
}

enum SelectedBitRange {
    Range(usize, usize),
    Control(Flow),
}

// Prepared actual values are owned here; the cleanup frame remains installed
// across preparation and invocation. Preparation temporaries do not occupy
// every recursive caller's host stack while the callee executes.
struct InterfaceExit {
    conditions: Vec<Expr>,
    result_binding: Option<LocalId>,
    environment: Environment,
    result_slot: usize,
}

enum PreparedCall {
    Invoke {
        interface_exit: Option<Box<InterfaceExit>>,
        callee: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    },
    Control(Flow),
}

macro_rules! value {
    ($result:expr) => {
        match $result? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        }
    };
}

mod associated_catalog;
mod associated_dispatch;
mod associated_execution;
mod code_session;

use code_session::CodeOrigin;

mod contextual_code;
mod declaration_index;
mod layout_catalog;
mod list_values;
mod source_resolution;
mod source_state;

pub use list_values::{ListIntoIter, ListIter, ListValues};
use source_resolution::SourceDispatch;
pub use source_resolution::{BudgetedTypeSolver, SourceResolvers};

mod method_catalog;

pub use code_session::CodeSession;

mod invocation_budget;
mod observations;

mod effects;

/// Result from the compiler's type-proposition solver. Unknown never authorizes
/// construction. Program-state/resource specifications use a separate system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractOutcome {
    Satisfied,
    Rejected,
    Unknown,
}

/// A failed query produced no judgment about the proposition.
pub type ContractQueryResult = Result<ContractOutcome, ErrorKind>;

/// Compiler-owned selection service for the same immutable declaration world.
/// Selection must discharge implementation and member formation obligations;
/// the VM additionally validates inputs, phase and public signature before use.
pub type MethodResolver<'a> =
    dyn Fn(&Expr, &mut u64) -> Result<nir::program::MethodInstance, Error> + 'a;

pub use nir::associated_initializers::AssociatedSelection;

/// Resolve against the same frozen declaration world; never execute the program.
pub type AssociatedResolver<'a> =
    dyn Fn(&Expr, &mut u64) -> Result<AssociatedSelection, Error> + 'a;

/// Form a pending attribute Object in the owning source checker, while the VM
/// retains its current operand stack. An empty merged Object has no payload.
pub type AttributeResolver<'a> =
    dyn Fn(DeclarationId, &mut u64) -> Result<Option<Arc<Term>>, Error> + 'a;

enum TypeSolver<'a> {
    Unmetered(&'a dyn Fn(&nir::program::TypePredicate) -> ContractOutcome),
    Budgeted(&'a BudgetedTypeSolver<'a>),
    /// Observes projected operands in the same declaration world as dispatch.
    Declarations(&'a BudgetedTypeSolver<'a>),
}

enum PointerCopyPermission<'a> {
    Checked(&'a dyn Fn(&Term) -> bool),
    Budgeted(&'a dyn Fn(&Term, &mut u64) -> bool),
}

pub struct Interpreter<'a> {
    bytecode_invocations: usize,
    bytecode_default_evaluations: usize,
    bytecode_staged_inputs: usize,
    bytecode_contextual_inputs: usize,
    bytecode_input_preparations: usize,
    bytecode_compilations: usize,
    bytecode_compilation_steps: u64,
    compilation_session: comptime_compiler::CompilationSession,
    bytecode_cache: bytecode::CodeCache,
    borrowed_code_cache: bytecode::BorrowedCodeCache<'a>,
    last_named_signature_work: Option<(&'a Term, u64)>,
    input_body_cache: input_bodies::Cache,
    default_code_cache: default_code::Cache,
    contextual_code_cache: contextual_code::Cache,
    prepared_instance_cache: function_instances::Cache,
    definition_service: Option<&'a dyn DefinitionBuildService>,
    newtype_builder: Option<
        &'a dyn Fn(&NewtypeBuildRequest, &mut u64) -> Result<PreparedNewtypeBuild<'a>, Error>,
    >,
    enum_builder:
        Option<&'a dyn Fn(&EnumBuildRequest, &mut u64) -> Result<PreparedEnumBuild<'a>, Error>>,
    struct_builder:
        Option<&'a dyn Fn(&StructBuildRequest, &mut u64) -> Result<PreparedStructBuild<'a>, Error>>,
    declaration_world: Option<&'a Package>,
    phase_inspection: Option<nir::phases::Inspection<'a>>,
    reserved_source_declarations: Option<Arc<declaration_index::ReservedDeclarations>>,
    target: Option<nir::target::TargetData>,
    effect_declarations: declaration_index::Effects<'a>,
    effect_state: effects::State,
    effect_values: Option<&'a iir::effect_values::Plan<'a>>,
    graph: Option<&'a iir::Package>,
    cleanup_schedule: Option<&'a iir::cleanup::Schedule>,
    cleanup_scopes: DeterministicMap<&'a iir::InstanceKey, Vec<&'a iir::cleanup::FunctionSchedule>>,
    // Borrowed from the validated, immutable schedule. Preserve all sites at
    // each source position, including distinct contextual instances.
    cleanup_sites:
        BTreeMap<(u32, u32), Vec<(&'a iir::cleanup::FunctionSchedule, &'a iir::cleanup::Site)>>,
    cleanup_effect_context: DeterministicMap<nir::effects::Operation, bool>,
    cleanup_frame: Option<iir::InstanceKey>,
    cleanup_failed: bool,
    prepared_frames: Vec<cleanup::PreparedFrame>,
    residual_temporaries: Vec<(SourceRange, memory::Storage)>,
    pattern_binding_orders: BTreeMap<(u32, u32), Vec<LocalId>>,
    cleanup_context: Vec<iir::cleanup::ContextKey>,
    comptime_only: bool,
    bytecode_enabled: bool,
    abstract_type_parameters: BTreeSet<DeclarationId>,
    abstract_value_parameters: BTreeMap<DeclarationId, Term>,
    type_solver: Option<TypeSolver<'a>>,
    method_resolver: Option<&'a MethodResolver<'a>>,
    associated_resolver: Option<&'a AssociatedResolver<'a>>,
    attribute_resolver: Option<source_values::Resolver<'a>>,
    pointer_copy: Option<PointerCopyPermission<'a>>,
    declaration_parameter_domains: Arc<BTreeMap<DeclarationId, Term>>,
    method_parameters: Arc<BTreeMap<(DeclarationId, DeclarationId), Vec<LocalId>>>,
    // None records an ambiguous member identity across different interfaces.
    method_origins: Arc<BTreeMap<DeclarationId, Option<DeclarationId>>>,
    method_contracts: method_catalog::Contracts<'a>,
    trait_declarations: declaration_index::Traits<'a>,
    implemented_interfaces: Arc<DeterministicMap<(InterfaceInstance, Term), DeclarationId>>,
    assumed_interfaces: DeterministicSet<(InterfaceInstance, Term)>,
    method_bodies:
        Arc<DeterministicMap<(InterfaceInstance, Term, DeclarationId), (DeclarationId, Vec<Term>)>>,
    resolved_method_bodies:
        DeterministicMap<(InterfaceInstance, Term, DeclarationId), (DeclarationId, Vec<Term>)>,
    associated_values: associated_catalog::Catalog<'a>,
    associated_initializers: associated_catalog::Catalog<'a>,
    active_associated_initializers: DeterministicSet<(InterfaceInstance, Term, DeclarationId)>,
    // Source specializations retain code, while IIR handles carry VM-local identity.
    instances: function_instances::Instances<Expr>,
    iir_callables: BTreeMap<usize, Arc<IirCallable>>,
    functions: nir::callable_source::Callables<'a>,

    graph_indexes: Option<Arc<iir_execution::GraphIndexes>>,
    normal_forms: declaration_index::NormalForms<'a>,
    structs: layout_catalog::Catalog<generated_structs::StructDefinition<'a>>,
    newtypes: declaration_index::Newtypes<'a>,
    alias_contracts: declaration_index::Aliases<'a>,
    enums: layout_catalog::Catalog<generated_enums::EnumDefinition<'a>>,
    budget: Budget,
    depth: usize,
    invocation_depth: usize,
    invocation_depth_limit: usize,
    events: Vec<Event>,
}

impl<'a> Interpreter<'a> {
    pub(crate) fn runtime_domains(&self) -> &'a BTreeMap<nir::RuntimeId, Term> {
        static EMPTY: std::sync::OnceLock<BTreeMap<nir::RuntimeId, Term>> =
            std::sync::OnceLock::new();
        self.declaration_world.map_or_else(
            || EMPTY.get_or_init(BTreeMap::new),
            |package| &package.runtime_domains,
        )
    }

    pub fn target(&self) -> Option<nir::target::TargetData> {
        self.target
    }

    fn integer_layout(&self, primitive: Primitive) -> Option<(bool, u32)> {
        match self.target {
            Some(target) => target.integer_layout(primitive),
            None => primitive.fixed_integer_layout(),
        }
    }

    pub fn new(package: &'a Package, budget: Budget) -> Result<Self, Error> {
        Self::with_assumptions(package, &[], budget)
    }

    fn with_assumptions(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        budget: Budget,
    ) -> Result<Self, Error> {
        Self::with_assumptions_and_index(
            package,
            assumptions,
            budget,
            None,
            None,
            None,
            SourceDispatch::Closed,
        )
    }

    fn with_assumptions_and_index(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        mut budget: Budget,
        positions: Option<Arc<declaration_index::SourceIndexes>>,
        scope: Option<source_state::InvocationScope<'_>>,
        inspection: Option<&nir::phases::Inspection<'_>>,
        dispatch: SourceDispatch<'a>,
    ) -> Result<Self, Error> {
        let closed_dispatch = matches!(dispatch, SourceDispatch::Closed);
        if closed_dispatch && !package.method_links.is_empty() {
            return Err(Error {
                kind: ErrorKind::InvalidCallPlan,
                source: None,
            });
        }
        // Only sessions without caller premises publish complete world indexes.
        // A caller with extra premises still needs its own implication checks.
        let positions = positions.filter(|index| {
            closed_dispatch && scope.is_none() && std::ptr::eq(package, index.source.as_ref())
        });
        if assumptions.is_empty() {
            if let Some(index) = &positions {
                return Self::from_source_indexes(package, budget, index.clone());
            }
        }
        let dispatch_instances = dispatch.instances(package);
        invocation_budget::charge_world_tables(package, dispatch_instances.len(), &mut budget)?;
        // Phase inspection already prepared these structural facts. Reuse
        // them only in the exact source owner; this carries no caller premises
        // or permission to skip body, scope or method admission.
        let admitted_domains =
            inspection.and_then(|inspection| inspection.admitted_parameter_domains(package));
        let directory = inspection
            .or_else(|| positions.as_ref().map(|index| &index.phase_inspection))
            .and_then(|inspection| inspection.callables_for(package));
        let functions = match directory {
            Some(directory) => directory,
            None => {
                let before = usize::try_from(budget.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let result = nir::callable_source::Callables::new(package, &mut fuel);
                budget.steps -= (before - fuel) as u64;
                result.map_err(|failure| Error {
                    kind: match failure.kind {
                        nir::phases::ErrorKind::Budget => ErrorKind::StepsExhausted,
                        nir::phases::ErrorKind::Depth => ErrorKind::DepthExhausted,
                        nir::phases::ErrorKind::Invalid => ErrorKind::InvalidCallPlan,
                    },
                    source: Some(failure.source),
                })?
            }
        };
        let effect_declarations = declaration_index::Effects::new(
            &package.effects,
            positions.as_ref().map(|index| index.effects.clone()),
            |effect| effect.declaration,
        )?;
        for effect in &package.effects {
            if !matches!(effect.signature, Term::Function { .. }) {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        let mut structs = positions
            .as_ref()
            .map(|index| layout_catalog::Catalog::from_base(index.structs.clone()))
            .unwrap_or_default();
        let mut enums = positions
            .as_ref()
            .map(|index| layout_catalog::Catalog::from_base(index.enums.clone()))
            .unwrap_or_default();
        for definition in &package.enums {
            if definition.validate_encoding_in(package.target).is_err() {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
            if positions.is_none()
                && enums
                    .insert(
                        definition.declaration,
                        generated_enums::EnumDefinition::Borrowed(definition),
                    )
                    .is_some()
            {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        for definition in &package.structs {
            if positions.is_none()
                && structs
                    .insert(
                        definition.declaration,
                        generated_structs::StructDefinition::Borrowed(definition),
                    )
                    .is_some()
            {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        let invalid = || Error {
            kind: ErrorKind::InvalidCallPlan,
            source: None,
        };
        // Matching source indexes certify structural metadata for this exact
        // immutable snapshot. Caller premises still receive their own checks
        // below, but cannot invalidate or replace this structural evidence.
        if positions.is_none() && admitted_domains.is_none() {
            invocation_budget::validate_opaque_metadata(package, &mut budget)?;
        }
        let normal_forms = declaration_index::NormalForms::new(
            &package.normal_forms,
            positions.as_ref().map(|index| index.normal_forms.clone()),
            |head| head.declaration,
        )?;
        let traits = declaration_index::Traits::new(
            &package.traits,
            positions.as_ref().map(|index| index.traits.clone()),
            |interface| interface.declaration,
        )?;
        for interface in &package.traits {
            let mut names = BTreeSet::new();
            let mut members = BTreeSet::new();
            for member in &interface.methods {
                if !matches!(
                    member.signature.ordinary(),
                    Ok(Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_))
                ) {
                    return Err(invalid());
                }
                if !names.insert(&member.name) || !members.insert(member.declaration) {
                    return Err(invalid());
                }
                if member.default.is_some_and(|body| {
                    functions
                        .method_slot(&body)
                        .is_none_or(|(_, quote, _)| quote != member.is_quote)
                }) {
                    return Err(invalid());
                }
            }
        }
        let initial_trait_fuel = usize::try_from(budget.steps).unwrap_or(usize::MAX);
        let mut trait_parameter_fuel = initial_trait_fuel;
        let declaration_parameter_domains = if let Some(index) = &positions {
            index.parameter_domains.clone()
        } else if let Some(domains) = admitted_domains {
            domains
        } else {
            Arc::new(
                package
                    .declaration_parameter_domains_checked(&mut trait_parameter_fuel)
                    .map_err(constructor_premises::failure)?,
            )
        };
        let assumed_interfaces =
            constructor_premises::expand(assumptions, &traits, &mut trait_parameter_fuel)?;
        let mut method_bodies = DeterministicMap::new();
        for interface in &package.traits {
            if interface
                .parents
                .iter()
                .any(|parent| !traits.contains_key(&parent.declaration))
            {
                return Err(invalid());
            }
        }
        // Explicit instances contain demanded members only. Index them before
        // considering legacy concrete rows, so an omitted conditional method
        // never enters the executable directory.
        let mut explicit = DeterministicMap::new();
        for instance in dispatch_instances {
            let key = (instance.interface_instance(), instance.subject.clone());
            if explicit.insert(key, instance.implementation).is_some() {
                return Err(invalid());
            }
        }
        let mut worlds = DeterministicMap::new();
        let mut implementations = BTreeMap::new();
        for implementation in &package.implementations {
            if !implementation.phases.runtime && !implementation.phases.comptime {
                return Err(invalid());
            }
            if implementations
                .insert(implementation.declaration, implementation)
                .is_some()
            {
                return Err(invalid());
            }
            let concrete =
                implementation.type_parameters.is_empty() && implementation.requirements.is_empty();
            let Some(interface) = implementation.interface_instance() else {
                if implementation.phases.runtime && implementation.phases.comptime {
                    return Err(invalid());
                }
                continue;
            };
            let contract = traits.get(&interface.declaration).ok_or_else(invalid)?;
            if interface.arguments.len() != contract.type_parameters.len() {
                return Err(invalid());
            }
            if concrete {
                contract
                    .validate_arguments_with_layouts(
                        &implementation.subject,
                        &interface.arguments,
                        &[],
                        &package.traits,
                        &BTreeMap::new(),
                        &mut trait_parameter_fuel,
                        |_, _, _| Ok(false),
                        Some(package),
                    )
                    .map_err(|error| match error {
                        nir::NominalAdmissionError::Budget => Error {
                            kind: ErrorKind::StepsExhausted,
                            source: None,
                        },
                        nir::NominalAdmissionError::Unresolved(_) => Error {
                            kind: ErrorKind::TypeContractUnresolved,
                            source: None,
                        },
                        nir::NominalAdmissionError::Invalid(_) => invalid(),
                    })?;
            }
            let implicit_methods = if concrete && closed_dispatch {
                let key = (interface.clone(), implementation.subject.clone());
                let implicit = !explicit.contains_key(&key);
                if worlds.insert(key, implementation.declaration).is_some() {
                    return Err(invalid());
                }
                implicit
            } else {
                false
            };
            let mut supplied = BTreeMap::new();
            for (name, body) in &implementation.methods {
                if supplied.insert(name, body).is_some() || functions.method_slot(body).is_none() {
                    return Err(invalid());
                }
            }
            for member in &contract.methods {
                if !implementation.phases.contains(member.phase) {
                    if supplied.contains_key(&member.name) {
                        return Err(invalid());
                    }
                    continue;
                }
                let body = **supplied.get(&member.name).ok_or_else(invalid)?;
                let (phase, quote, type_parameters) =
                    functions.method_slot(&body).ok_or_else(invalid)?;
                if member.is_quote != quote {
                    return Err(invalid());
                }
                if phase == nir::program::FunctionPhase::NormalForm
                    || (phase == nir::program::FunctionPhase::Comptime)
                        != member.signature.is_comptime_callable()
                {
                    return Err(invalid());
                }
                let expected_parameters = if member.default == Some(body) {
                    contract.parameters_with_self()
                } else {
                    implementation.type_parameters.clone()
                };
                if type_parameters.len() != expected_parameters.len() + member.type_parameters.len()
                {
                    return Err(invalid());
                }
                if !type_parameters.starts_with(&expected_parameters)
                    && !nir::function_parameters::equivalent_prefix_domains(
                        &expected_parameters,
                        &type_parameters[..expected_parameters.len()],
                        &declaration_parameter_domains,
                        &mut trait_parameter_fuel,
                    )
                    .map_err(|_| Error {
                        kind: ErrorKind::StepsExhausted,
                        source: None,
                    })?
                {
                    return Err(invalid());
                }
                if implicit_methods && !member.is_quote {
                    let arguments = if member.default == Some(body) {
                        std::iter::once(implementation.subject.clone())
                            .chain(interface.arguments.iter().cloned())
                            .collect()
                    } else {
                        vec![]
                    };
                    method_bodies.insert(
                        (
                            interface.clone(),
                            implementation.subject.clone(),
                            member.declaration,
                        ),
                        (body, arguments),
                    );
                }
            }
        }
        let mut associated_values = associated_catalog::Catalog::source(package);
        let mut associated_initializers = associated_catalog::Catalog::source(package);
        for (dispatch, instance) in dispatch_instances.iter().enumerate() {
            for (position, (member, _)) in instance.associated.iter().enumerate() {
                if associated_values
                    .insert_at(
                        (
                            instance.interface_instance(),
                            instance.subject.clone(),
                            *member,
                        ),
                        associated_catalog::Position {
                            dispatch,
                            member: position,
                            initializer: false,
                        },
                    )
                    .is_some()
                {
                    return Err(invalid());
                }
            }
        }

        for (dispatch, instance) in dispatch_instances.iter().enumerate() {
            for (position, (member, _)) in instance.associated_initializers.iter().enumerate() {
                let key = (
                    instance.interface_instance(),
                    instance.subject.clone(),
                    *member,
                );
                if associated_values.contains_key(&key)
                    || associated_initializers
                        .insert_at(
                            key,
                            associated_catalog::Position {
                                dispatch,
                                member: position,
                                initializer: true,
                            },
                        )
                        .is_some()
                {
                    return Err(invalid());
                }
            }
        }

        // Index every selection before validating obligations: a required row
        // may follow its consumer. The directory stays private until all
        // selections, members and obligations have been admitted.
        for (key, selected) in explicit {
            if worlds
                .insert(key, selected)
                .is_some_and(|old| old != selected)
            {
                return Err(invalid());
            }
        }
        for instance in dispatch_instances {
            let implementation = implementations
                .get(&instance.implementation)
                .copied()
                .ok_or_else(invalid)?;
            let implementation_inputs = nir::DeclarationSubstitution::bounded(
                &implementation.type_parameters,
                &implementation.parameter_types,
                &instance.arguments,
                &[],
                &mut trait_parameter_fuel,
            )
            .map_err(constructor_premises::failure)?;
            let interface = implementation.interface_instance().ok_or_else(invalid)?;
            if implementation_inputs
                .instantiate_interface_bounded(&interface, &mut trait_parameter_fuel)
                .map_err(constructor_premises::failure)?
                != instance.interface_instance()
                || implementation_inputs
                    .instantiate_bounded(&implementation.subject, &mut trait_parameter_fuel)
                    .map_err(constructor_premises::failure)?
                    != instance.subject
            {
                return Err(invalid());
            }
            let contract = traits.get(&instance.interface).ok_or_else(invalid)?;
            associated_dispatch::validate(
                instance,
                implementation,
                &implementation_inputs,
                contract,
                &associated_values,
                package,
                &|id| normal_forms.get(&id),
                &mut trait_parameter_fuel,
            )?;

            let mut method_keys = DeterministicSet::new();
            for method in &instance.methods {
                let member = contract
                    .methods
                    .iter()
                    .find(|m| m.declaration == method.member)
                    .ok_or_else(invalid)?;
                if !implementation.phases.contains(member.phase) {
                    return Err(invalid());
                }
                if member.type_parameters.len() != method.method_arguments.len()
                    || !method_keys.insert((method.member, method.method_arguments.clone()))
                {
                    return Err(invalid());
                }
                let expected_body = implementation
                    .methods
                    .iter()
                    .find(|(name, _)| name == &member.name)
                    .ok_or_else(invalid)?
                    .1;
                let prefix_arguments: Vec<_> = if member.default == Some(expected_body) {
                    std::iter::once(instance.subject.clone())
                        .chain(instance.interface_arguments.iter().cloned())
                        .collect()
                } else {
                    instance.arguments.clone()
                };
                if method.body != expected_body
                    || method
                        .arguments
                        .iter()
                        .ne(prefix_arguments.iter().chain(&method.method_arguments))
                {
                    return Err(invalid());
                }
                if member.is_quote || functions.quote(&method.body).is_some() {
                    return Err(invalid());
                }
                let formation = functions.formation(&method.body).ok_or_else(invalid)?;
                let domains = function_inputs::input_domains(
                    &declaration_parameter_domains,
                    formation.parameters,
                    &mut trait_parameter_fuel,
                )
                .map_err(constructor_premises::failure)?;
                let body_inputs = nir::DeclarationSubstitution::bounded_domains(
                    formation.parameters,
                    &domains,
                    &method.arguments,
                    nir::DeclarationInputScope::Caller(&[]),
                    &mut trait_parameter_fuel,
                )
                .map_err(constructor_premises::failure)?;
                let ty = formation.qualify(
                    body_inputs
                        .instantiate_bounded(formation.signature, &mut trait_parameter_fuel)
                        .map_err(constructor_premises::failure)?,
                );
                let required = nir::member_types::method(
                    contract,
                    member,
                    &instance.subject,
                    &instance.interface_arguments,
                    &method.method_arguments,
                    |id, _| Ok(declaration_parameter_domains.get(&id)),
                    &[],
                    &mut trait_parameter_fuel,
                )
                .map_err(constructor_premises::failure)?;
                if !selected_types::same(
                    &ty,
                    required.ordinary().map_err(|_| invalid())?,
                    &associated_values,
                    &mut trait_parameter_fuel,
                )
                .map_err(|kind| Error { kind, source: None })?
                {
                    return Err(invalid());
                }
                let key = (
                    instance.interface_instance(),
                    instance.subject.clone(),
                    member.declaration,
                );
                let value = (method.body, prefix_arguments);
                if method_bodies.get(&key).is_some_and(|old| old != &value) {
                    return Err(invalid());
                }
                method_bodies.insert(key, value);
            }
            for capability in &implementation.capabilities {
                let capability = implementation_inputs
                    .instantiate_requirement_bounded(capability, &mut trait_parameter_fuel)
                    .map_err(constructor_premises::failure)?;
                if !worlds
                    .contains_key(&(capability.interface_instance(), capability.subject.clone()))
                    && !assumed_interfaces
                        .contains(&(capability.interface_instance(), capability.subject))
                {
                    return Err(invalid());
                }
            }
        }
        // Resolver-backed source execution checks parent/capability closure at
        // demand. The legacy table path still requires a fully closed table.
        for (interface, subject) in worlds
            .keys()
            .chain(assumed_interfaces.iter())
            .filter(|_| !matches!(dispatch, SourceDispatch::Resolving(_)))
        {
            let contract = traits
                .get(&interface.declaration)
                .expect("checked interface");
            let arguments = nir::member_types::arguments(
                subject,
                &interface.arguments,
                &[],
                &mut trait_parameter_fuel,
            )
            .map_err(constructor_premises::failure)?;
            let inputs = contract
                .input_substitution(&arguments, &[], &mut trait_parameter_fuel)
                .map_err(constructor_premises::failure)?;
            for capability in &contract.capabilities {
                let capability = inputs
                    .instantiate_requirement_bounded(capability, &mut trait_parameter_fuel)
                    .map_err(constructor_premises::failure)?;
                if !worlds
                    .contains_key(&(capability.interface_instance(), capability.subject.clone()))
                    && !assumed_interfaces
                        .contains(&(capability.interface_instance(), capability.subject))
                {
                    return Err(invalid());
                }
            }
            for parent in &contract.parents {
                let parent = inputs
                    .instantiate_interface_bounded(parent, &mut trait_parameter_fuel)
                    .map_err(constructor_premises::failure)?;
                if !worlds.contains_key(&(parent.clone(), subject.clone()))
                    && !assumed_interfaces.contains(&(parent, subject.clone()))
                {
                    return Err(invalid());
                }
            }
        }
        let method_parameters = if let Some(index) = &positions {
            index.method_parameters.clone()
        } else {
            let method_parameters: BTreeMap<_, _> = package
                .traits
                .iter()
                .flat_map(|t| {
                    t.methods.iter().map(move |m| {
                        ((t.declaration, m.declaration), m.parameter_bindings.clone())
                    })
                })
                .collect();
            Arc::new(method_parameters)
        };
        let mut method_contracts = BTreeMap::new();
        let mut method_origins = BTreeMap::new();
        if positions.is_none() {
            for (interface_index, contract) in package.traits.iter().enumerate() {
                let parameters = contract
                    .binders_with_self_checked(&mut trait_parameter_fuel)
                    .map_err(constructor_premises::failure)?;
                for (member_index, member) in contract.methods.iter().enumerate() {
                    method_origins
                        .entry(member.declaration)
                        .and_modify(|origin| *origin = None)
                        .or_insert(Some(contract.declaration));
                    let ids = parameters
                        .iter()
                        .map(|p| p.declaration)
                        .chain(member.type_parameters.iter().copied())
                        .collect();
                    method_contracts.insert(
                        (contract.declaration, member.declaration),
                        method_catalog::Entry {
                            parameters: ids,
                            interface: interface_index,
                            member: member_index,
                        },
                    );
                }
            }
        }
        let method_origins = positions
            .as_ref()
            .map(|index| index.method_origins.clone())
            .unwrap_or_else(|| Arc::new(method_origins));
        let method_contracts = method_catalog::Contracts {
            traits: &package.traits,
            entries: positions
                .as_ref()
                .map(|index| index.method_contracts.clone())
                .unwrap_or_else(|| Arc::new(method_contracts)),
        };
        let remaining_steps = budget.steps - (initial_trait_fuel - trait_parameter_fuel) as u64;
        let mut interpreter = Self::from_source_state(
            package,
            Budget {
                steps: remaining_steps,
                depth: budget.depth,
            },
            positions,
            source_state::Prepared {
                normal_forms,
                trait_declarations: traits,
                effect_declarations,
                method_parameters,
                declaration_parameter_domains,
                method_contracts,
                method_origins,
                implemented_interfaces: Arc::new(worlds),
                assumed_interfaces,
                method_bodies: Arc::new(method_bodies),
                associated_values,
                associated_initializers,
                functions,
                structs,
                enums,
            },
        )?;
        if let Some(scope) = scope {
            // Only phase-checked private comptime construction supplies scope.
            // No VM or reusable source index is published before admission.
            interpreter.comptime_only = true;
            interpreter.authorize_abstract_type_parameters(scope.types)?;
            interpreter.authorize_abstract_value_parameters(scope.values)?;
        }
        interpreter.admit_dispatch_inputs(dispatch_instances, &implementations)?;
        if let SourceDispatch::Resolving(resolvers) = dispatch {
            interpreter.type_solver = Some(TypeSolver::Declarations(resolvers.types));
            interpreter.method_resolver = Some(resolvers.methods);
            interpreter.associated_resolver = Some(resolvers.associated);
        }
        Ok(interpreter)
    }

    /// A phase-checked entrypoint for concrete first-order comptime functions.
    /// Resource/effect verification remains the caller's separate obligation.
    pub fn for_comptime(package: &'a Package, budget: Budget) -> Result<Self, Error> {
        Self::for_comptime_with_assumptions(package, &[], budget)
    }

    /// Phase-checked reference execution with bytecode routing disabled.
    /// Keeps comptime value-reference semantics and admission checks intact,
    /// allowing differential checks without substituting the runtime universe.
    pub fn for_comptime_tree(package: &'a Package, budget: Budget) -> Result<Self, Error> {
        Self::for_comptime(package, budget).map(Self::with_reference_execution)
    }

    /// Select reference evaluation after constructing the appropriate world.
    /// This changes execution routing only: phase, scope, source services and
    /// admission remain those established by the constructor. It is intended
    /// for differential checks, not recovery from bytecode compilation errors.
    pub fn with_reference_execution(mut self) -> Self {
        self.bytecode_enabled = false;
        self
    }

    /// Evaluate under caller-established trait premises. Premises discharge
    /// obligations only; they provide neither method bodies nor associated values.
    /// They belong to this interpreter and never mutate the source package.
    pub fn for_comptime_with_assumptions(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        budget: Budget,
    ) -> Result<Self, Error> {
        Self::for_comptime_with_index(package, assumptions, budget, None)
    }

    fn for_comptime_with_index(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        budget: Budget,
        positions: Option<Arc<declaration_index::SourceIndexes>>,
    ) -> Result<Self, Error> {
        Self::for_comptime_with_index_and_scope(package, assumptions, budget, positions, None, None)
    }

    fn for_comptime_with_index_and_scope(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        mut budget: Budget,
        positions: Option<Arc<declaration_index::SourceIndexes>>,
        scope: Option<source_state::InvocationScope<'_>>,
        phase: Option<&nir::phases::PhaseCheckedPackage<'a>>,
    ) -> Result<Self, Error> {
        // Index publication certifies successful phase validation of this exact
        // owned snapshot. Never transfer that evidence by declaration identity
        // or structural equality. Caller assumptions are not phase evidence.
        let positions = positions
            .filter(|index| scope.is_none() && std::ptr::eq(package, index.source.as_ref()));
        let inspection = match &positions {
            Some(index) => index.phase_inspection.clone(),
            None => invocation_budget::phase_inspection(package, phase, &mut budget)?,
        };
        let mut interpreter = Self::with_assumptions_and_index(
            package,
            assumptions,
            budget,
            positions,
            scope,
            Some(&inspection),
            SourceDispatch::Closed,
        )?;
        interpreter.phase_inspection = Some(inspection);
        interpreter.comptime_only = true;
        Ok(interpreter)
    }

    /// Prepare a comptime world using a caller-owned construction allowance.
    /// Success reports the remaining steps. Failure (including unwinding) keeps
    /// the allowance reserved: a failed constructor cannot report a trustworthy
    /// remainder. This is conservative reservation, not exact failed-work timing.
    /// Depth configuration is unchanged. After executing the returned VM, callers
    /// must synchronize remaining_steps again; this does not share a live counter.
    pub fn for_comptime_in_budget(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let reserved = *budget;
        budget.steps = 0;
        let interpreter = Self::for_comptime_with_assumptions(package, assumptions, reserved)?;
        budget.steps = interpreter.remaining_steps();
        Ok(interpreter)
    }

    /// Prepare a private open comptime world. Explicit caller scope is installed
    /// before dispatch input admission, and is never published as a shared index.
    /// Construction reserves the caller's allowance on failure, like the closed
    /// entrypoint. These binders remain opaque and do not authorize export.
    pub fn for_comptime_in_scope_in_budget(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        parameters: &[DeclarationId],
        value_parameters: &[(DeclarationId, Term)],
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let reserved = *budget;
        budget.steps = 0;
        let interpreter = Self::for_comptime_with_index_and_scope(
            package,
            assumptions,
            reserved,
            None,
            Some(source_state::InvocationScope {
                types: parameters,
                values: value_parameters,
            }),
            None,
        )?;
        budget.steps = interpreter.remaining_steps();
        Ok(interpreter)
    }

    /// Resolve associated programs in this source VM's frozen world.
    pub fn with_associated_resolver(mut self, resolver: &'a AssociatedResolver<'a>) -> Self {
        self.associated_resolver = Some(resolver);
        self
    }

    pub fn with_attribute_resolver(mut self, resolver: &'a AttributeResolver<'a>) -> Self {
        self.attribute_resolver = Some(source_values::Resolver::Value(resolver));
        self
    }

    pub fn with_attribute_source(mut self, resolver: &'a AttributeSource<'a>) -> Self {
        self.attribute_resolver = Some(source_values::Resolver::Source(resolver));
        self
    }

    /// Resolve invocation-demanded methods in this source VM's frozen world.
    pub fn with_method_resolver(mut self, resolver: &'a MethodResolver<'a>) -> Self {
        self.method_resolver = Some(resolver);
        self
    }

    /// Supply the host's type theory without coupling the interpreter to the
    /// elaborator. The callback receives an instantiated type predicate only.
    pub fn with_type_solver(
        mut self,
        solver: &'a dyn Fn(&nir::program::TypePredicate) -> ContractOutcome,
    ) -> Self {
        self.type_solver = Some(TypeSolver::Unmetered(solver));
        self
    }

    /// Charge host proof work against this invocation, without granting the
    /// callback authority to replenish its execution budget.
    pub fn with_budgeted_type_solver(mut self, solver: &'a BudgetedTypeSolver<'a>) -> Self {
        self.type_solver = Some(TypeSolver::Budgeted(solver));
        self
    }

    fn solve_type_predicate(
        &mut self,
        predicate: &nir::program::TypePredicate,
    ) -> ContractQueryResult {
        // The declaration resolver owns a frozen source world. Supply facts
        // established by completed publications in this invocation as well.
        let local = if matches!(self.type_solver, Some(TypeSolver::Declarations(_))) {
            self.published_kind_predicate(predicate)?
        } else {
            None
        };
        let predicate = local.as_ref().unwrap_or(predicate);
        match &self.type_solver {
            Some(TypeSolver::Unmetered(solver)) => Ok(solver(predicate)),
            Some(TypeSolver::Budgeted(solver) | TypeSolver::Declarations(solver)) => {
                if self.budget.steps == 0 {
                    return Err(ErrorKind::StepsExhausted);
                }
                host_budget::clamped(&mut self.budget.steps, |remaining| {
                    solver(predicate, remaining)
                })
            }
            None => self.retained_type_contract(predicate, 0),
        }
    }

    /// Host-checked Copy delivery, including representation and Drop legality.
    /// Raw interpreters retain intrinsic-only pointer reads and replacement.
    pub fn with_pointer_copy_permission(mut self, permission: &'a dyn Fn(&Term) -> bool) -> Self {
        self.pointer_copy = Some(PointerCopyPermission::Checked(permission));
        self
    }

    /// Acquire resource evidence using this invocation's remaining execution
    /// fuel. Proof work is charged before the transfer or its continuation.
    pub fn with_budgeted_pointer_copy_permission(
        mut self,
        permission: &'a dyn Fn(&Term, &mut u64) -> bool,
    ) -> Self {
        self.pointer_copy = Some(PointerCopyPermission::Budgeted(permission));
        self
    }

    fn permits_pointer_copy(&mut self, ty: &Term) -> bool {
        if nir::program::intrinsic_pointer_value(ty) {
            return true;
        }
        match &self.pointer_copy {
            Some(PointerCopyPermission::Checked(permission)) => permission(ty),
            Some(PointerCopyPermission::Budgeted(permission)) => {
                let mut remaining = self.budget.steps;
                let allowed = permission(ty, &mut remaining);
                self.budget.steps = self.budget.steps.min(remaining);
                allowed
            }
            None => false,
        }
    }

    /// Permit opaque Type inputs belonging to this normalization scope. This
    /// does not grant layout, pattern discrimination or normal-value export.
    pub fn with_abstract_type_parameters(
        mut self,
        parameters: &[DeclarationId],
    ) -> Result<Self, Error> {
        self.authorize_abstract_type_parameters(parameters)?;
        Ok(self)
    }

    /// Extend scope atomically while keeping failed preparation costs visible.
    pub fn authorize_abstract_type_parameters(
        &mut self,
        parameters: &[DeclarationId],
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        self.integer_operation(None, |budget| budget.consume(parameters.len()))?;
        if parameters
            .iter()
            .any(|id| self.abstract_value_parameters.contains_key(id))
        {
            return Err(self.failure(ErrorKind::Type, None));
        }
        self.abstract_type_parameters
            .extend(parameters.iter().copied());
        Ok(())
    }

    /// Grant explicit declaration-domain evidence for opaque comptime data.
    /// These values may be transported but cannot authorize a branch decision
    /// or a completed snapshot. No authority is inferred from an input value.
    pub fn with_abstract_value_parameters(
        mut self,
        parameters: &[(DeclarationId, Term)],
    ) -> Result<Self, Error> {
        self.authorize_abstract_value_parameters(parameters)?;
        Ok(self)
    }

    /// Validate the entire batch before publishing any new value authority.
    /// Errors retain consumed fuel but leave the previous scope unchanged.
    pub fn authorize_abstract_value_parameters(
        &mut self,
        parameters: &[(DeclarationId, Term)],
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        self.integer_operation(None, |budget| budget.consume(parameters.len()))?;
        // A later domain can mention an earlier value in this telescope.
        // Validate against the completed prefix and roll it back as a unit.
        let mut inserted = Vec::new();
        let result = (|| {
            for (declaration, ty) in parameters {
                self.check_type_description(ty, None)?;
                if self.abstract_type_parameters.contains(declaration)
                    || *ty == Term::Primitive(Primitive::Type)
                {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                if let Some(previous) = self.abstract_value_parameters.get(declaration) {
                    if previous != ty {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                } else {
                    self.abstract_value_parameters
                        .insert(*declaration, ty.clone());
                    inserted.push(*declaration);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            for id in inserted {
                self.abstract_value_parameters.remove(&id);
            }
        }
        result
    }

    fn query_normal_value_fields(
        &self,
        ty: &Term,
        variant: Option<u32>,
        fuel: &mut usize,
    ) -> Result<Vec<Term>, nir::NominalAdmissionError> {
        use nir::NominalAdmissionError as AdmissionError;
        use nir::normal_forms::instantiate_value_fields_checked as instantiate_value_fields;
        nir::type_computations::charge_term(ty, fuel)?;
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err("layout requires a nominal type".into());
        };
        if self.graph.is_some() {
            return match (
                self.concrete_nominal(ty)
                    .ok_or_else(|| AdmissionError::from("concrete nominal layout unavailable"))?,
                variant,
            ) {
                (ConcreteNominal::Enum(definition), Some(index)) => instantiate_value_fields(
                    &[],
                    &[],
                    &[],
                    definition
                        .variants
                        .get(index as usize)
                        .ok_or_else(|| AdmissionError::from("enum variant index is unavailable"))?
                        .fields
                        .iter(),
                    fuel,
                ),
                (ConcreteNominal::Newtype(definition), None) => instantiate_value_fields(
                    &[],
                    &[],
                    &[],
                    std::iter::once(&definition.representation),
                    fuel,
                ),
                (ConcreteNominal::Struct(definition), None) => instantiate_value_fields(
                    &[],
                    &[],
                    &[],
                    definition.fields.iter().map(|(_, ty)| ty),
                    fuel,
                ),
                _ => Err("concrete nominal variant kind mismatch".into()),
            };
        }
        if let Some(index) = variant {
            let definition = self
                .enums
                .get(declaration)
                .ok_or_else(|| AdmissionError::from("enum layout unavailable"))?;
            instantiate_value_fields(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                definition
                    .variants
                    .get(index as usize)
                    .ok_or_else(|| AdmissionError::from("enum variant index is unavailable"))?
                    .fields
                    .iter(),
                fuel,
            )
        } else if let Some(definition) = self.newtypes.get(declaration) {
            Ok(vec![
                definition
                    .representation_budgeted(arguments, &[], fuel)
                    .map_err(|error| match error {
                        nir::newtypes::ConversionError::Budget => AdmissionError::Budget,
                        error => AdmissionError::Invalid(error.to_string()),
                    })?,
            ])
        } else {
            let definition = self
                .structs
                .get(declaration)
                .ok_or_else(|| AdmissionError::from("struct layout unavailable"))?;
            instantiate_value_fields(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                definition.fields.iter().map(|(_, ty)| ty),
                fuel,
            )
        }
    }

    fn admits_description(&self, data: &nir::program::DescriptionLiteral) -> bool {
        self.comptime_only
            && self
                .declaration_world
                .is_some_and(|world| world.refinements.contains(data))
    }

    fn type_data(&self, value: Value, source: SourceRange) -> Result<Value, Error> {
        match value {
            Value::Type(_) => Ok(value),
            Value::Description(ref data) if self.admits_description(data) => Ok(value),
            Value::TypeParameter(declaration)
                if self.abstract_type_parameters.contains(&declaration) =>
            {
                Ok(Value::Type(Term::Rigid {
                    declaration,
                    sort: nir::Sort::Type,
                }))
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    fn type_identity_term(&self, value: Value, source: SourceRange) -> Result<Term, Error> {
        match self.type_data(value, source)? {
            Value::Type(ty) => Ok(ty),
            _ => Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source))),
        }
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Account for host-side argument preparation against this invocation's
    /// budget. Hosts can consume fuel here but cannot replenish it.
    pub fn charge_preparation(&mut self, work: usize) -> Result<(), Error> {
        let work =
            u64::try_from(work).map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
        self.budget.steps = self
            .budget
            .steps
            .checked_sub(work)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        Ok(())
    }

    pub fn remaining_steps(&self) -> u64 {
        self.budget.steps
    }

    fn charge_copy_work(
        &mut self,
        source: Option<SourceRange>,
        check: impl FnOnce(&mut usize) -> Result<(), nir::copy_budget::CopyBudgetExhausted>,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = check(&mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|reason| {
            self.failure(
                match reason {
                    nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                    nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                },
                source,
            )
        })
    }

    fn instantiate_activation_code(
        &mut self,
        expression: &Expr,
        substitution: &nir::DeclarationSubstitution<'_>,
    ) -> Result<Expr, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = dynamic_instances::instantiate_code(substitution, expression, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|kind| self.failure(kind, Some(expression.source)))
    }

    fn copy_invocation_condition(
        &mut self,
        condition: &Expr,
        substitution: Option<&nir::DeclarationSubstitution<'_>>,
    ) -> Result<Expr, Error> {
        match substitution {
            Some(inputs) => self.instantiate_activation_code(condition, inputs),
            None => {
                self.charge_copy_work(Some(condition.source), |fuel| {
                    let mut steps = *fuel as u64;
                    let result = nir::copy_budget::charge_expression(condition, &mut steps);
                    *fuel = steps as usize;
                    result
                })?;
                Ok(condition.clone())
            }
        }
    }

    pub fn invoke(
        &mut self,
        function: DeclarationId,
        arguments: Vec<Value>,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.invoke_instance(
                &iir::InstanceKey {
                    declaration: function,
                    arguments: vec![].into(),
                },
                arguments,
            );
        }
        self.invoke_value(Value::Function(function), arguments)
    }

    fn concrete_value<V: std::borrow::Borrow<Value>>(
        value: V,
        source: SourceRange,
    ) -> Result<V, Error> {
        if matches!(value.borrow(), Value::AbstractValue { .. }) {
            Err(Error {
                kind: ErrorKind::AbstractValueUnresolved,
                source: Some(source),
            })
        } else {
            Ok(value)
        }
    }

    fn failure(&self, kind: ErrorKind, source: Option<SourceRange>) -> Error {
        Error { kind, source }
    }

    fn tick(&mut self, source: Option<SourceRange>) -> Result<(), Error> {
        if self.budget.steps == 0 {
            return Err(self.failure(ErrorKind::StepsExhausted, source));
        }
        self.budget.steps -= 1;
        if self.depth >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, source));
        }
        Ok(())
    }

    fn invoke_value(&mut self, function: Value, arguments: Vec<Value>) -> Result<Value, Error> {
        self.invoke_value_scoped(function, arguments, None)
    }

    // Finish binding before recursive body evaluation so its temporary values
    // do not accumulate in every invocation frame on small host stacks.
    #[cfg(test)]
    fn bind_invocation_inputs(
        &mut self,
        parameters: &[LocalId],
        arguments: Vec<Value>,
        signature: &Term,
        environment: &mut Environment,
        source: SourceRange,
        observed: Option<&mut Vec<Term>>,
    ) -> Result<Option<nir::FunctionInputBinding>, Error> {
        self.bind_invocation_inputs_for_body(
            parameters,
            arguments,
            signature,
            environment,
            source,
            observed,
            None,
        )
    }

    #[inline(never)]
    fn bind_invocation_inputs_for_body(
        &mut self,
        parameters: &[LocalId],
        arguments: Vec<Value>,
        signature: &Term,
        environment: &mut Environment,
        source: SourceRange,
        mut observed: Option<&mut Vec<Term>>,
        mut defer: Option<(&BTreeSet<usize>, &mut Vec<(usize, LocalId)>)>,
    ) -> Result<Option<nir::FunctionInputBinding>, Error> {
        let Term::Function {
            parameters: types, ..
        } = signature.callable_signature()
        else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if parameters.len() != arguments.len() || types.len() != arguments.len() {
            return Err(self.failure(ErrorKind::Arity, Some(source)));
        }
        let mut builder: Option<nir::FunctionInputBuilder<'_>> = None;
        let mut needs_binding = false;
        for (index, ((binding, argument), parameter)) in
            parameters.iter().zip(arguments).zip(types).enumerate()
        {
            // Type observation may invoke associated programs. Finish it
            // before borrowing the argument's value-reference storage.
            if builder.is_none() && parameter.sort != nir::Sort::Runtime {
                let mut prefix = nir::FunctionInputBuilder::new(signature)
                    .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                for _ in 0..index {
                    self.query_admission(Some(source), |_, fuel| {
                        prefix.next_type_bounded(fuel)?;
                        Ok(())
                    })?;
                    prefix
                        .push(None)
                        .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                }
                builder = Some(prefix);
            }
            let raw = match &mut builder {
                Some(prefix) => self
                    .query_admission(Some(source), |_, fuel| Ok(prefix.next_type_bounded(fuel)?))?,
                None => &parameter.ty,
            };
            let expected = self.invocation_domain(raw, source)?;
            if self.comptime_only
                && parameter.sort == nir::Sort::Runtime
                && !matches!(argument, Value::Storage(_))
            {
                if let Value::ComptimeReference(reference) = &argument {
                    self.admit_reference_input(reference, &expected, source)?;
                } else {
                    self.check_borrowed_representation(&argument, source, 0)?;
                    self.check_value_type(&argument, &expected, Some(source))?;
                    self.admit_embedded_type_descriptions(&argument, 0)?;
                }
                self.commit_input_domain(
                    builder.as_mut(),
                    observed.as_deref_mut(),
                    None,
                    expected,
                    source,
                )?;
                environment.insert(*binding, argument);
                continue;
            }
            let representation = match &argument {
                Value::Storage(storage) => std::borrow::Cow::Owned(self.read_owned_value(storage)?),
                Value::ComptimeReference(reference) => {
                    std::borrow::Cow::Owned(self.read_value_reference(reference, source)?)
                }
                value if self.comptime_only => {
                    self.check_borrowed_representation(value, source, 0)?;
                    std::borrow::Cow::Borrowed(value)
                }
                value => std::borrow::Cow::Owned(self.checkpoint_representation(value, source)?),
            };
            self.check_value_type(&representation, &expected, Some(source))?;
            if self.comptime_only {
                self.admit_embedded_type_descriptions(&representation, 0)?;
            }
            let input = if parameter.sort != nir::Sort::Runtime
                && matches!(representation, std::borrow::Cow::Borrowed(_))
                && let Some((entry, deferred)) = defer.as_mut()
                && !entry.contains(&index)
            {
                // The activation owns this argument. Materialize an identity
                // only if a body annotation reads this slot. References and
                // storage must instead retain their entry-time observation.
                deferred.push((index, *binding));
                needs_binding = true;
                None
            } else if parameter.sort == nir::Sort::Type
                || (parameter.sort == nir::Sort::ComptimeValue
                    && expected == Term::Primitive(Primitive::Type))
            {
                needs_binding = true;
                self.ordinary_type_identity(&representation, source)?
            } else if parameter.sort == nir::Sort::ComptimeValue
                && self.normal_input_is_data(&representation, source)?
            {
                // Checked data inputs may remain symbolic in a generic
                // invocation. Preserve their authorized declaration identity
                // in dependent inputs, results and body annotations.
                Some(match representation {
                    std::borrow::Cow::Borrowed(value) => {
                        self.borrowed_residual_input_term(value, source)?
                    }
                    std::borrow::Cow::Owned(value) => self.residual_input_term(value, source)?,
                })
            } else {
                None
            };
            needs_binding |= input.is_some();
            self.commit_input_domain(
                builder.as_mut(),
                observed.as_deref_mut(),
                input,
                expected,
                source,
            )?;
            environment.insert(*binding, argument);
        }
        if needs_binding && let Some(builder) = builder {
            self.query_admission(Some(source), |_, fuel| {
                Ok(Some(builder.finish_bounded(fuel)?))
            })
        } else {
            Ok(None)
        }
    }

    /// The domain and representation have been admitted in this invocation.
    /// A description stays live as data without supplying an exact type term.
    fn ordinary_type_identity(
        &mut self,
        value: &Value,
        source: SourceRange,
    ) -> Result<Option<Term>, Error> {
        match value {
            Value::Type(ty) => {
                self.charge_copy_work(Some(source), |fuel| {
                    nir::copy_budget::charge_term_copy(ty, fuel, 128)
                })?;
                Ok(Some(ty.clone()))
            }
            Value::TypeParameter(declaration)
                if self.abstract_type_parameters.contains(declaration) =>
            {
                Ok(Some(Term::Rigid {
                    declaration: *declaration,
                    sort: nir::Sort::Type,
                }))
            }
            Value::Description(_) => Ok(None),
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    fn commit_input_domain(
        &mut self,
        builder: Option<&mut nir::FunctionInputBuilder<'_>>,
        observed: Option<&mut Vec<Term>>,
        input: Option<Term>,
        domain: Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if let Some(builder) = builder {
            if let Some(domains) = observed {
                let copy = self.query_admission(Some(source), |_, fuel| {
                    nir::copy_budget::clone_term(&domain, fuel)
                        .map_err(|_| nir::NominalAdmissionError::Budget)
                })?;
                domains.push(copy);
            }
            builder
                .push_viewed(input, domain)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
        } else if let Some(domains) = observed {
            domains.push(domain);
        }
        Ok(())
    }

    /// Invocation binding may retain an ordinary slot whose value exists but
    /// supplies no exact term. A closed annotation is an identity consumer.
    fn require_closed_input_domain(&self, ty: &Term, source: SourceRange) -> Result<(), Error> {
        ty.check_scope(&[]).map_err(|error| {
            let kind = match error {
                nir::TypeError::InvalidBound { .. } => ErrorKind::TypeContractUnresolved,
                _ => ErrorKind::Type,
            };
            self.failure(kind, Some(source))
        })
    }

    fn specialize_input_types(
        &mut self,
        expression: &mut Expr,
        scope: &nir::FunctionInputBinding,
    ) -> Result<(), Error> {
        let source = expression.source;
        expression.rewrite_types(&mut |ty| {
            self.tick(Some(source))?;
            let ty =
                self.query_admission(Some(source), |_, fuel| Ok(scope.rebind_bounded(ty, fuel)?))?;
            self.require_closed_input_domain(&ty, source)?;
            Ok(ty)
        })
    }

    fn clone_invocation_environment(
        &mut self,
        environment: &Environment,
        source: SourceRange,
    ) -> Result<Environment, Error> {
        self.charge_preparation(environment.len())?;
        environment
            .iter()
            .map(|(id, value)| {
                self.checkpoint_representation(value, source)
                    .map(|value| (*id, value))
            })
            .collect()
    }

    fn local_value(
        &mut self,
        environment: &Environment,
        id: LocalId,
        source: Option<SourceRange>,
    ) -> Result<Value, Error> {
        let value = environment
            .get(&id)
            .ok_or_else(|| self.failure(ErrorKind::MissingBinding, source))?;
        self.invocation_slot_value(value, source)
    }

    fn invocation_slot_value(
        &mut self,
        value: &Value,
        source: Option<SourceRange>,
    ) -> Result<Value, Error> {
        match value {
            Value::ComptimeReference(reference) => self.read_value_reference_in(reference, source),
            Value::Storage(storage) => {
                if self.comptime_only {
                    let reference = value_reference::ValueReference::root(storage.clone());
                    let contents = reference
                        .borrow(&mut self.budget.steps)
                        .map_err(|kind| self.failure(kind, source))?;
                    return self.checkpoint_representation_in(contents.value(), source);
                }
                // Read the representation, retaining shared nested identities.
                // Ownership is checked by the actual access/transfer operation;
                // a projection may legally read one field of a partial owner.
                let value = storage
                    .0
                    .try_lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidPointer, source))?;
                self.checkpoint_representation_in(&value, source)
            }
            value => self.checkpoint_representation_in(value, source),
        }
    }

    fn eval_pointer(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        if self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        if nir::program::pointer_operation_error_in(expr, self.target).is_some() {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let result = match &expr.kind {
            K::AddressOf(place) => {
                let mut base = place.as_ref();
                let mut fields = vec![];
                loop {
                    match &base.kind {
                        K::Field {
                            value,
                            declaration,
                            index,
                        } => {
                            self.tick(Some(expr.source))?;
                            fields.push(memory::Projection::Record(*declaration, *index));
                            base = value;
                        }
                        K::AnonymousField { value, name, index } => {
                            self.tick(Some(expr.source))?;
                            fields.push(memory::Projection::AnonymousRecord(name.clone(), *index));
                            base = value;
                        }
                        _ => break,
                    }
                }
                fields.reverse();
                let K::Local(id) = base.kind else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let value = environment
                    .get_mut(&id)
                    .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(expr.source)))?;
                if !matches!(value, Value::Storage(_)) {
                    *value = Value::Storage(Storage::new(value.clone()));
                }
                let Value::Storage(storage) = value else {
                    unreachable!()
                };
                let pointer = storage.pointer(place.ty.clone(), fields);
                pointer
                    .inspect(|target| {
                        self.check_retained_value(target, &place.ty, Some(expr.source))
                    })
                    .map_err(|kind| self.failure(kind, Some(expr.source)))??;
                Value::Pointer(pointer)
            }
            K::PointerAnonymousField {
                pointer,
                name,
                index,
            } => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Term::Pointer(pointee) = &expr.ty else {
                    unreachable!()
                };
                Value::Pointer(
                    pointer
                        .project_anonymous(name, *index, (**pointee).clone())
                        .map_err(|kind| self.failure(kind, Some(expr.source)))?,
                )
            }
            K::PointerField {
                pointer,
                declaration,
                index,
            } => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Term::Pointer(pointee) = &expr.ty else {
                    unreachable!()
                };
                Value::Pointer(
                    pointer
                        .project(*declaration, *index, (**pointee).clone())
                        .map_err(|kind| self.failure(kind, Some(expr.source)))?,
                )
            }
            K::ReadPointer(pointer) => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                if pointer.pointee != expr.ty || !self.permits_pointer_copy(&expr.ty) {
                    return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
                }
                let value = pointer
                    .inspect(|value| self.copy_representation(value, expr.source, 0))
                    .map_err(|kind| self.failure(kind, Some(expr.source)))??;
                self.check_retained_value(&value, &expr.ty, Some(expr.source))?;
                value
            }
            K::WritePointer { pointer, value } => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let value = value!(self.eval(value, environment));
                if !self.permits_pointer_copy(&pointer.pointee) {
                    return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
                }
                self.check_retained_value(&value, &pointer.pointee, Some(expr.source))?;
                pointer
                    .write(value, |old| {
                        self.check_retained_value(old, &pointer.pointee, Some(expr.source))
                            .map_err(|error| error.kind)
                    })
                    .map_err(|kind| self.failure(kind, Some(expr.source)))?;
                Value::Unit
            }
            _ => unreachable!(),
        };
        Ok(Flow::Value(result))
    }

    fn eval(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.eval_in_position(expr, environment, false)
    }

    fn eval_in_position(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        direct_callee: bool,
    ) -> Result<Flow, Error> {
        self.tick(Some(expr.source))?;
        let result = self.with_native_depth(|vm| {
            if vm.graph.is_some()
                && matches!(
                    expr.kind,
                    K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
                )
            {
                vm.eval_iir_callable(expr)
            } else if direct_callee && matches!(expr.kind, K::TraitMethod { .. }) {
                vm.eval_trait_method(expr, environment, true)
            } else {
                vm.eval_inner(expr, environment)
            }
        });
        self.finish_evaluation(expr, environment, result)
    }

    // Mapped computation evaluation must use the same delivery/control hooks
    // as ordinary evaluation. Transparent effect wrappers share their child's
    // source identity and must not deliver or retire that value a second time.
    fn finish_evaluation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        result: Result<Flow, Error>,
    ) -> Result<Flow, Error> {
        // Most comptime evaluation has no scheduled runtime cleanup. Avoid
        // dispatching both delivery hooks per instruction in that mode; effect
        // transfers still need their scope handler even without a schedule.
        if self.cleanup_schedule.is_none()
            && !matches!(&result, Err(error) if error.kind == ErrorKind::EffectControlTransfer)
        {
            return result;
        }
        let mut result = if matches!(&result,Err(error) if error.kind==ErrorKind::EffectControlTransfer)
            && self.effect_transfer_targets_scope()?
        {
            let flow = self.effect_state.transfer.take().unwrap().1;
            match &flow {
                Flow::Return(value) => {
                    self.check_scheduled_result(value, expr.source, environment)?;
                    self.cleanup_site(
                        expr.source,
                        iir::cleanup::Boundary::EffectReturn,
                        environment,
                    )?;
                }
                Flow::Break(id) => self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::EffectBreak(*id),
                    environment,
                )?,
                Flow::Continue(id) => self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::EffectContinue(*id),
                    environment,
                )?,
                _ => {}
            }
            Ok(flow)
        } else {
            result
        };
        if matches!(&result, Ok(Flow::Value(_)))
            && !matches!(
                expr.kind,
                K::Block { .. }
                    | K::CallPreparation { .. }
                    | K::Return(_)
                    | K::EffectComputation { .. }
                    | K::EffectRun { .. }
            )
        {
            if let Ok(Flow::Value(value)) = &mut result {
                self.cleanup_delivery(expr, environment, value)?;
            }
            if !expr.shares_discard_temporary_boundary() {
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::TemporaryEnd,
                    environment,
                )?;
            }
        }
        result
    }

    fn with_arithmetic_budget<T>(
        &mut self,
        operation: impl FnOnce(&mut comptime_runtime::Budget) -> T,
    ) -> T {
        let steps = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps,
            max_limbs: 4096,
        };
        let result = operation(&mut budget);
        self.budget.steps -= (steps - budget.steps) as u64;
        result
    }

    fn integer_operation<T>(
        &mut self,
        source: Option<SourceRange>,
        operation: impl FnOnce(&mut comptime_runtime::Budget) -> Result<T, comptime_runtime::Error>,
    ) -> Result<T, Error> {
        self.with_arithmetic_budget(operation)
            .map_err(|cause| self.failure(cause.into(), source))
    }

    fn integer_literal(
        &mut self,
        spelling: &str,
        source: Option<SourceRange>,
    ) -> Result<comptime_runtime::Integer, Error> {
        self.integer_operation(source, |budget| {
            let (negative, digits) = spelling
                .strip_prefix('-')
                .map_or((false, spelling), |s| (true, s));
            let value = comptime_runtime::Integer::parse_literal(digits, budget)?;
            if negative {
                budget.consume(value.bit_len().div_ceil(32).saturating_add(1))?;
                Ok(value.negated())
            } else {
                Ok(value)
            }
        })
    }

    fn bit_operation<T>(
        &mut self,
        source: SourceRange,
        operation: impl FnOnce(
            &mut comptime_runtime::Budget,
        ) -> Result<T, comptime_runtime::bitvec::Error>,
    ) -> Result<T, Error> {
        let result = self.with_arithmetic_budget(operation);
        result.map_err(|error| {
            self.failure(
                match error {
                    comptime_runtime::bitvec::Error::Resource(
                        comptime_runtime::Error::BudgetExhausted,
                    ) => ErrorKind::StepsExhausted,
                    comptime_runtime::bitvec::Error::Resource(
                        comptime_runtime::Error::SizeLimit,
                    ) => ErrorKind::RepresentationSizeLimit,
                    _ => ErrorKind::Type,
                },
                Some(source),
            )
        })
    }

    // Keep the recursive dispatcher small. A monolithic debug-build match
    // reserves stack for temporaries from every arm at each recursive level,
    // exhausting the host stack before the logical depth budget can fire.
    fn eval_inner(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::OptionalPresent(_) | K::OptionalAbsent => self.eval_optional(expr, environment),
            K::EffectResume(_)
            | K::EffectHandle { .. }
            | K::EffectRun { .. }
            | K::EffectComputation { .. }
            | K::EffectRequest { .. } => self.eval_effect(expr, environment),
            K::Unit | K::Local(_) | K::Function(_) | K::Break(_) | K::Continue(_) => {
                self.eval_leaf(expr, environment)
            }
            K::StagedValue(_) => Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source))),
            K::ComptimeValue(..) | K::TypeValue(..) => self.eval_type_value(expr, environment),
            K::NormalFormType { .. } | K::ApplyNormalForm { .. } => {
                self.eval_normal_form_type(expr, environment)
            }
            K::FunctionType { .. } | K::FunctionResultType { .. } => {
                self.eval_type_constructor(expr, environment)
            }
            K::OptionalType(..)
            | K::PointerType(..)
            | K::TraitObjectType { .. }
            | K::ClosureType { .. } => self.eval_type_constructor(expr, environment),
            K::PlannedFunctionInstance { .. } | K::NominalType { .. } => {
                self.eval_declaration_plan(expr, environment)
            }
            K::CheckTypeAlias { .. } | K::WithTypeInputs { .. } => {
                self.eval_declaration_plan(expr, environment)
            }
            K::ScalarEqual(..)
            | K::ScalarLessEqual(..)
            | K::ScalarLess(..)
            | K::ScalarGreater(..)
            | K::ScalarGreaterEqual(..) => self.eval_scalar_operation(expr, environment),
            K::TypeEqual(..) => self.eval_type_equal(expr, environment),
            K::TypeKind { .. } => self.eval_type_kind(expr, environment),
            K::TypeNotSized { .. } => self.eval_type_not_sized(expr, environment),
            K::TemplateEqual(..) => self.eval_template_equal(expr, environment),
            K::ListLength(_) | K::ListConcat(..) => self.eval_list_operation(expr, environment),
            K::BuildStruct(_) | K::BuildEnum(_) | K::BuildNewtype(_) => {
                self.eval_definition_build(expr, environment)
            }
            K::EnumStructure { .. } => self.eval_observation(expr, environment),
            K::ComptimeFunctionInstance { .. } => {
                self.eval_comptime_function_instance(expr, environment)
            }
            K::ObjectEdit { .. } => self.eval_object_edit(expr, environment),
            K::ObjectQuery { .. } => self.eval_object_query(expr, environment),
            K::NewtypeRepresentation(_)
            | K::DefinitionAttributes(_)
            | K::StructFieldTypes(_)
            | K::StructFieldNames(_)
            | K::RecordFieldTypes(_)
            | K::RecordFieldNames(_)
            | K::ReflectCallable(_)
            | K::TypeId(_)
            | K::TypeLayout { .. } => self.eval_observation(expr, environment),
            K::TypeListIndex(..) => self.eval_type_list_index(expr, environment),
            K::TypeList(..) => self.eval_type_list(expr, environment),
            K::BitVecType(..) => self.eval_bit_vec_type(expr, environment),
            K::Literal(..) => self.eval_literal(expr, environment),
            K::BuildMetadata { .. } => self.eval_build_metadata(expr, environment),
            K::Variant { .. } => self.eval_variant(expr, environment),
            K::Record { .. } | K::AnonymousRecord { .. } => self.eval_record(expr, environment),
            K::Object { .. } => self.eval_object(expr, environment),
            K::Field { .. } | K::AnonymousField { .. } => self.eval_field(expr, environment),
            K::AddressOf(_)
            | K::ReadPointer(_)
            | K::PointerAnonymousField { .. }
            | K::PointerField { .. }
            | K::WritePointer { .. } => self.eval_pointer(expr, environment),
            K::TraitMethod { .. } => self.eval_trait_method(expr, environment, false),
            K::AssociatedValue { .. } => self.eval_associated_value(expr),
            K::FunctionInstance { .. } => self.eval_function_instance(expr, environment),
            K::Closure { .. } => self.eval_closure(expr, environment),
            K::Tuple(..) => self.eval_tuple(expr, environment),
            K::Match { .. } => self.eval_match(expr, environment),
            K::Block { .. } | K::CallPreparation { .. } => self.eval_block(expr, environment),
            K::LetPattern { .. } => self.eval_let_pattern(expr, environment),
            K::BitWritePointer { .. } => self.eval_bit_write_pointer(expr, environment),
            K::BitAssign { .. } | K::BitUpdate { .. } => self.eval_bit_assign(expr, environment),
            K::Assign { .. } => self.eval_assign(expr, environment),
            K::Loop { .. } => self.eval_loop(expr, environment),
            K::Let { .. } => self.eval_let(expr, environment),
            K::RequirePredicate(..) | K::EnsurePredicate(..) | K::CheckPredicate(..) => {
                self.eval_check_predicate(expr, environment)
            }
            K::Intrinsic { .. } => self.eval_scalar_math(expr, environment),
            K::UnsafeScope(body) => self.eval(body, environment),
            K::Discard(..) => self.eval_discard(expr, environment),
            K::Return(..) => self.eval_return(expr, environment),
            K::If { .. } => self.eval_if(expr, environment),
            K::BitSlice { .. } | K::BitIndex(..) | K::BitWindow { .. } => {
                self.eval_bit_values(expr, environment)
            }
            K::BitEqual(..) => self.eval_bit_values(expr, environment),
            K::FloatNegate(..) | K::FloatBinary { .. } => {
                self.eval_scalar_operation(expr, environment)
            }
            K::IntegerNegate(..) => self.eval_scalar_operation(expr, environment),
            K::IntegerBinary { .. } => self.eval_scalar_operation(expr, environment),
            K::BitConvert(..) => self.eval_bit_values(expr, environment),
            K::ErrorConvert { .. } => self.eval_error_convert(expr, environment),
            K::NewtypeConvert(..) => self.eval_newtype_convert(expr, environment),
            K::BitConcat(..) => self.eval_bit_values(expr, environment),
            K::EnumName(_) => self.eval_observation(expr, environment),
            K::SymbolName(_) => self.eval_observation(expr, environment),
            K::TextQuery { .. } => self.eval_observation(expr, environment),
            K::Snapshot(_) => self.eval_snapshot(expr, environment),
            K::ComptimeIntegerConvert(_) => self.eval_scalar_operation(expr, environment),
            K::IntegerRepresentation(_) => self.eval_scalar_operation(expr, environment),
            K::TryIntegerConvert(_) => self.eval_scalar_operation(expr, environment),
            K::TakeBits(..) => self.eval_bit_values(expr, environment),
            K::Not(..) => self.eval_not(expr, environment),
            K::And(..) | K::Or(..) => self.eval_and(expr, environment),
            K::Call { .. } => self.eval_call(expr, environment),
            K::ConstAddress { .. } | K::StaticAddress(_) | K::DynCast(_) | K::DynCall(_) => {
                return Err(self.failure(
                    if self.comptime_only {
                        ErrorKind::PhaseMismatch
                    } else {
                        ErrorKind::Unsupported
                    },
                    Some(expr.source),
                ));
            }
        }
    }

    #[inline(never)]
    fn eval_leaf(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::Unit => Value::Unit,
            K::Local(id) => self.local_value(environment, *id, Some(expr.source))?,
            K::Function(id) if self.functions.runtime_identity(id).is_some() => {
                self.function_instance_value(*id, &[], &expr.ty, expr.source)?
            }
            K::Function(id) => Value::Function(*id),
            K::Break(identity) => {
                self.cleanup_site(expr.source, iir::cleanup::Boundary::Break, environment)?;
                return Ok(Flow::Break(*identity));
            }
            K::Continue(identity) => {
                self.cleanup_site(expr.source, iir::cleanup::Boundary::Continue, environment)?;
                return Ok(Flow::Continue(*identity));
            }
            _ => unreachable!("expression dispatcher selected an incompatible leaf"),
        };
        Ok(Flow::Value(result))
    }

    fn freeze_normal_argument(
        &mut self,
        value: &Value,
        source: SourceRange,
    ) -> Result<nir::described_applications::DataArgument, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps: before,
            max_limbs: 4096,
        };
        let mut fields = Vec::new();
        let template = value.freeze_input_template(
            &mut budget,
            &self.abstract_type_parameters,
            &self.abstract_value_parameters,
            self.runtime_domains(),
            true,
            Some(&mut fields),
        );
        self.budget.steps -= (before - budget.steps) as u64;
        let template = template.map_err(|kind| self.failure(kind, Some(source)))?;
        Ok(nir::described_applications::DataArgument::new(
            template, fields,
        ))
    }

    fn complete_normal_data(
        &mut self,
        head: DeclarationId,
        inputs: Vec<nir::described_applications::DataArgument>,
        result_ty: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if *result_ty != Term::Primitive(Primitive::Type) {
            if inputs.iter().any(|input| !input.is_concrete()) {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)));
            }
            let arguments = inputs
                .into_iter()
                .map(|input| input.into_concrete().expect("concrete input"))
                .collect();
            return self.complete_normal_value(head, arguments, result_ty, source);
        }
        if inputs.iter().all(|input| input.is_concrete()) {
            return self.complete_normal_type(
                head,
                inputs
                    .into_iter()
                    .map(|input| input.into_concrete().expect("concrete input"))
                    .collect(),
                source,
            );
        }
        let world = self
            .declaration_world
            .filter(|_| self.comptime_only)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let mut parameters = Vec::new();
        let validation = inputs
            .iter()
            .map(|input| input.in_telescope(&mut parameters, &mut fuel))
            .collect::<Result<Vec<_>, _>>();
        self.budget.steps -= (before - fuel) as u64;
        let arguments = validation.map_err(|_| {
            self.failure(
                if fuel == 0 {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })?;
        // Validate the original head and all argument domains in one binder
        // frame. This temporary type never escapes as a result or PTC fact.
        let schema = Term::Function {
            parameters,
            result: Box::new(Term::NormalForm {
                head,
                sort: nir::Sort::Type,
                arguments,
            }),
        };
        self.check_type_description(&schema, Some(source))?;
        self.admit_type(&schema, 0)?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let data = world.refinements.application(head, inputs, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        data.map(Value::Description).map_err(|_| {
            self.failure(
                if fuel == 0 {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })
    }

    fn complete_normal_type(
        &mut self,
        head: DeclarationId,
        arguments: Vec<Term>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let metadata = self
            .normal_forms
            .get(&head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let ty = Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            arguments,
        };
        self.check_type_description(&ty, Some(source))?;
        let Term::NormalForm {
            arguments: values, ..
        } = &ty
        else {
            unreachable!()
        };
        self.check_normal_requirements(metadata, values, &[], source)?;
        Ok(Value::Type(ty))
    }

    fn complete_normal_value(
        &mut self,
        head: DeclarationId,
        arguments: Vec<Term>,
        result_ty: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let metadata = self
            .normal_forms
            .get(&head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        if metadata.role != nir::normal_forms::Role::Opaque {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.check_type_description(result_ty, Some(source))?;
        self.admit_type(result_ty, 0)?;
        let term = Term::NormalForm {
            head,
            sort: nir::Sort::ComptimeValue,
            arguments,
        };
        self.check_term_description(&term, Some(source))?;
        let Term::NormalForm {
            arguments: values, ..
        } = &term
        else {
            unreachable!()
        };
        self.check_normal_requirements(metadata, values, &[], source)?;
        Ok(Value::NormalForm {
            term,
            ty: result_ty.clone(),
        })
    }

    fn check_normal_requirements(
        &mut self,
        metadata: &nir::normal_forms::Head,
        values: &[Term],
        frames: &[Vec<nir::Sort>],
        source: SourceRange,
    ) -> Result<(), Error> {
        self.check_normal_requirements_in(metadata, values, frames, Some(source))
    }

    pub(crate) fn check_normal_requirements_in(
        &mut self,
        metadata: &nir::normal_forms::Head,
        values: &[Term],
        frames: &[Vec<nir::Sort>],
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        if metadata.requirements.is_empty() {
            return Ok(());
        }
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let prepared = (|| {
            let substitution = metadata.input_substitution(values, frames, &mut remaining)?;
            metadata
                .requirements
                .iter()
                .map(|predicate| {
                    substitution.instantiate_predicate_bounded(predicate, &mut remaining)
                })
                .collect::<Result<Vec<_>, nir::type_computations::Error>>()
        })();
        self.budget.steps -= (before - remaining) as u64;
        let predicates = prepared.map_err(|error| {
            self.failure(
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                source,
            )
        })?;
        for predicate in predicates {
            self.check_formation_predicate_in(&predicate, !frames.is_empty(), source)?;
        }
        Ok(())
    }

    /// Only data with a structural normal-input representation is frozen here.
    /// Callable/storage identities continue to use their ordinary environment
    /// binding; an aggregate containing one must not acquire a partial snapshot.
    fn normal_input_is_data(&mut self, value: &Value, source: SourceRange) -> Result<bool, Error> {
        let mut pending = vec![(value, 0usize)];
        while let Some((value, depth)) = pending.pop() {
            self.tick(Some(source))?;
            if depth > 128 {
                return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
            }
            let width = match value {
                Value::List { values, .. } => values.len(),
                Value::Tuple(values)
                | Value::Record { fields: values, .. }
                | Value::Variant { fields: values, .. } => values.len(),
                Value::Object(object) => object.entries().len(),
                Value::Optional { value, .. } => usize::from(value.is_some()),
                Value::Newtype { .. } | Value::ErrorResult { .. } => 1,
                Value::AnonymousRecord(fields) => fields.len(),
                Value::AbstractValue { declaration, ty }
                    if self.abstract_value_parameters.get(declaration) == Some(ty) =>
                {
                    continue;
                }
                Value::Unit
                | Value::Bool(_)
                | Value::Char(_)
                | Value::String(_)
                | Value::Symbol(_)
                | Value::UnsignedInteger(_)
                | Value::SignedInteger(_)
                | Value::Float(_)
                | Value::Integer(_)
                | Value::Float16(_)
                | Value::BFloat16(_)
                | Value::Float32(_)
                | Value::Float64(_)
                | Value::BitVec(_)
                | Value::Type(_)
                | Value::NormalForm { .. }
                | Value::TypeParameter(_) => continue,
                _ => return Ok(false),
            };
            // Charge width before growing the traversal stack. Freezing checks
            // payload sizes, labels and completed embedded types separately.
            self.integer_operation(Some(source), |budget| budget.consume(width))?;
            match value {
                Value::List { values, .. } => {
                    pending.extend(values.iter().rev().map(|value| (value, depth + 1)))
                }
                Value::Tuple(values)
                | Value::Record { fields: values, .. }
                | Value::Variant { fields: values, .. } => {
                    pending.extend(values.iter().rev().map(|value| (value, depth + 1)))
                }
                Value::Object(object) => pending.extend(
                    object
                        .entries()
                        .iter()
                        .rev()
                        .map(|entry| (&entry.value().value, depth + 1)),
                ),
                Value::Optional { value, .. } => {
                    pending.extend(value.as_deref().map(|v| (v, depth + 1)));
                }
                Value::Newtype { value, .. } | Value::ErrorResult { value, .. } => {
                    pending.push((value, depth + 1));
                }
                Value::AnonymousRecord(fields) => {
                    pending.extend(fields.iter().rev().map(|(_, value)| (value, depth + 1)))
                }
                _ => unreachable!(),
            }
        }
        Ok(true)
    }

    // Type construction may retain authorized value binders during a generic
    // invocation. Completed input export keeps its separate closed rules.
    fn residual_input_term(&mut self, value: Value, source: SourceRange) -> Result<Term, Error> {
        let steps = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps,
            max_limbs: 4096,
        };
        let term = value.into_scoped_term(
            &mut budget,
            &self.abstract_type_parameters,
            &self.abstract_value_parameters,
            self.runtime_domains(),
            true,
        );
        self.budget.steps -= (steps - budget.steps) as u64;
        term.map_err(|kind| self.failure(kind, Some(source)))
    }

    fn borrowed_residual_input_term(
        &mut self,
        value: &Value,
        source: SourceRange,
    ) -> Result<Term, Error> {
        let steps = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps,
            max_limbs: 4096,
        };
        let term = value.to_scoped_term(
            &mut budget,
            &self.abstract_type_parameters,
            &self.abstract_value_parameters,
            self.runtime_domains(),
            true,
        );
        self.budget.steps -= (steps - budget.steps) as u64;
        term.map_err(|kind| self.failure(kind, Some(source)))
    }

    fn append_normal_list_item(
        &mut self,
        ty: &Term,
        values: &mut Vec<Value>,
        spread: bool,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        if spread {
            self.validate_immutable_input(&value, ty)
                .map_err(|mut failure| {
                    failure.source.get_or_insert(source);
                    failure
                })?;
            let elements = match value {
                Value::List {
                    values: elements, ..
                } => {
                    self.integer_operation(Some(source), |budget| budget.consume(elements.len()))?;
                    elements
                }
                _ => return Err(self.failure(ErrorKind::Type, Some(source))),
            };
            if values.len().saturating_add(elements.len()) > 100_000 {
                return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
            }
            values.extend(elements);
        } else {
            // Normalize authorized opaque Type inputs before later operands run.
            let value = if self.list_element(ty) == Some(&Term::Primitive(Primitive::Type)) {
                self.type_data(value, source)?
            } else {
                value
            };
            values.push(value);
        }
        if values.len() > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        Ok(())
    }

    fn finish_normal_list(&mut self, ty: Term, values: Vec<Value>) -> Result<Value, Error> {
        let value = Value::List {
            ty: ty.clone(),
            values: values.into(),
        };
        self.validate_immutable_input(&value, &ty)?;
        Ok(value)
    }

    fn prepare_normal_argument(
        &mut self,
        input: &nir::program::NormalFormArgument,
        expected: &Term,
        literals: &mut BTreeMap<InputKey, PreparedNormalInput>,
        depth: usize,
    ) -> Result<(), Error> {
        let prepared =
            self.prepare_input_domain(input_domains::Node::Source(input, None), expected, depth)?;
        prepared.deliver_literals(literals);
        Ok(())
    }

    fn check_type_contract(
        &mut self,
        predicate: &nir::program::TypePredicate,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.check_type_contract_in(predicate, Some(source))
    }

    fn check_type_contract_in(
        &mut self,
        predicate: &nir::program::TypePredicate,
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        let normalized;
        // A resolving VM retains only the associated values already demanded
        // by execution. The declaration solver owns the full projection world.
        let predicate = if !matches!(self.type_solver, Some(TypeSolver::Declarations(_)))
            && predicate.trivial_truth().is_none()
            && !self.associated_values.is_empty()
        {
            let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let result =
                selected_types::predicate(predicate, &self.associated_values, &mut fuel, 0);
            self.budget.steps = fuel as u64;
            normalized = result.map_err(|kind| self.failure(kind, source))?;
            &normalized
        } else {
            predicate
        };
        let outcome = match predicate.trivial_truth() {
            Some(true) => ContractOutcome::Satisfied,
            Some(false) => ContractOutcome::Rejected,
            None => self
                .solve_type_predicate(predicate)
                .map_err(|kind| self.failure(kind, source))?,
        };
        match outcome {
            ContractOutcome::Satisfied => Ok(()),
            ContractOutcome::Rejected => Err(self.failure(ErrorKind::TypeContractRejected, source)),
            ContractOutcome::Unknown => {
                Err(self.failure(ErrorKind::TypeContractUnresolved, source))
            }
        }
    }

    #[inline(never)]
    fn eval_type_value(
        &mut self,
        expr: &Expr,
        _environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let value = match &expr.kind {
            K::ComptimeValue(value) => self.frozen_constant(value, &expr.ty, expr.source)?,
            K::TypeValue(value) => self.type_constant(value, &expr.ty, expr.source)?,
            _ => unreachable!("type value dispatcher selected an incompatible operand"),
        };
        Ok(Flow::Value(value))
    }

    fn frozen_constant(
        &mut self,
        value: &Term,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = nir::comptime_values::validate_in(value, ty, self.target, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|error| {
            self.failure(
                if error == nir::comptime_values::Error::Budget {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })?;
        self.normal_pattern_value(value, ty)
    }

    fn type_constant(
        &mut self,
        ty: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if *output != Term::Primitive(Primitive::Type) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if let Term::Rigid {
            declaration,
            sort: nir::Sort::Type,
        } = ty
        {
            if self.abstract_type_parameters.contains(declaration) {
                self.tick(Some(source))?;
                return Ok(Value::TypeParameter(*declaration));
            }
        }
        // Structural scope permits transporting residual identities;
        // their nominal domains still need independent admission.
        if !self.abstract_value_parameters.is_empty() {
            self.admit_type(ty, 0)?;
        }
        match self.check_type_description(ty, Some(source)) {
            Ok(()) => return Ok(Value::Type(ty.clone())),
            Err(error)
                if matches!(
                    error.kind,
                    ErrorKind::Type
                        | ErrorKind::UnmaterializedDispatch
                        | ErrorKind::TypeContractUnresolved
                ) => {}
            Err(error) => return Err(error),
        }
        let ty = self.selected_type_view(ty, Some(source))?;
        self.check_type_description(&ty, Some(source))?;
        Ok(Value::Type(ty))
    }

    #[inline(never)]
    fn eval_declaration_plan(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let mut frame = declaration_plan::Frame::new(
            self,
            input_collection::Plan::borrowed(expr),
            environment,
        )?;
        let result = self.run_type_plan(&mut frame, environment);
        input_collection::InputPlan::cleanup(&frame, environment);
        result
    }

    #[inline(never)]
    fn eval_literal(&mut self, expr: &Expr, _environment: &mut Environment) -> Result<Flow, Error> {
        let K::Literal(literal) = &expr.kind else {
            unreachable!()
        };
        self.literal_constant(literal, &expr.ty, expr.source)
            .map(Flow::Value)
    }

    fn form_float_literal(
        &mut self,
        spelling: &str,
        negative: bool,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Term::Primitive(
            domain @ (Primitive::Float
            | Primitive::F16
            | Primitive::Bf16
            | Primitive::F32
            | Primitive::F64),
        ) = *ty
        else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let literal = self
            .with_arithmetic_budget(|budget| {
                nir::float_formation::FloatFormation::parse(spelling, negative, budget)?
                    .form(domain, budget)
            })
            .map_err(|cause| {
                let kind = match cause {
                    comptime_runtime::Error::OutOfRange => ErrorKind::FloatLiteralOverflow(domain),
                    other => other.into(),
                };
                self.failure(kind, Some(source))
            })?;
        self.literal_constant(&literal, ty, source)
    }

    fn literal_constant(
        &mut self,
        literal: &Literal,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if let Literal::PendingInteger { spelling, negative } = literal {
            let Term::Primitive(domain) = *ty else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            let target = self.target;
            let literal = self.integer_operation(Some(source), |budget| {
                nir::integer_formation::form(spelling, *negative, domain, target, budget)
            })?;
            return self.literal_constant(&literal, ty, source);
        }
        if let Literal::PendingFloat { spelling, negative } = literal {
            return self.form_float_literal(spelling, *negative, ty, source);
        }
        if !literal.has_primitive_type_in(ty, self.target) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let result = match literal {
            Literal::Integer(spelling) => {
                Value::Integer(self.integer_literal(spelling, Some(source))?)
            }
            Literal::BitVec { width, words } => {
                Value::BitVec(self.bit_operation(source, |budget| {
                    comptime_runtime::bitvec::BitVec::from_words(*width, words, budget)
                })?)
            }
            Literal::Float(spelling) => {
                Value::Float(self.integer_operation(Some(source), |budget| {
                    comptime_runtime::decimal::DecimalValue::parse_literal(spelling, budget)
                })?)
            }
            Literal::Float16(bits) => Value::Float16(*bits),
            Literal::BFloat16(bits) => Value::BFloat16(*bits),
            Literal::Float32(bits) => Value::Float32(*bits),
            Literal::Float64(bits) => Value::Float64(*bits),
            Literal::SignedInteger(v) => {
                let value = Value::SignedInteger(*v);
                value
            }
            Literal::UnsignedInteger(v) => {
                let value = Value::UnsignedInteger(*v);
                value
            }
            Literal::Description(description) => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
                }
                if !self.admits_description(description) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                Value::Description(description.clone())
            }
            Literal::Bool(v) => Value::Bool(*v),
            Literal::Char(v) => Value::Char(*v),
            Literal::String(v) | Literal::Symbol(v) => {
                self.integer_operation(Some(source), |budget| {
                    budget.consume(v.len().saturating_add(1))
                })?;
                let text = Arc::from(v.as_str());
                if matches!(literal, Literal::String(_)) {
                    Value::String(text)
                } else {
                    Value::Symbol(text)
                }
            }
            _ => return Err(self.failure(ErrorKind::Unsupported, Some(source))),
        };
        Ok(result)
    }

    #[inline(never)]
    fn eval_field(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        if let Some(value) = self.eval_local_field(expr, environment) {
            return value;
        }
        let (base, nominal, name, index) = match &expr.kind {
            K::AnonymousField { value, name, index } => (value, None, Some(name.as_str()), *index),
            K::Field {
                value,
                declaration,
                index,
            } => (value, Some(*declaration), None, *index),
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        let representation = value!(self.eval(base, environment));
        if !matches!(
            base.kind,
            K::Field { .. } | K::AnonymousField { .. } | K::Local(_)
        ) {
            self.retain_residual(base.source, &representation)?;
        }
        self.read_record_field(
            &representation,
            nominal,
            name,
            index,
            &expr.ty,
            base.source,
            expr.source,
        )
        .map(Flow::Value)
    }

    #[inline(never)]
    fn eval_trait_method(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        direct_callee: bool,
    ) -> Result<Flow, Error> {
        let method =
            comptime_compiler::methods::View::expression(expr).expect("trait method dispatcher");
        match self.select_trait_method(method, direct_callee)? {
            trait_methods::Selection::Value(value) => Ok(Flow::Value(value)),
            trait_methods::Selection::Function {
                declaration,
                arguments,
                ty,
                instantiate,
            } => {
                let resolved = Expr {
                    ty,
                    source: expr.source,
                    kind: if instantiate {
                        K::FunctionInstance {
                            declaration,
                            arguments,
                        }
                    } else {
                        K::Function(declaration)
                    },
                };
                self.eval(&resolved, environment)
            }
        }
    }

    #[inline(never)]
    fn eval_function_instance(
        &mut self,
        expr: &Expr,
        _environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::FunctionInstance {
            declaration,
            arguments,
        } = &expr.kind
        else {
            unreachable!()
        };
        self.function_instance_value(*declaration, arguments, &expr.ty, expr.source)
            .map(Flow::Value)
    }

    fn closure_capture_types<'t>(
        &self,
        ty: &'t Term,
        parameters: usize,
        captures: usize,
        source: SourceRange,
    ) -> Result<&'t [Term], Error> {
        let capture_types: &[Term] = match ty {
            Term::Closure { environment, .. } => environment,
            Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_) => &[],
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        let Term::Function {
            parameters: types, ..
        } = ty.callable_signature()
        else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if types.len() != parameters || capture_types.len() != captures {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(capture_types)
    }

    fn check_closure_capture(
        &mut self,
        value: &Value,
        supplied: &Term,
        expected: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if supplied != expected {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.check_value_type(value, expected, Some(source))
    }

    #[inline(never)]
    fn eval_closure(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let result = match &expr.kind {
                K::Closure {
                    parameters,
                    captures,
                    body,
                } => {
                    let capture_types = this.closure_capture_types(
                        &expr.ty,
                        parameters.len(),
                        captures.len(),
                        expr.source,
                    )?;
                    let mut captured = Environment::new();
                    for (slot, (capture, ty)) in captures.iter().zip(capture_types).enumerate() {
                        this.tick(Some(expr.source))?;
                        let value = if let Some(value) = &capture.value {
                            value!(this.eval(value, environment))
                        } else {
                            let mut value =
                                this.local_value(environment, capture.binding, Some(expr.source))?;
                            this.cleanup_copy_delivery(
                                expr.source,
                                capture.binding,
                                &[],
                                &mut value,
                            )?;
                            this.cleanup_transfer(expr.source, capture.binding, &[], environment)?;
                            value
                        };
                        this.check_closure_capture(&value, &capture.ty, ty, expr.source)?;
                        this.retain_prepared(
                            slot,
                            capture
                                .value
                                .as_ref()
                                .map_or(expr.source, |value| value.source),
                            &value,
                        )?;
                        // Each construction owns fresh capture cells; repeated
                        // invocations retain updates in either execution phase.
                        // Constant materialization snapshots their values and
                        // reconstructs sharing without exporting active storage.
                        let value = Value::Storage(memory::Storage::new(value));
                        if parameters.contains(&capture.binding)
                            || captured.insert(capture.binding, value).is_some()
                        {
                            return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                        }
                    }
                    this.events.push(Event::CreateClosure {
                        source: expr.source,
                        captures: captures.len(),
                    });
                    Value::Closure(Arc::new(Closure {
                        definition_source: Some(expr.source),
                        parameters: parameters.clone(),
                        environment: captured,
                        capture_order: captures.iter().map(|capture| capture.binding).collect(),
                        body: body.clone(),
                        ty: expr.ty.clone(),
                    }))
                }
                _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
            };
            Ok(Flow::Value(result))
        })
    }

    #[inline(never)]
    fn eval_tuple(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let result = match &expr.kind {
                K::Tuple(items) => {
                    let mut values = Vec::new();
                    for (slot, item) in items.iter().enumerate() {
                        let value = value!(this.eval(item, environment));
                        this.retain_prepared(slot, item.source, &value)?;
                        values.push(value);
                    }
                    Value::Tuple(values)
                }
                _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
            };
            Ok(Flow::Value(result))
        })
    }

    #[inline(never)]
    fn match_arm_guard(
        &mut self,
        pattern: &nir::program::Pattern,
        guard: Option<&Expr>,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.match_arm_guard_with_bindings(pattern, guard, value, environment)
            .map(|(flow, _)| flow)
    }

    fn match_arm_guard_with_bindings(
        &mut self,
        pattern: &nir::program::Pattern,
        guard: Option<&Expr>,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<(Flow, PatternBindings), Error> {
        let (flow, bindings, _) =
            self.match_arm_guard_with_bindings_and_then(pattern, guard, value, environment)?;
        Ok((flow, bindings))
    }

    fn match_arm_guard_with_bindings_and_then(
        &mut self,
        pattern: &nir::program::Pattern,
        guard: Option<&Expr>,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<(Flow, PatternBindings, Vec<PendingThen>), Error> {
        let (matched, bindings, mut pending_then) =
            self.match_pattern_with_bindings_and_then(pattern, value, environment)?;
        if !matches!(matched, Flow::Value(Value::Bool(true))) {
            return Ok((matched, PatternBindings::default(), Vec::new()));
        }
        let flow = if let Some(guard) = guard {
            match self.eval(guard, environment) {
                Ok(flow @ Flow::Value(Value::Bool(_))) => flow,
                Ok(Flow::Value(Value::AbstractValue { .. })) => {
                    let _ = self.cleanup_pending_then(&mut pending_then, 0, environment);
                    return Err(
                        self.failure(ErrorKind::AbstractValueUnresolved, Some(guard.source))
                    );
                }
                Ok(Flow::Value(_)) => {
                    let _ = self.cleanup_pending_then(&mut pending_then, 0, environment);
                    return Err(self.failure(ErrorKind::Type, Some(guard.source)));
                }
                Ok(flow) => {
                    self.cleanup_pending_then(&mut pending_then, 0, environment)?;
                    flow
                }
                Err(error) => {
                    let _ = self.cleanup_pending_then(&mut pending_then, 0, environment);
                    return Err(error);
                }
            }
        } else {
            Flow::Value(Value::Bool(true))
        };
        if matches!(flow, Flow::Value(Value::Bool(true))) {
            Ok((flow, bindings, pending_then))
        } else {
            if matches!(flow, Flow::Value(Value::Bool(false))) {
                self.cleanup_pending_then(&mut pending_then, 0, environment)?;
            }
            Ok((flow, PatternBindings::default(), Vec::new()))
        }
    }

    fn cleanup_pending_then(
        &mut self,
        pending: &mut Vec<PendingThen>,
        mark: usize,
        environment: &mut Environment,
    ) -> Result<(), Error> {
        while pending.len() > mark {
            let item = pending.pop().unwrap();
            self.cleanup_pattern_then_failure(item.source, item.value, environment)?;
        }
        Ok(())
    }

    #[inline(never)]
    fn eval_match(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::Match { value, arms } => {
                let value = value!(self.eval(value, environment));
                for arm in arms {
                    let flow = environment.scoped(|environment| {
                        self.tick(Some(expr.source))?;
                        let (matched, bindings, pending_then) = self
                            .match_arm_guard_with_bindings_and_then(
                                &arm.pattern,
                                arm.guard.as_ref(),
                                &value,
                                environment,
                            )?;
                        match matched {
                            Flow::Value(Value::Bool(false)) => return Ok(None),
                            Flow::Value(Value::Bool(true)) => {}
                            flow => return Ok(Some(flow)),
                        }
                        for pt in &pending_then {
                            self.retain_pattern_residual(pt.source, &pt.value, &pt.bindings)?;
                        }
                        self.retain_pattern_residual(arm.body.source, &value, &bindings)?;
                        let result = self.eval(&arm.body, environment)?;
                        if matches!(result, Flow::Value(_))
                            && !matches!(arm.body.kind, K::Block { .. } | K::CallPreparation { .. })
                        {
                            self.cleanup_site(
                                arm.body.source,
                                iir::cleanup::Boundary::ScopeEnd,
                                environment,
                            )?;
                        }
                        Ok(Some(result))
                    });
                    if let Some(flow) = flow? {
                        return Ok(flow);
                    }
                }
                return Err(self.failure(ErrorKind::NonExhaustiveMatch, Some(expr.source)));
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    #[inline(never)]
    fn eval_block(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let mut frames = Vec::new();
        let mut current = expr;
        let mut flow = 'evaluate: loop {
            environment.begin_scope();
            frames.push(current);
            let (K::Block { statements, result } | K::CallPreparation { statements, result }) =
                &current.kind
            else {
                unreachable!("expression dispatcher selected an incompatible evaluator")
            };
            for statement in statements {
                match self.eval(statement, environment) {
                    Ok(Flow::Value(_)) => {}
                    other => break 'evaluate other,
                }
            }
            let Some(tail) = result.as_deref() else {
                break Ok(Flow::Value(Value::Unit));
            };
            if matches!(tail.kind, K::Block { .. } | K::CallPreparation { .. }) {
                if let Err(failure) = self.tick(Some(tail.source)) {
                    break Err(failure);
                }
                current = tail;
            } else {
                break self.eval(tail, environment);
            }
        };
        while let Some(frame) = frames.pop() {
            let cleanup = if matches!(&flow, Ok(Flow::Value(_))) {
                if matches!(frame.kind, K::CallPreparation { .. }) {
                    self.cleanup_site(frame.source, iir::cleanup::Boundary::ScopeEnd, environment)
                } else {
                    self.cleanup_block(frame.source, environment)
                }
            } else {
                Ok(())
            };
            environment.leave_scope();
            if let Err(failure) = cleanup {
                flow = Err(failure);
            }
            // The outer eval owns the root's delivery hook. Every tail block
            // still receives the same hook after its lexical cleanup.
            if !frames.is_empty() {
                flow = self.finish_evaluation(frame, environment, flow);
            }
        }
        flow
    }

    #[inline(never)]
    fn eval_let_pattern(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                let value_source = value.source;
                let value = value!(self.eval(value, environment));
                environment.begin_scope();
                let succeeds = self.match_arm_guard_with_bindings_and_then(
                    pattern,
                    guard.as_deref(),
                    &value,
                    environment,
                );
                if let Ok((Flow::Value(Value::Bool(true)), bindings, pending_then)) = &succeeds {
                    for pt in pending_then {
                        if let Err(error) =
                            self.retain_pattern_residual(pt.source, &pt.value, &pt.bindings)
                        {
                            environment.leave_scope();
                            return Err(error);
                        }
                    }
                    match self.retain_pattern_residual(expr.source, &value, bindings) {
                        Ok(()) => {
                            environment.commit_scope();
                            return Ok(Flow::Value(Value::Unit));
                        }
                        Err(error) => {
                            environment.leave_scope();
                            return Err(error);
                        }
                    }
                }
                // Roll back candidate bindings, preserving writes to outer places.
                environment.leave_scope();
                match succeeds?.0 {
                    Flow::Value(Value::Bool(false)) => {
                        if let Some(otherwise) = otherwise {
                            self.retain_residual(value_source, &value)?;
                            return match self.eval(otherwise, environment)? {
                                Flow::Value(_) => {
                                    Err(self
                                        .failure(ErrorKind::InvalidControlFlow, Some(expr.source)))
                                }
                                transfer => Ok(transfer),
                            };
                        }
                        return Err(self.failure(ErrorKind::NonExhaustiveMatch, Some(expr.source)));
                    }
                    Flow::Value(_) => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                    transfer => return Ok(transfer),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    fn eval_bit_selections(
        &mut self,
        expr: &Expr,
        storage_type: &Term,
        selections: &[nir::program::BitSelection],
        environment: &mut Environment,
    ) -> Result<SelectedBitRange, Error> {
        let mut width = self
            .unrestricted_bit_storage_width(storage_type, expr.source)?
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let position = |value: Value| {
            match value {
                Value::Integer(n) => n.to_u128().ok(),
                Value::UnsignedInteger(n) => Some(n),
                Value::SignedInteger(n) => u128::try_from(n).ok(),
                _ => None,
            }
            .and_then(|n| usize::try_from(n).ok())
        };
        let mut offset = 0usize;
        for selection in selections {
            let (lo, hi, expected_width) = match selection {
                nir::program::BitSelection::Static { start, end } => {
                    (*start, *end, end.saturating_sub(*start))
                }
                nir::program::BitSelection::Index(index) => {
                    let evaluated = match self.eval(index, environment)? {
                        Flow::Value(value) => value,
                        control => return Ok(SelectedBitRange::Control(control)),
                    };
                    self.check_retained_value(&evaluated, &index.ty, Some(index.source))?;
                    let lo = position(evaluated).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(index.source))
                    })?;
                    let hi = lo.checked_add(1).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(index.source))
                    })?;
                    (lo, hi, 1)
                }
                nir::program::BitSelection::Window { start, end, width } => {
                    let lower = match self.eval(start, environment)? {
                        Flow::Value(value) => value,
                        control => return Ok(SelectedBitRange::Control(control)),
                    };
                    let upper = match self.eval(end, environment)? {
                        Flow::Value(value) => value,
                        control => return Ok(SelectedBitRange::Control(control)),
                    };
                    self.check_retained_value(&lower, &start.ty, Some(expr.source))?;
                    self.check_retained_value(&upper, &end.ty, Some(expr.source))?;
                    let lo = position(lower).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(start.source))
                    })?;
                    let hi = position(upper).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(end.source))
                    })?;
                    (lo, hi, *width)
                }
            };
            if lo > hi || hi > width {
                return Err(self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)));
            }
            if hi - lo != expected_width {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            offset = offset
                .checked_add(lo)
                .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;
            width = expected_width;
        }
        let end = offset
            .checked_add(width)
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;

        Ok(SelectedBitRange::Range(offset, end))
    }

    fn replaced_bit_value(
        &mut self,
        source: SourceRange,
        storage_type: &Term,
        target: &Value,
        start: usize,
        rhs: &comptime_runtime::bitvec::BitVec,
    ) -> Result<Value, Error> {
        self.check_retained_value(target, storage_type, Some(source))?;
        Ok(match target {
            Value::BitVec(bits) => Value::BitVec(
                self.bit_operation(source, |budget| bits.replaced(start, rhs, budget))?,
            ),
            Value::SignedInteger(_)
            | Value::UnsignedInteger(_)
            | Value::Float16(_)
            | Value::BFloat16(_)
            | Value::Float32(_)
            | Value::Float64(_) => {
                let Term::Primitive(primitive) = storage_type else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Some(width) = primitive.unrestricted_scalar_width_in(self.target) else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let raw = match target {
                    Value::SignedInteger(value) => *value as u128,
                    Value::UnsignedInteger(value) => *value,
                    Value::Float16(bits) | Value::BFloat16(bits) => u128::from(*bits),
                    Value::Float32(bits) => u128::from(*bits),
                    Value::Float64(bits) => u128::from(*bits),
                    _ => unreachable!(),
                };
                let raw = if width == 128 {
                    raw
                } else {
                    raw & ((1u128 << width) - 1)
                };
                let raw = self.bit_operation(source, |budget| {
                    let bits = comptime_runtime::Integer::from_u128(raw)
                        .to_bitvec(width as usize, budget)?;
                    let result = bits.replaced(start, rhs, budget)?;
                    budget.consume(result.words().len())?;
                    Ok(result
                        .words()
                        .iter()
                        .rev()
                        .fold(0u128, |acc, word| (acc << 32) | u128::from(*word)))
                })?;
                if *primitive == Primitive::F16 {
                    Value::Float16(raw as u16)
                } else if *primitive == Primitive::Bf16 {
                    Value::BFloat16(raw as u16)
                } else if *primitive == Primitive::F32 {
                    Value::Float32(raw as u32)
                } else if *primitive == Primitive::F64 {
                    Value::Float64(raw as u64)
                } else if self
                    .integer_layout(*primitive)
                    .is_some_and(|(signed, _)| signed)
                {
                    let shift = 128 - width;
                    Value::SignedInteger(((raw << shift) as i128) >> shift)
                } else {
                    Value::UnsignedInteger(raw)
                }
            }
            Value::Record { .. } => {
                if self
                    .unrestricted_bit_storage_width(storage_type, source)?
                    .is_none()
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let bits = self.encode_bit_packed_value(target, storage_type, source)?;
                let updated =
                    self.bit_operation(source, |budget| bits.replaced(start, rhs, budget))?;
                self.decode_bit_packed_value(&updated, storage_type, source)?
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        })
    }

    #[inline(never)]
    fn eval_bit_write_pointer(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let K::BitWritePointer {
            pointer,
            selections,
            value,
        } = &expr.kind
        else {
            unreachable!()
        };
        let Term::Pointer(storage_type) = &pointer.ty else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        self.charge_preparation(selections.len())?;
        self.check_bit_update_selections(
            storage_type,
            selections,
            &value.ty,
            &expr.ty,
            expr.source,
        )?;
        let target_value = value!(self.eval(pointer, environment));
        self.check_retained_value(&target_value, &pointer.ty, Some(expr.source))?;
        let Value::Pointer(target) = target_value else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        let (start, end) =
            match self.eval_bit_selections(expr, storage_type, selections, environment)? {
                SelectedBitRange::Range(start, end) => (start, end),
                SelectedBitRange::Control(control) => return Ok(control),
            };
        let Value::BitVec(rhs) = value!(self.eval(value, environment)) else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        if rhs.width() != end - start || expr.ty != Term::Primitive(Primitive::Void) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let old = target
            .read()
            .map_err(|kind| self.failure(kind, Some(expr.source)))?;
        let changed = self.replaced_bit_value(expr.source, storage_type, &old, start, &rhs)?;
        target
            .write(changed, |old| {
                self.check_retained_value(old, storage_type, Some(expr.source))
                    .map_err(|error| error.kind)
            })
            .map_err(|kind| self.failure(kind, Some(expr.source)))?;
        Ok(Flow::Value(Value::Unit))
    }

    #[inline(never)]
    fn eval_bit_assign(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (place, storage_type, start, end, expression) = match &expr.kind {
            K::BitAssign {
                place,
                storage_type,
                start,
                end,
                value,
            } => (place, storage_type, *start, *end, value.as_ref()),
            K::BitUpdate {
                place,
                storage_type,
                selections,
                value,
            } => {
                if !environment.contains_key(&place.root) {
                    return Err(self.failure(ErrorKind::MissingBinding, Some(expr.source)));
                }
                self.charge_preparation(selections.len())?;
                self.check_bit_update_selections(
                    storage_type,
                    selections,
                    &value.ty,
                    &expr.ty,
                    expr.source,
                )?;
                let (start, end) =
                    match self.eval_bit_selections(expr, storage_type, selections, environment)? {
                        SelectedBitRange::Range(start, end) => (start, end),
                        SelectedBitRange::Control(control) => return Ok(control),
                    };
                (place, storage_type, start, end, value.as_ref())
            }
            _ => unreachable!("bit writeback dispatcher"),
        };
        let result = {
            if !environment.contains_key(&place.root) {
                return Err(self.failure(ErrorKind::MissingBinding, Some(expr.source)));
            }
            // Finish the RHS before reading preserved bits. RHS writes to
            // this root or sibling fields must survive the final update.
            let Value::BitVec(rhs) = value!(self.eval(expression, environment)) else {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            };
            self.write_bit_place(
                environment.get_mut(&place.root).unwrap(),
                &place.fields,
                storage_type,
                start,
                end,
                rhs,
                &expr.ty,
                expr.source,
            )?;
            Value::Unit
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_assign(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        if let Some(result) = self.eval_local_list_append(expr, environment) {
            return result;
        }
        let result = match &expr.kind {
            K::Assign {
                place,
                value: expression,
            } => {
                if !environment.contains_key(&place.root) {
                    return Err(self.failure(ErrorKind::MissingBinding, Some(expr.source)));
                }
                let value = value!(self.eval(expression, environment));
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::Replacement,
                    environment,
                )?;
                if self.comptime_only {
                    let target = environment.get_mut(&place.root).unwrap();
                    self.write_comptime_place(target, &place.fields, value, expr.source)?;
                    return Ok(Flow::Value(Value::Unit));
                }
                // Resolve again after RHS evaluation: a write performed by the
                // RHS to a sibling or the root must not be lost to a snapshot.
                if matches!(
                    environment.get(&place.root),
                    Some(Value::ComptimeReference(_))
                ) {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
                }
                let storage = match environment.get(&place.root) {
                    Some(Value::Storage(storage)) => Some(storage.clone()),
                    _ => None,
                };
                if let Some(storage) = storage {
                    let pointer = storage
                        .assignment_pointer(expression.ty.clone(), &place.fields)
                        .map_err(|e| self.failure(e, Some(expr.source)))?;
                    pointer
                        .write(value, |_| Ok(()))
                        .map_err(|e| self.failure(e, Some(expr.source)))?;
                    return Ok(Flow::Value(Value::Unit));
                }
                let mut target = environment.get_mut(&place.root).unwrap();
                for (expected, index) in &place.fields {
                    target = match (expected, target) {
                        (
                            nir::program::FieldOwner::Nominal(expected),
                            Value::Record {
                                declaration,
                                fields,
                                ..
                            },
                        ) => {
                            if declaration == expected {
                                fields.get_mut(*index)
                            } else {
                                None
                            }
                        }
                        (nir::program::FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                            fields.get_mut(*index).map(|(_, value)| value)
                        }
                        _ => None,
                    }
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                }
                *target = value;
                Value::Unit
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_loop(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::Loop { identity, body } => self.with_effect_loop(*identity, expr.source, |this| {
                loop {
                    match this.eval(body, environment)? {
                        Flow::Value(_) => {}
                        Flow::Break(target) if target == *identity => {
                            return Ok(Flow::Value(Value::Unit));
                        }
                        Flow::Continue(target) if target == *identity => {}
                        transfer => return Ok(transfer),
                    }
                }
            }),
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    #[inline(never)]
    fn eval_let(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::Let {
                binding,
                value: expression,
            } => {
                let value = value!(self.eval(expression, environment));
                environment.insert(*binding, value);
                Value::Unit
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    fn check_predicate_shape(&mut self, condition: &Expr) -> Result<bool, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::program::checkable_predicate(condition, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(condition.source)))
    }

    #[inline(never)]
    fn eval_check_predicate(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (K::RequirePredicate(condition)
        | K::EnsurePredicate(condition)
        | K::CheckPredicate(condition)) = &expr.kind
        else {
            unreachable!()
        };
        if expr.ty != Term::Primitive(Primitive::Void) || !self.check_predicate_shape(condition)? {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let value = value!(self.eval(condition, environment));
        self.finish_predicate(value, expr.source, condition.source, environment)?;
        Ok(Flow::Value(Value::Unit))
    }

    fn finish_predicate(
        &mut self,
        value: Value,
        source: SourceRange,
        condition_source: SourceRange,
        environment: &mut Environment,
    ) -> Result<(), Error> {
        let Value::Bool(passed) = Self::concrete_value(value, source)? else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        self.events.push(Event::CheckPredicate { source, passed });
        if !passed {
            self.cleanup_site(
                condition_source,
                iir::cleanup::Boundary::PredicateFailure,
                environment,
            )?;
            return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
        }
        Ok(())
    }

    #[inline(never)]
    fn eval_discard(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::Discard(expression) => {
                let value = value!(self.eval(expression, environment));
                self.cleanup_discard(expr.source, expression.source, value)?;
                Value::Unit
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_return(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::Return(expression) => {
                let value = if let Some(expression) = expression {
                    value!(self.eval(expression, environment))
                } else {
                    Value::Unit
                };
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::TemporaryEnd,
                    environment,
                )?;
                self.check_scheduled_result(&value, expr.source, environment)?;
                self.cleanup_site(expr.source, iir::cleanup::Boundary::Return, environment)?;
                return Ok(Flow::Return(value));
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    #[inline(never)]
    fn eval_if(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = value!(self.eval(condition, environment));
                match condition {
                    Value::Bool(true) => value!(self.eval(then_branch, environment)),
                    Value::Bool(false) => {
                        if let Some(branch) = else_branch {
                            value!(self.eval(branch, environment))
                        } else {
                            Value::Unit
                        }
                    }
                    Value::AbstractValue { .. } => {
                        return Err(
                            self.failure(ErrorKind::AbstractValueUnresolved, Some(expr.source))
                        );
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_not(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        // A linear Not spine needs no native recursion. Keep every node's
        // execution cost and delivery boundary; only the host stack frames are
        // removed. The enclosing eval already charged the outermost node.
        let mut pending = vec![expr];
        let mut current = expr;
        let mut result = loop {
            let K::Not(operand) = &current.kind else {
                unreachable!("expression dispatcher selected an incompatible evaluator")
            };
            if !matches!(operand.kind, K::Not(_)) {
                break self.eval(operand, environment);
            }
            if let Err(error) = self.tick(Some(operand.source)) {
                break Err(error);
            }
            current = operand;
            pending.push(current);
        };
        while let Some(node) = pending.pop() {
            result = result.and_then(|flow| {
                let Flow::Value(value) = flow else {
                    return Ok(flow);
                };
                let value = match Self::concrete_value(value, node.source)? {
                    Value::Bool(value) => Value::Bool(!value),
                    Value::BitVec(value) => {
                        if node.ty != Term::Primitive(Primitive::BitVec(value.width())) {
                            return Err(self.failure(ErrorKind::Type, Some(node.source)));
                        }
                        Value::BitVec(self.bit_operation(node.source, |budget| value.not(budget))?)
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(node.source))),
                };
                Ok(Flow::Value(value))
            });
            if !pending.is_empty() {
                result = self.finish_evaluation(node, environment, result);
            }
        }
        // The enclosing eval delivers the outermost result exactly once.
        result
    }

    #[inline(never)]
    fn eval_and(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::And(a, b) | K::Or(a, b) => {
                let left = Self::concrete_value(value!(self.eval(a, environment)), expr.source)?;
                match left {
                    Value::BitVec(left) => {
                        // BitVec operations are eager, unlike boolean short circuiting.
                        let Value::BitVec(right) =
                            Self::concrete_value(value!(self.eval(b, environment)), expr.source)?
                        else {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        };
                        self.bit_values(
                            if matches!(expr.kind, K::And(..)) {
                                comptime_compiler::bits::Operation::And
                            } else {
                                comptime_compiler::bits::Operation::Or
                            },
                            &[&a.ty, &b.ty],
                            &expr.ty,
                            &mut [Value::BitVec(left), Value::BitVec(right)],
                            expr.source,
                        )?
                    }
                    Value::Bool(left) => {
                        if (matches!(expr.kind, K::And(..)) && !left)
                            || (matches!(expr.kind, K::Or(..)) && left)
                        {
                            Value::Bool(left)
                        } else {
                            let right = value!(self.eval(b, environment));
                            self.check_retained_value(
                                &right,
                                &Term::Primitive(Primitive::Bool),
                                Some(expr.source),
                            )?;
                            right
                        }
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn call_parameters<'v>(
        &'v self,
        callee: &'v Value,
        source: SourceRange,
    ) -> Result<&'v [LocalId], Error> {
        match callee {
            Value::RuntimeTraitMethod(_) => {
                Err(self.failure(ErrorKind::PhaseMismatch, Some(source)))
            }
            Value::Function(id)
            | Value::FunctionInstance {
                declaration: id, ..
            } => self
                .functions
                .declaration(id)
                .map(|f| f.parameters.as_slice())
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source))),
            Value::IirCallable(handle) => Ok(self.iir_target(handle)?.parameters.as_slice()),
            Value::Closure(closure) => Ok(closure.parameters.as_slice()),
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    fn interface_call_contract(
        &mut self,
        callee: &Expr,
        source: SourceRange,
    ) -> Result<(nir::contracts::ProgramContract, Option<Vec<LocalId>>), Error> {
        match comptime_compiler::methods::View::expression(callee) {
            Some(method) => self.interface_method_contract(method, source),
            None => Ok((Default::default(), None)),
        }
    }

    fn interface_method_contract(
        &mut self,
        method: comptime_compiler::methods::View<'_>,
        source: SourceRange,
    ) -> Result<(nir::contracts::ProgramContract, Option<Vec<LocalId>>), Error> {
        let comptime_compiler::methods::View {
            interface,
            member,
            subject,
            interface_arguments,
            method_arguments,
            ..
        } = method;
        let (program, bindings) = if let Some(contract) = self.iir_method(
            interface,
            interface_arguments,
            subject,
            member,
            method_arguments,
        )? {
            // The concrete graph already specialized all declaration inputs.
            self.charge_copy_work(Some(source), |fuel| {
                *fuel = fuel
                    .checked_sub(contract.program.entry_snapshots.len() + 1)
                    .ok_or(nir::copy_budget::CopyBudgetExhausted::Steps)?;
                for condition in contract
                    .program
                    .preconditions
                    .iter()
                    .chain(&contract.program.postconditions)
                {
                    nir::copy_budget::charge_substitution(condition, &[], &[], fuel)?;
                }
                Ok(())
            })?;
            (contract.program.clone(), &contract.parameter_bindings)
        } else {
            let (inputs, contract) = self.method_inputs(method)?;
            let frames = [inputs.frame];
            let admitted = self.admit_function_inputs(
                &inputs.parameters,
                &inputs.arguments,
                nir::DeclarationInputScope::ClosedInBody(&frames),
                source,
            )?;
            let program = self.query_admission(Some(source), |_, fuel| {
                Ok(contract.program.instantiate_admitted(&admitted, fuel)?)
            })?;
            let bindings = self
                .method_parameters
                .get(&(interface, member))
                .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
            (program, bindings)
        };
        self.budget.steps = self
            .budget
            .steps
            .checked_sub(bindings.len() as u64)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        Ok((program, Some(bindings.clone())))
    }

    #[inline(never)]
    fn prepare_call(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<PreparedCall, Error> {
        macro_rules! prepared_value {
            ($value:expr) => {
                match $value? {
                    Flow::Value(value) => value,
                    control => return Ok(PreparedCall::Control(control)),
                }
            };
        }
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
            env_arguments,
        } = &expr.kind
        else {
            unreachable!("call preparation")
        };
        if !env_arguments.is_empty() {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        if callee.ty.is_normal_form_callable() {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let prepared_trait;
        let mut trait_default_inputs = None;
        let callee = match self.prepare_dynamic_trait_instance(callee, environment)? {
            dynamic_instances::PreparedTraitInstance::Ready(method) => {
                if arguments.iter().any(|argument| argument.is_default) {
                    trait_default_inputs = Some(self.trait_default_inputs(&method)?);
                }
                prepared_trait = method;
                &prepared_trait
            }
            dynamic_instances::PreparedTraitInstance::Control(flow) => {
                return Ok(PreparedCall::Control(flow));
            }
            dynamic_instances::PreparedTraitInstance::Unrelated => callee.as_ref(),
        };
        let this = self;
        let (interface_program, interface_parameters) =
            this.interface_call_contract(callee, expr.source)?;
        let dynamic_defaults = !matches!(
            callee.kind,
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
        );
        let callee_source = callee.source;
        // Static function identities carry no owned callee environment. A
        // synthesized trait-method node can share its enclosing call's source
        // with a handler-input temporary; retaining the function under that
        // source would replace the tuple's cleanup lookup with the wrong value.
        let retain_callee = !matches!(
            callee.kind,
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
        );
        let callee = prepared_value!(this.eval_in_position(callee, environment, true));
        this.check_transported_instance(&callee)?;
        // Spread preparation can retain an already-created instance in a local.
        // Anonymous closure environments have no declaration substitution.
        let dynamic_defaults = dynamic_defaults && callee.declaration_instance().is_some();
        if retain_callee {
            this.retain_residual(callee_source, &callee)?;
        }
        let mut operands = call_operands::CallOperands::open(
            this,
            &callee,
            interface_parameters.as_deref(),
            parameter_bindings,
            arguments.iter().any(|arg| arg.is_default),
            expr.source,
        )?;
        for argument in arguments {
            if let std::ops::ControlFlow::Break(flow) = operands.evaluate(
                this,
                expr.source,
                argument,
                environment,
                trait_default_inputs.as_ref(),
                dynamic_defaults.then_some(&callee),
                |vm, expr, env| vm.eval(expr, env),
            )? {
                return Ok(PreparedCall::Control(flow));
            }
        }
        let (arguments, mut default_scope) = operands.finish(this, expr.source)?;
        // The public interface domain is checked even when the chosen
        // implementation accepts a wider domain. Argument routing uses
        // declaration-side locals, independently of body parameter names.
        if !interface_program.is_empty() {
            let preparation_parameters = interface_parameters
                .as_deref()
                .ok_or_else(|| this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)))?;
            for (binding, value) in preparation_parameters.iter().zip(&arguments) {
                let observed = match value {
                    Value::Storage(storage) => this.read_owned_value(storage)?,
                    Value::ComptimeReference(reference) => {
                        this.read_value_reference(reference, expr.source)?
                    }
                    value => this.checkpoint_representation(value, expr.source)?,
                };
                default_scope.insert(*binding, observed);
            }
            for snapshot in &interface_program.entry_snapshots {
                if !preparation_parameters.contains(&snapshot.parameter) {
                    return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                }
                let value =
                    this.local_value(&default_scope, snapshot.parameter, Some(expr.source))?;
                default_scope.insert(snapshot.binding, value);
            }
            if !interface_program.postconditions.is_empty() {
                this.require_interface_exit_site(expr.source)?;
            }
            for condition in interface_program.preconditions {
                if this.cleanup_schedule.is_some() {
                    if !this.check_predicate_shape(&condition)? {
                        return Err(this.failure(ErrorKind::Type, Some(condition.source)));
                    }
                    let Flow::Value(Value::Bool(passed)) =
                        this.eval(&condition, &mut default_scope)?
                    else {
                        return Err(
                            this.failure(ErrorKind::InvalidControlFlow, Some(condition.source))
                        );
                    };
                    this.events.push(Event::CheckPredicate {
                        source: condition.source,
                        passed,
                    });
                    if !passed {
                        this.cleanup_site(
                            expr.source,
                            iir::cleanup::Boundary::InterfacePreconditionFailure,
                            environment,
                        )?;
                        return Err(this
                            .failure(ErrorKind::ProgramContractRejected, Some(condition.source)));
                    }
                    continue;
                }
                let check = Expr {
                    source: condition.source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::CheckPredicate(Box::new(condition)),
                };
                if !matches!(
                    this.eval(&check, &mut default_scope)?,
                    Flow::Value(Value::Unit)
                ) {
                    return Err(this.failure(ErrorKind::InvalidControlFlow, Some(check.source)));
                }
            }
        }
        let interface_exit = (!interface_program.postconditions.is_empty()).then(|| {
            Box::new(InterfaceExit {
                conditions: interface_program.postconditions,
                result_binding: interface_program.result_binding,
                environment: default_scope,
                result_slot: arguments.len(),
            })
        });
        let invocation_context = this.invocation_cleanup_context(&callee, expr.source)?;
        Ok(PreparedCall::Invoke {
            interface_exit,
            callee,
            arguments,
            context: invocation_context,
        })
    }

    #[inline(never)]
    fn eval_call(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.eval_call_with_effects(expr, environment, None)
    }

    fn eval_call_with_effects(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        effects: Option<Vec<effects::Entry>>,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let (callee, arguments, context, interface_exit) =
                match this.prepare_call(expr, environment)? {
                    PreparedCall::Invoke {
                        interface_exit,
                        callee,
                        arguments,
                        context,
                    } => (callee, arguments, context, interface_exit),
                    PreparedCall::Control(control) => return Ok(control),
                };
            this.transfer_prepared();
            // Callee selection and all actual/default arguments have completed
            // in the caller's environment. Only invocation receives the row map.
            let previous =
                effects.map(|row| std::mem::replace(&mut this.effect_state.environment, row));
            let invoked = this.invoke_value_scoped(callee, arguments, context);
            if let Some(previous) = previous {
                this.effect_state.environment = previous;
            }
            match invoked {
                Ok(value) => {
                    if let Some(exit) = interface_exit {
                        this.check_interface_exit(*exit, &value, expr.source, environment)?;
                    }
                    Ok(Flow::Value(value))
                }
                Err(error) => {
                    this.cleanup_effect_failure(&error, expr.source, environment)?;
                    Err(error)
                }
            }
        })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => code.fmt(f),
            Self::Optional { ty, value } => {
                write!(f, "(optional-value (type {ty}) ")?;
                match value {
                    Some(value) => write!(f, "(present {value}))"),
                    None => f.write_str("(absent))"),
                }
            }
            Self::Object(object) => object.fmt(f),
            Self::ErrorResult { ty, error, value } => {
                write!(f, "(error-result (type {ty}) (branch ")?;
                match error {
                    Some(index) => write!(f, "(error {index})")?,
                    None => f.write_str("success")?,
                };
                write!(f, ") (value {value}))")
            }
            Self::RuntimeTraitMethod(identity) => identity.fmt(f),
            Self::Newtype { ty, value } => {
                write!(f, "(newtype-value (type {ty}) (representation {value}))")
            }
            Self::FunctionInstance {
                declaration,
                arguments,
            } => {
                write!(f, "(function-instance {declaration} (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::Pointer(pointer) => pointer.fmt(f),
            Self::Storage(_) => f.write_str("(internal-storage)"),
            Self::ComptimeReference(reference) => reference.fmt(f),
            Self::Type(ty) => write!(f, "(type-value {ty})"),
            Self::NormalForm { term, ty } => {
                write!(f, "(normal-form-value (type {ty}) (term {term}))")
            }
            Self::Description(description) => description.fmt(f),
            Self::TypeParameter(id) => write!(f, "(abstract-type-input {id})"),
            Self::AbstractValue { declaration, ty } => {
                write!(f, "(abstract-value-input {declaration} (domain {ty}))")
            }
            Self::Integer(value) => value.fmt(f),
            Self::BitVec(value) => value.fmt(f),
            Self::Float(value) => write!(f, "(comptime-float {value})"),
            Self::Float16(bits) => Literal::Float16(*bits).fmt(f),
            Self::BFloat16(bits) => Literal::BFloat16(*bits).fmt(f),
            Self::Float32(bits) => Literal::Float32(*bits).fmt(f),
            Self::Float64(bits) => Literal::Float64(*bits).fmt(f),
            Self::SignedInteger(value) => write!(f, "(signed-integer {value})"),
            Self::UnsignedInteger(value) => write!(f, "(unsigned-integer {value})"),
            Self::Variant {
                declaration,
                arguments,
                index,
                fields,
            } => {
                write!(f, "(variant {declaration} {index} (type-arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str(") (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::AnonymousRecord(fields) => {
                f.write_str("(anonymous-record (fields")?;
                for (name, value) in fields {
                    write!(
                        f,
                        " (field {} {value})",
                        nir::program::Literal::String(name.clone())
                    )?;
                }
                f.write_str("))")
            }
            Self::Record {
                declaration,
                arguments,
                fields,
            } => {
                write!(f, "(record (declaration {declaration}) (type-arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str(") (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::Unit => f.write_str("(unit)"),
            Self::Bool(v) => write!(f, "(bool {v})"),
            Self::Char(v) => write!(f, "(char {})", *v as u32),
            Self::String(v) => nir::program::Literal::String(v.to_string()).fmt(f),
            Self::Symbol(v) => nir::program::Literal::Symbol(v.to_string()).fmt(f),
            Self::Function(id) => write!(f, "(function {id})"),
            Self::List { ty, values } => {
                write!(f, "(comptime-list (type {ty}) (values")?;
                for value in values {
                    write!(f, " {value}")?;
                }
                f.write_str("))")
            }
            Self::Tuple(values) => {
                f.write_str("(tuple")?;
                for value in values {
                    write!(f, " {value}")?;
                }
                f.write_str(")")
            }
            Self::IirCallable(handle) => {
                write!(f, "(iir-callable {} (type {}))", handle.key, handle.ty)
            }
            Self::Closure(closure) => {
                write!(f, "(closure (type {})", closure.ty)?;
                f.write_str(" (environment")?;
                for id in &closure.capture_order {
                    write!(f, " (binding {id} {})", closure.environment[id])?;
                }
                f.write_str(") (parameters")?;
                for parameter in &closure.parameters {
                    write!(f, " {parameter}")?;
                }
                write!(f, ") (body {}))", closure.body)
            }
        }
    }
}

#[cfg(test)]
mod pattern_budget_tests;
mod pattern_journal;

#[cfg(test)]
mod pattern_remap_tests;
mod pattern_structure;
mod type_patterns;

use pattern_journal::{PatternBindings, PatternJournal, PendingThen};

impl Interpreter<'_> {
    #[cfg(test)]
    fn match_pattern(
        &mut self,
        pattern: &nir::program::Pattern,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.match_pattern_with_bindings(pattern, value, environment)
            .map(|(flow, _)| flow)
    }

    #[allow(dead_code)]
    fn match_pattern_with_bindings(
        &mut self,
        pattern: &nir::program::Pattern,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<(Flow, PatternBindings), Error> {
        let (flow, bindings, _) =
            self.match_pattern_with_bindings_and_then(pattern, value, environment)?;
        Ok((flow, bindings))
    }

    fn match_pattern_with_bindings_and_then(
        &mut self,
        pattern: &nir::program::Pattern,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<(Flow, PatternBindings, Vec<PendingThen>), Error> {
        let mut journal = PatternJournal::new();
        let flow = self.match_pattern_node(pattern, value, environment, &mut journal)?;
        Ok((flow, journal.bindings, journal.pending_then))
    }

    fn match_pattern_node(
        &mut self,
        pattern: &nir::program::Pattern,
        value: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        self.tick(None)?;
        let mark = undo.len();
        let then_mark = undo.then_len();
        let result =
            self.with_native_depth(|vm| vm.match_pattern_inner(pattern, value, environment, undo));
        if !matches!(result, Ok(Flow::Value(Value::Bool(true)))) {
            undo.rollback(mark, environment);
            if undo.then_len() > then_mark {
                match &result {
                    Ok(_) => {
                        self.cleanup_pending_then(&mut undo.pending_then, then_mark, environment)?;
                    }
                    Err(_) => {
                        let _ = self.cleanup_pending_then(
                            &mut undo.pending_then,
                            then_mark,
                            environment,
                        );
                    }
                }
            }
        }
        result
    }

    fn match_pattern_inner(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        if matches!(input, Value::AbstractValue { .. })
            && !matches!(
                pattern,
                Pattern::Wild
                    | Pattern::Binding { .. }
                    | Pattern::Guard { .. }
                    | Pattern::Then { .. }
                    | Pattern::Remap { .. }
                    | Pattern::Or(..)
                    | Pattern::Observe { .. }
            )
        {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, None));
        }
        if input.lacks_pattern_shape()
            && !matches!(
                pattern,
                Pattern::Wild
                    | Pattern::Binding { .. }
                    | Pattern::Guard { .. }
                    | Pattern::Then { .. }
                    | Pattern::Remap { .. }
                    | Pattern::Or(..)
            )
        {
            return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
        }
        if let Some((shape, fields)) = nir::pattern_structure::View::from_pattern(pattern) {
            return self.match_value_structure(shape, fields, input, environment, undo);
        }
        if let Some((shape, fields)) = nir::pattern_structure::TypeStructure::from_pattern(pattern)
        {
            return self.match_type_structure(shape, fields, input, environment, undo);
        }
        let matched = match pattern {
            Pattern::CodeEvidence { .. } => {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
            }
            Pattern::OptionalAbsent => matches!(input, Value::Optional { value: None, .. }),
            Pattern::Wild => true,
            Pattern::Unit => matches!(input, Value::Unit),
            Pattern::Literal(literal) => {
                return self
                    .match_literal_value(literal, input)
                    .map(|matched| Flow::Value(Value::Bool(matched)));
            }
            Pattern::Observe { binding, inner } => {
                if environment.contains_key(binding) {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
                // This alias is not part of the rollback/export journal. Only
                // user bindings produced by the inner pattern can escape.
                environment.insert(*binding, input.clone());
                let result = self.match_pattern_node(inner, input, environment, undo);
                environment.remove(binding);
                return result;
            }
            Pattern::Then { .. } => return self.match_then(pattern, input, environment, undo),
            Pattern::Guard { .. } => return self.match_guard(pattern, input, environment, undo),
            Pattern::Binding { .. } => {
                return self.match_binding(pattern, input, environment, undo);
            }
            Pattern::Remap { .. } => return self.match_remap(pattern, input, environment, undo),
            Pattern::Or(..) => return self.match_or(pattern, input, environment, undo),
            Pattern::TypeTuple(_)
            | Pattern::NominalType { .. }
            | Pattern::NormalForm { .. }
            | Pattern::OptionalPresent(_)
            | Pattern::ErrorResult { .. }
            | Pattern::AnonymousRecord { .. }
            | Pattern::Tuple(_)
            | Pattern::Record { .. }
            | Pattern::Variant { .. } => unreachable!("value structure dispatched above"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_literal_value(&mut self, literal: &Literal, input: &Value) -> Result<bool, Error> {
        let matched = match literal {
            Literal::Description(_) => {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
            }
            Literal::PendingInteger { .. } | Literal::PendingFloat { .. } => {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
            }

            Literal::Integer(spelling) => {
                if let Value::Integer(actual) = input {
                    let expected = self.integer_literal(spelling, None)?;
                    self.integer_operation(None, |budget| {
                        budget.consume(
                            actual
                                .bit_len()
                                .max(expected.bit_len())
                                .div_ceil(32)
                                .saturating_add(1),
                        )?;
                        Ok(actual == &expected)
                    })?
                } else {
                    false
                }
            }
            Literal::BitVec { width, words } => {
                if let Value::BitVec(actual) = input {
                    self.integer_operation(None, |budget| {
                        budget.consume(actual.words().len().min(words.len()).saturating_add(1))
                    })?;
                    actual.width() == *width && actual.words() == words
                } else {
                    false
                }
            }
            Literal::Float(spelling) => {
                if let Value::Float(actual) = input {
                    self.integer_operation(None, |budget| {
                        let expected = comptime_runtime::decimal::DecimalValue::parse_literal(
                            spelling, budget,
                        )?;
                        Ok(actual.compare(&expected, budget)?.is_eq())
                    })?
                } else {
                    false
                }
            }
            Literal::Float16(expected) => {
                matches!(input, Value::Float16(actual) if comptime_runtime::float16::Float16Format::Binary16.compare(*actual, *expected).is_some_and(|o| o.is_eq()))
            }
            Literal::BFloat16(expected) => {
                matches!(input, Value::BFloat16(actual) if comptime_runtime::float16::Float16Format::BFloat16.compare(*actual, *expected).is_some_and(|o| o.is_eq()))
            }
            Literal::Float32(expected) => {
                matches!(input, Value::Float32(actual) if f32::from_bits(*actual) == f32::from_bits(*expected))
            }
            Literal::Float64(expected) => {
                matches!(input, Value::Float64(actual) if f64::from_bits(*actual) == f64::from_bits(*expected))
            }
            Literal::SignedInteger(expected) => {
                matches!(input, Value::SignedInteger(actual) if actual == expected)
            }
            Literal::UnsignedInteger(expected) => {
                matches!(input, Value::UnsignedInteger(actual) if actual == expected)
            }
            Literal::Bool(expected) => {
                matches!(input, Value::Bool(actual) if actual == expected)
            }
            Literal::Char(expected) => {
                matches!(input, Value::Char(actual) if actual == expected)
            }
            Literal::Symbol(expected) | Literal::String(expected) => {
                let actual = match (literal, input) {
                    (Literal::Symbol(_), Value::Symbol(actual))
                    | (Literal::String(_), Value::String(actual)) => Some(actual),
                    _ => None,
                };
                if let Some(actual) = actual {
                    self.integer_operation(None, |budget| {
                        budget.consume(actual.len().min(expected.len()).saturating_add(1))
                    })?;
                    actual.as_ref() == expected
                } else {
                    false
                }
            }
        };
        Ok(matched)
    }

    #[inline(never)]
    fn match_then(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Then { first, value, next } => {
                if !matches!(
                    value!(self.match_pattern_node(first, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    let value_source = value.source;
                    let computed = value!(self.eval(value, environment));
                    let pt_index = undo.pending_then.len();
                    undo.pending_then.push(PendingThen {
                        source: value_source,
                        value: computed.clone(),
                        bindings: PatternBindings::default(),
                    });
                    let mut then_journal = PatternJournal::new();
                    let result = match self.match_pattern_node(
                        next,
                        &computed,
                        environment,
                        &mut then_journal,
                    ) {
                        Ok(flow) => flow,
                        Err(error) => {
                            let _ = self.cleanup_pending_then(
                                &mut then_journal.pending_then,
                                0,
                                environment,
                            );
                            let _ = self.cleanup_pending_then(
                                &mut undo.pending_then,
                                pt_index,
                                environment,
                            );
                            return Err(error);
                        }
                    };
                    if !matches!(result, Flow::Value(Value::Bool(true))) {
                        self.cleanup_pending_then(&mut then_journal.pending_then, 0, environment)?;
                        self.cleanup_pending_then(&mut undo.pending_then, pt_index, environment)?;
                        return Ok(result);
                    }
                    let base_rank = undo.len();
                    let mut ranked = Vec::with_capacity(then_journal.bindings.1.len());
                    for (id, rank) in &then_journal.bindings.1 {
                        ranked.push((*rank, *id));
                    }
                    ranked.sort();
                    undo.pending_then[pt_index].bindings = then_journal.bindings;
                    let nested_then = then_journal.pending_then;
                    let mut seen_bindings = BTreeSet::new();
                    for (id, old_val, old_origin, old_rank) in then_journal.changes {
                        let first_change = seen_bindings.insert(id);
                        let effective_origin = if first_change {
                            undo.bindings.0.get(&id).cloned()
                        } else {
                            old_origin
                        };
                        let effective_rank = if first_change {
                            undo.bindings.1.get(&id).copied()
                        } else {
                            old_rank
                        };
                        undo.changes
                            .push((id, old_val, effective_origin, effective_rank));
                    }
                    for (offset, (_, id)) in ranked.into_iter().enumerate() {
                        undo.bindings.0.insert(id, None);
                        undo.bindings.1.insert(id, base_rank + offset);
                    }
                    undo.pending_then.extend(nested_then);
                    true
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_guard(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Guard { pattern, condition } => {
                if !matches!(
                    value!(self.match_pattern_node(pattern, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    match Self::concrete_value(
                        value!(self.eval(condition, environment)),
                        condition.source,
                    )? {
                        Value::Bool(value) => value,
                        _ => return Err(self.failure(ErrorKind::Type, Some(condition.source))),
                    }
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_binding(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Binding { binding, inner } => {
                if !matches!(
                    value!(self.match_pattern_node(inner, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    undo.insert(*binding, input.clone(), undo.path.clone(), environment);
                    true
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_remap(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Remap { pattern, bindings } => {
                let mut sources = BTreeSet::new();
                let mut targets = BTreeSet::new();
                for (from, to) in bindings {
                    self.tick(None)?;
                    if !sources.insert(*from) || !targets.insert(*to) {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                    }
                }
                let candidate_start = undo.len();
                let then_start = undo.then_len();
                if !matches!(
                    value!(self.match_pattern_node(pattern, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    // Only bindings produced by this successful candidate may
                    // be renamed. Ambient locals and bindings rolled back by
                    // an earlier alternative are not candidate exports.
                    let mut exports = BTreeSet::new();
                    for (id, _, _, _) in &undo.changes[candidate_start..] {
                        self.tick(None)?;
                        if environment.contains_key(id) && undo.bindings.0.contains_key(id) {
                            exports.insert(*id);
                        }
                    }
                    for source in &sources {
                        self.tick(None)?;
                        if !exports.contains(source) {
                            return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                        }
                    }
                    for target in &targets {
                        self.tick(None)?;
                        if exports.contains(target) && !sources.contains(target) {
                            return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                        }
                    }
                    let values = bindings
                        .iter()
                        .map(|(from, to)| {
                            let rank =
                                undo.bindings.1.get(from).copied().ok_or_else(|| {
                                    self.failure(ErrorKind::InvalidCallPlan, None)
                                })?;
                            environment
                                .get(from)
                                .cloned()
                                .map(|v| {
                                    (*to, v, undo.bindings.0.get(from).cloned().flatten(), rank)
                                })
                                .ok_or_else(|| self.failure(ErrorKind::MissingBinding, None))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    for (id, value, origin, rank) in values {
                        undo.insert_ranked(id, value, origin, rank, environment);
                    }
                    for (from, _) in bindings {
                        if !targets.contains(from) {
                            undo.restore_outer(*from, candidate_start, environment);
                        }
                    }
                    for pt in &mut undo.pending_then[then_start..] {
                        let mapped_bindings = bindings
                            .iter()
                            .filter_map(|(from, to)| {
                                let origin = pt.bindings.0.remove(from)?;
                                let rank = pt.bindings.1.remove(from)?;
                                Some((*to, origin, rank))
                            })
                            .collect::<Vec<_>>();
                        for (to, origin, rank) in mapped_bindings {
                            pt.bindings.0.insert(to, origin);
                            pt.bindings.1.insert(to, rank);
                        }
                    }
                    true
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_or(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Or(patterns) => {
                let mut found = false;
                for pattern in patterns {
                    if matches!(
                        value!(self.match_pattern_node(pattern, input, environment, undo)),
                        Value::Bool(true)
                    ) {
                        found = true;
                        break;
                    }
                }
                found
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    /// Invoke with an already evaluated value graph. Check every advertised
    /// domain before executing code, without reconstructing closure storage.
    pub fn invoke_with_typed_values(
        &mut self,
        function: DeclarationId,
        arguments: Vec<(Term, Value)>,
    ) -> Result<Value, Error> {
        self.invoke_with_prepared_arguments(
            function,
            arguments
                .into_iter()
                .map(|(ty, value)| (ty, InvocationValue::Live(value)))
                .collect(),
        )
    }

    /// Mix completed graph inputs with frozen inputs. Validate every live
    /// domain before running a normal-input factory or the target body.
    pub fn invoke_with_prepared_arguments(
        &mut self,
        function: DeclarationId,
        arguments: Vec<(Term, InvocationValue)>,
    ) -> Result<Value, Error> {
        let values = self.prepare_invocation_arguments(arguments)?;
        self.invoke(function, values)
    }

    fn prepare_invocation_arguments(
        &mut self,
        arguments: Vec<(Term, InvocationValue)>,
    ) -> Result<Vec<Value>, Error> {
        for (ty, value) in &arguments {
            if let InvocationValue::Live(value) = value {
                let mut fuel = usize::try_from(self.remaining_steps()).unwrap_or(usize::MAX);
                let before = fuel;
                let bounded = nir::type_computations::charge_term(ty, &mut fuel);
                self.charge_preparation(before - fuel)?;
                bounded.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                ty.check_scope(&[])
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
                if ty.sort() != nir::Sort::Type {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                if let Value::Storage(storage) = value {
                    // Prepared calls may retain owned roots between invocations.
                    // Validate the payload without copying the cell or turning a
                    // borrowed pointer into an owner. Check responsibility while
                    // borrowing, before any argument factory or body executes.
                    let contents = storage
                        .contents_for_copy(&mut self.budget.steps, 128)
                        .map_err(|kind| self.failure(kind, None))?;
                    self.check_value_type(contents.value(), ty, None)?;
                } else if let Value::ComptimeReference(reference) = value {
                    if !self.comptime_only {
                        return Err(self.failure(ErrorKind::PhaseMismatch, None));
                    }
                    let contents = reference
                        .borrow(&mut self.budget.steps)
                        .map_err(|kind| self.failure(kind, None))?;
                    self.check_value_type(contents.value(), ty, None)?;
                } else {
                    self.check_value_type(value, ty, None)?;
                }
            }
        }
        arguments
            .into_iter()
            .map(|(ty, value)| match value {
                InvocationValue::Live(value) => Ok(value),
                InvocationValue::Normal(term) => self.normal_pattern_value(&term, &ty),
            })
            .collect()
    }

    /// Invoke with typed, canonical normal-form inputs. Reuse the checked
    /// conversion used by normal-form patterns. Abstract Type descriptions need
    /// explicit scope authorization; they never become concrete runtime inputs.
    pub fn invoke_with_normal_arguments(
        &mut self,
        function: DeclarationId,
        arguments: Vec<(Term, Term)>,
    ) -> Result<Value, Error> {
        let values = arguments
            .iter()
            .map(|(ty, term)| self.normal_pattern_value(term, ty))
            .collect::<Result<Vec<_>, _>>()?;
        self.invoke(function, values)
    }

    fn list_element<'a>(&self, ty: &'a Term) -> Option<&'a Term> {
        let Term::NormalForm { head, .. } = ty else {
            return None;
        };
        self.normal_forms.get(head)?.list_element(ty)
    }

    fn tuple_input_elements<'a>(&self, ty: &'a Term) -> Option<&'a [Term]> {
        match ty {
            Term::Tuple(elements) => Some(elements),
            Term::NormalForm { head, .. } => self
                .normal_forms
                .get(head)
                .and_then(|metadata| metadata.tuple_elements(ty)),
            _ => None,
        }
    }
}

/// Bound shape inspection for a concrete type value. Scoped binders inside a
/// function type are legitimate; free runtime environments/value-store handles
/// and active rigid inputs require elaboration support outside this interpreter.
#[cfg(test)]
fn type_value_work(ty: &Term, heads: &impl declaration_index::HeadLookup) -> Option<usize> {
    scoped_type_value_work(ty, heads, &BTreeSet::new())
}

// Rigid Type inputs are descriptions, never permission to inspect their layout.
#[cfg(test)]
fn scoped_type_value_work(
    ty: &Term,
    heads: &impl declaration_index::HeadLookup,
    parameters: &BTreeSet<DeclarationId>,
) -> Option<usize> {
    scoped_type_value_work_with_evidence(ty, heads, parameters, &BTreeMap::new())
}

#[cfg(test)]
fn scoped_type_value_work_with_evidence(
    ty: &Term,
    heads: &impl declaration_index::HeadLookup,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
) -> Option<usize> {
    if ty.sort() != nir::Sort::Type {
        return None;
    }
    scoped_term_value_work_with_evidence(ty, heads, parameters, value_parameters)
}

// Instance identities contain completed value arguments as well as types.
// Use the same bounded structural validation without imposing a Type root.
#[cfg(test)]
fn instance_argument_work(
    argument: &Term,
    heads: &impl declaration_index::HeadLookup,
) -> Option<usize> {
    scoped_term_value_work_with_evidence(argument, heads, &BTreeSet::new(), &BTreeMap::new())
}

#[cfg(test)]
fn scoped_term_value_work_with_evidence(
    ty: &Term,
    heads: &impl declaration_index::HeadLookup,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
) -> Option<usize> {
    // Representation estimates count expanded work within the value-size limit.
    let mut fuel = 100_000;
    type_descriptions::inspect(
        ty,
        &[],
        heads,
        parameters,
        value_parameters,
        &BTreeMap::new(),
        None,
        |_, _, _| Err("nominal layout unavailable during structural inspection".into()),
        |_, _, _| Ok(None),
        None,
        &mut fuel,
    )
    .ok()?;
    Some(100_000 - fuel)
}

#[cfg(test)]
mod computation_type_tests {
    use super::*;

    #[test]
    fn normal_form_values_need_no_body_but_cannot_be_invoked() {
        let id = DeclarationId {
            package: 9,
            local: 1,
        };
        let metadata = nir::normal_forms::Head {
            collector: None,
            role: Default::default(),
            declaration: id,
            requirements: vec![],
            parameters: vec![],
        };
        let ty = metadata.callable_type().unwrap();
        let package = Package {
            normal_forms: vec![metadata],
            ..Default::default()
        };
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert!(
            vm.check_retained_value(&Value::Function(id), &ty, None)
                .is_ok()
        );
        assert!(
            vm.check_retained_value(
                &Value::Function(id),
                &ty.callable_signature().clone().with_comptime_phase(true),
                None
            )
            .is_err()
        );
        assert!(
            vm.check_retained_value(
                &Value::Function(DeclarationId {
                    package: 9,
                    local: 2
                }),
                &ty,
                None
            )
            .is_err()
        );
        assert!(matches!(
            vm.invoke_value(Value::Function(id), vec![])
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        ));
        assert!(
            vm.check_retained_value(&Value::Function(id), &ty, None)
                .is_ok()
        );
    }

    #[test]
    fn normal_form_callable_snapshots_round_trip_and_validate_identity() {
        let id = DeclarationId {
            package: 9,
            local: 1,
        };
        let head = nir::normal_forms::Head {
            collector: None,
            role: Default::default(),
            declaration: id,
            requirements: vec![],
            parameters: vec![],
        };
        let ty = head.callable_type().unwrap();
        let package = Package {
            normal_forms: vec![head],
            ..Default::default()
        };
        let frozen = Value::Function(id)
            .into_scoped_invocation_term(&mut comptime_runtime::Budget::default(), &BTreeSet::new())
            .unwrap();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.normal_pattern_value(&frozen, &ty).unwrap(),
            Value::Function(id)
        );
        let record_type = Term::Record(vec![("factory".into(), ty.clone())]);
        let record = Term::RecordValue(vec![("factory".into(), frozen.clone())]);
        let value = vm.normal_pattern_value(&record, &record_type).unwrap();
        assert_eq!(
            value
                .into_scoped_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::new()
                )
                .unwrap(),
            record
        );
        for bad in [
            Term::FunctionValue {
                declaration: DeclarationId {
                    package: 9,
                    local: 2,
                },
                arguments: vec![],
            },
            Term::FunctionValue {
                declaration: id,
                arguments: vec![Term::Primitive(Primitive::Bool)],
            },
        ] {
            assert!(vm.normal_pattern_value(&bad, &ty).is_err());
        }
        let ordinary = ty.callable_signature().clone().with_comptime_phase(true);
        assert!(vm.normal_pattern_value(&frozen, &ordinary).is_err());
        let mismatch = Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![nir::Parameter {
                    sort: nir::Sort::Type,
                    ty: Term::Primitive(Primitive::Type),
                }],
                result: Box::new(Term::Primitive(Primitive::Type)),
            }),
            None,
        );
        assert!(vm.normal_pattern_value(&frozen, &mismatch).is_err());
        assert_eq!(
            vm.normal_pattern_value(&frozen, &ty).unwrap(),
            Value::Function(id)
        );
        let mut exhausted = Interpreter::new(&package, Budget::default()).unwrap();
        exhausted.budget.steps = 0;
        assert!(matches!(
            exhausted
                .normal_pattern_value(&frozen, &ty)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        ));
    }

    #[test]
    fn callable_bound_value_domains_are_checked_without_admitting_foreign_or_later_inputs() {
        use nir::{Parameter, Sort};
        let id = DeclarationId {
            package: 9,
            local: 1,
        };
        let metadata = nir::normal_forms::Head {
            collector: None,
            role: Default::default(),
            declaration: id,
            requirements: vec![],
            parameters: vec![nir::normal_forms::Parameter {
                declaration: DeclarationId {
                    package: 9,
                    local: 2,
                },
                binding: LocalId { owner: 1, index: 0 },
                kind: nir::normal_forms::Kind::Value,
                ty: Term::Primitive(Primitive::Bool),
            }],
        };
        let heads = BTreeMap::from([(id, &metadata)]);
        let head = |depth, index| Term::NormalForm {
            head: id,
            sort: Sort::Type,
            arguments: vec![Term::Bound {
                depth,
                index,
                sort: Sort::ComptimeValue,
            }],
        };
        let parameter = Parameter {
            sort: Sort::ComptimeValue,
            ty: Term::Primitive(Primitive::Bool),
        };
        let valid = Term::Function {
            parameters: vec![parameter.clone()],
            result: Box::new(head(0, 0)),
        };
        assert!(type_value_work(&valid, &heads).is_some());
        let nested = Term::Function {
            parameters: vec![parameter.clone()],
            result: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(head(1, 0)),
            }),
        };
        assert!(type_value_work(&nested, &heads).is_some());
        for invalid in [
            Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ..parameter.clone()
                }],
                result: Box::new(head(0, 0)),
            },
            Term::Function {
                parameters: vec![Parameter {
                    ty: Term::Primitive(Primitive::U8),
                    ..parameter.clone()
                }],
                result: Box::new(head(0, 0)),
            },
            Term::Function {
                parameters: vec![parameter.clone()],
                result: Box::new(head(0, 1)),
            },
            Term::Function {
                parameters: vec![parameter.clone()],
                result: Box::new(head(1, 0)),
            },
            Term::Function {
                parameters: vec![
                    Parameter {
                        sort: Sort::Runtime,
                        ty: head(0, 1),
                    },
                    parameter,
                ],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        ] {
            assert!(type_value_work(&invalid, &heads).is_none(), "{invalid}");
        }
        assert!(type_value_work(&valid, &heads).is_some());
    }

    #[test]
    fn instance_argument_budget_accepts_completed_values_but_rejects_open_terms() {
        let heads = BTreeMap::new();
        let value = Term::Bool(true);
        assert!(type_value_work(&value, &heads).is_none());
        assert_eq!(instance_argument_work(&value, &heads), Some(1));
        assert!(instance_argument_work(&Term::Primitive(Primitive::Bool), &heads).is_some());
        let open = Term::Rigid {
            declaration: DeclarationId {
                package: 0,
                local: 1,
            },
            sort: nir::Sort::ComptimeValue,
        };
        assert!(instance_argument_work(&open, &heads).is_none());
        assert!(instance_argument_work(&Term::TupleValue(vec![value, open]), &heads).is_none());
        let oversized = Term::TupleValue(vec![Term::Bool(false); 100_000]);
        assert!(instance_argument_work(&oversized, &heads).is_none());
    }

    #[test]
    fn enum_name_rejects_forged_domains_before_evaluation() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 3, end: 9 };
        for ty in [
            Term::Primitive(Primitive::Symbol),
            Term::Nominal {
                declaration: DeclarationId {
                    package: 0,
                    local: 17,
                },
                arguments: vec![],
            },
        ] {
            let expr = Expr {
                ty: Term::Primitive(Primitive::Str),
                source,
                kind: K::EnumName(Box::new(Expr {
                    ty,
                    source,
                    kind: K::Local(LocalId {
                        owner: 17,
                        index: 1,
                    }),
                })),
            };
            assert_eq!(
                vm.eval(&expr, &mut Environment::new()).unwrap_err().kind,
                ErrorKind::Type
            );
        }
    }

    #[test]
    fn symbol_name_rejects_forged_types_before_reading_the_operand() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 3, end: 9 };
        let missing = LocalId {
            owner: 17,
            index: 1,
        };
        for (input, output) in [
            (Primitive::Symbol, Primitive::Bool),
            (Primitive::Str, Primitive::Str),
            (Primitive::Bool, Primitive::Str),
        ] {
            let expr = Expr {
                ty: Term::Primitive(output),
                source,
                kind: K::SymbolName(Box::new(Expr {
                    ty: Term::Primitive(input),
                    source,
                    kind: K::Local(missing),
                })),
            };
            assert_eq!(
                vm.eval(&expr, &mut Environment::new()).unwrap_err().kind,
                ErrorKind::Type
            );
        }
        let valid = Expr {
            ty: Term::Primitive(Primitive::Str),
            source,
            kind: K::SymbolName(Box::new(Expr {
                ty: Term::Primitive(Primitive::Symbol),
                source,
                kind: K::Literal(Literal::Symbol("ready".into())),
            })),
        };
        assert!(
            matches!(vm.eval(&valid,&mut Environment::new()).unwrap(),Flow::Value(Value::String(name)) if name.as_ref()=="ready")
        );
    }

    #[test]
    fn scoped_type_descriptions_do_not_admit_foreign_inputs_or_pending_operations() {
        let id = DeclarationId {
            package: 7,
            local: 1,
        };
        let parameters = BTreeSet::from([id]);
        let heads = BTreeMap::new();
        let rigid = Term::Rigid {
            declaration: id,
            sort: nir::Sort::Type,
        };
        let pointer = Term::Pointer(Box::new(rigid.clone()));
        assert!(scoped_type_value_work(&pointer, &heads, &parameters).is_some());
        assert!(type_value_work(&pointer, &heads).is_none());
        for invalid in [
            Term::Rigid {
                declaration: DeclarationId {
                    package: 7,
                    local: 2,
                },
                sort: nir::Sort::Type,
            },
            Term::Rigid {
                declaration: id,
                sort: nir::Sort::Runtime,
            },
            Term::Rigid {
                declaration: id,
                sort: nir::Sort::ComptimeValue,
            },
            Term::ListConcat {
                ty: Box::new(Term::NormalForm {
                    head: nir::DeclarationId {
                        package: 91,
                        local: 9,
                    },
                    sort: nir::Sort::Type,
                    arguments: vec![Term::Primitive(nir::Primitive::Type)],
                }),
                parts: vec![Term::TupleValue(vec![rigid.clone()])],
            },
        ] {
            let nested = Term::NormalForm {
                head: id,
                sort: nir::Sort::Type,
                arguments: vec![invalid],
            };
            assert!(scoped_type_value_work(&nested, &heads, &parameters).is_none());
        }
        let value = test_type_list(vec![pointer]);
        assert!(value.scoped_copy_work(&parameters).is_some());
        assert!(value.immutable_copy_work().is_none());
        assert!(
            value
                .clone()
                .into_normal_term(&mut comptime_runtime::Budget {
                    steps: 100,
                    max_limbs: 128
                })
                .is_err()
        );
        assert!(
            value
                .clone()
                .into_scoped_normal_term(
                    &mut comptime_runtime::Budget {
                        steps: 100,
                        max_limbs: 128
                    },
                    &parameters
                )
                .is_ok()
        );
        assert_eq!(
            value.into_scoped_normal_term(
                &mut comptime_runtime::Budget {
                    steps: 0,
                    max_limbs: 128
                },
                &parameters
            ),
            Err(ErrorKind::StepsExhausted)
        );
    }

    #[test]
    fn concrete_type_inspection_checks_effect_arguments_and_width() {
        let operation = |arguments| nir::effects::Operation {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            arguments,
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let closed = Term::computation(vec![operation(vec![boolean.clone()])], boolean.clone());
        assert!(type_value_work(&closed, &BTreeMap::new()).is_some());
        let open = Term::computation(
            vec![operation(vec![Term::Rigid {
                declaration: DeclarationId {
                    package: 1,
                    local: 2,
                },
                sort: nir::Sort::Type,
            }])],
            boolean.clone(),
        );
        assert!(type_value_work(&open, &BTreeMap::new()).is_none());
        let wide = Term::computation(vec![operation(vec![]); 100_001], boolean);
        assert!(type_value_work(&wide, &BTreeMap::new()).is_none());
    }
}

#[cfg(test)]
mod normal_input_data_tests {
    use super::*;

    #[test]
    fn nominal_layout_admission_keeps_failed_reservations_typed() {
        let declaration = DeclarationId {
            package: 2,
            local: 10,
        };
        let fields = vec![
            ("a".into(), Term::Primitive(Primitive::Bool)),
            ("b".into(), Term::Primitive(Primitive::Bool)),
            ("c".into(), Term::Primitive(Primitive::Bool)),
        ];
        let package = Package {
            structs: vec![nir::program::Struct {
                declaration,
                name: "Fields".into(),
                fields,
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                packed: nir::program::PackedMode::None,
            }],
            ..Default::default()
        };
        let vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let ty = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let mut fuel = 2;
        assert_eq!(
            vm.admitted_nominal_layout(&ty, None, &mut fuel),
            Err(nir::NominalAdmissionError::Budget)
        );
        assert!(
            fuel > 0,
            "the typed query must not depend on a zero sentinel"
        );
        assert!(matches!(
            vm.admitted_nominal_layout(&ty, Some(0), &mut 1000),
            Err(nir::NominalAdmissionError::Invalid(_))
        ));
        assert_eq!(
            vm.admitted_nominal_layout(&ty, None, &mut 1000).unwrap(),
            vec![Term::Primitive(Primitive::Bool); 3]
        );
    }

    #[test]
    fn clause_list_inputs_share_domain_scope_and_budget_validation() {
        let list_id = DeclarationId {
            package: 2,
            local: 1,
        };
        let parameter = DeclarationId {
            package: 2,
            local: 2,
        };
        let element = DeclarationId {
            package: 2,
            local: 3,
        };
        let package = Package {
            normal_forms: vec![nir::normal_forms::Head {
                declaration: list_id,
                role: nir::normal_forms::Role::TypeList,
                collector: None,
                requirements: vec![],
                parameters: vec![nir::normal_forms::Parameter {
                    declaration: element,
                    binding: LocalId { owner: 1, index: 0 },
                    kind: nir::normal_forms::Kind::Type,
                    ty: Term::Primitive(Primitive::Type),
                }],
            }],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let domain = Term::NormalForm {
            head: list_id,
            sort: nir::Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Type)],
        };
        Arc::make_mut(&mut vm.declaration_parameter_domains).insert(parameter, domain.clone());
        let nominal = |fields| Term::NominalValue {
            ty: Box::new(domain.clone()),
            variant: None,
            fields,
        };
        let check = |input: Term, frames: &[Vec<nir::Sort>], fuel: &mut usize| {
            vm.admitted_function_inputs(
                &[parameter],
                &[input],
                nir::DeclarationInputScope::Caller(frames),
                fuel,
            )
            .map(|_| ())
        };
        let assert_value_rejection = |result: Result<(), ErrorKind>| {
            let Err(ErrorKind::FunctionInputRejected(reason)) = result else {
                panic!("expected a value-domain rejection, got {result:?}");
            };
            assert_eq!(reason.index, 0);
            assert_eq!(reason.parameter, parameter);
            assert_eq!(reason.stage, InputAdmissionStage::Value);
            assert!(!reason.reason.is_empty());
        };
        for fields in [vec![], vec![Term::Primitive(Primitive::Bool)]] {
            assert_value_rejection(check(Term::TupleValue(fields.clone()), &[], &mut 10000));
            for input in [nominal(fields)] {
                assert!(check(input.clone(), &[], &mut 10000).is_ok());
                assert_eq!(
                    check(input.clone(), &[], &mut 0),
                    Err(ErrorKind::StepsExhausted)
                );
                assert!(check(input, &[], &mut 10000).is_ok());
            }
        }
        let bound = Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        };
        for input in [nominal(vec![bound])] {
            assert_eq!(check(input.clone(), &[], &mut 10000), Err(ErrorKind::Type));
            assert!(check(input.clone(), &[vec![nir::Sort::Type]], &mut 10000).is_ok());
            assert_eq!(
                check(input, &[vec![nir::Sort::ComptimeValue]], &mut 10000),
                Err(ErrorKind::Type)
            );
        }
        for input in [
            Term::TupleValue(vec![Term::Bool(true)]),
            nominal(vec![Term::Bool(true)]),
        ] {
            assert_value_rejection(check(input, &[], &mut 10000));
        }
    }

    #[test]
    fn freezing_named_callables_does_not_erase_captures_or_specialization() {
        let declaration = DeclarationId {
            package: 2,
            local: 3,
        };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Type)),
        }
        .with_comptime_phase(true);
        let closure = Closure {
            definition_source: None,
            parameters: vec![],
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: Term::Primitive(Primitive::Type),
                kind: K::Unit,
            }),
            ty: signature,
        };
        let freeze = |value: Value| {
            value.into_scoped_invocation_term(
                &mut comptime_runtime::Budget::default(),
                &BTreeSet::new(),
            )
        };
        assert_eq!(
            freeze(Value::Function(declaration)).unwrap(),
            Term::FunctionValue {
                declaration,
                arguments: vec![]
            }
        );
        let named = |arguments: Vec<Term>| Value::FunctionInstance {
            declaration,
            arguments,
        };
        let mut captured = closure.clone();
        let binding = LocalId { owner: 1, index: 0 };
        captured.environment.insert(binding, Value::Bool(true));
        captured.capture_order.push(binding);
        for argument in [
            Term::Bool(true),
            Term::TupleValue(vec![Term::Bool(false), Term::Primitive(Primitive::Bool)]),
            Term::TupleValue(vec![Term::Primitive(Primitive::Bool)]),
        ] {
            assert_eq!(
                freeze(named(vec![argument.clone()])).unwrap(),
                Term::FunctionValue {
                    declaration,
                    arguments: vec![argument]
                }
            );
        }
        for argument in [
            Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            },
            Term::TupleValue(vec![Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            }]),
            Term::ListConcat {
                ty: Box::new(Term::NormalForm {
                    head: nir::DeclarationId {
                        package: 91,
                        local: 9,
                    },
                    sort: nir::Sort::Type,
                    arguments: vec![Term::Primitive(nir::Primitive::Type)],
                }),
                parts: vec![Term::TupleValue(vec![])],
            },
        ] {
            assert_eq!(
                named(vec![argument]).into_scoped_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::from([declaration]),
                ),
                Err(ErrorKind::Type)
            );
        }
        let parameter = Term::Rigid {
            declaration,
            sort: nir::Sort::Type,
        };
        let scoped = named(vec![parameter.clone()]);
        assert_eq!(freeze(scoped.clone()), Err(ErrorKind::Type));
        assert_eq!(
            scoped
                .into_scoped_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::from([declaration]),
                )
                .unwrap(),
            Term::FunctionValue {
                declaration,
                arguments: vec![parameter]
            }
        );
        for unsupported in [captured, closure] {
            assert_eq!(
                freeze(Value::Tuple(vec![Value::Closure(Arc::new(
                    unsupported.clone()
                ))]))
                .unwrap_err(),
                ErrorKind::Unsupported
            );
            assert_eq!(
                freeze(Value::Closure(Arc::new(unsupported))).unwrap_err(),
                ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn invocation_aggregates_retain_named_callables_without_exporting_them_as_data() {
        let declaration = DeclarationId {
            package: 2,
            local: 3,
        };
        let value = Value::Tuple(vec![Value::AnonymousRecord(vec![(
            "callback".into(),
            Value::Function(declaration),
        )])]);
        let expected = Term::TupleValue(vec![Term::RecordValue(vec![(
            "callback".into(),
            Term::FunctionValue {
                declaration,
                arguments: vec![],
            },
        )])]);
        assert_eq!(
            value
                .clone()
                .into_normal_term(&mut comptime_runtime::Budget::default()),
            Err(ErrorKind::Unsupported)
        );
        assert_eq!(
            value.clone().into_scoped_invocation_term(
                &mut comptime_runtime::Budget {
                    steps: 1,
                    max_limbs: 4096
                },
                &BTreeSet::new()
            ),
            Err(ErrorKind::StepsExhausted)
        );
        assert_eq!(
            value
                .into_scoped_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::new()
                )
                .unwrap(),
            expected
        );
        let mut deep = Value::Function(declaration);
        for _ in 0..130 {
            deep = Value::Tuple(vec![deep]);
        }
        assert_eq!(
            deep.into_scoped_invocation_term(
                &mut comptime_runtime::Budget::default(),
                &BTreeSet::new()
            ),
            Err(ErrorKind::DepthExhausted)
        );
    }

    #[test]
    fn aggregate_snapshot_classification_is_bounded_and_excludes_executable_members() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let data = Value::AnonymousRecord(vec![(
            "nested".into(),
            Value::Tuple(vec![Value::Bool(true)]),
        )]);
        assert!(vm.normal_input_is_data(&data, source).unwrap());
        let executable = Value::Tuple(vec![
            data.clone(),
            Value::Function(DeclarationId {
                package: 1,
                local: 1,
            }),
        ]);
        assert!(!vm.normal_input_is_data(&executable, source).unwrap());
        let payload_type = Term::Primitive(Primitive::Bool);
        let optional = Value::Optional {
            ty: Term::Optional(Box::new(payload_type.clone())),
            value: Some(Box::new(executable)),
        };
        let tagged = Value::ErrorResult {
            ty: Term::error_result(vec![Term::Primitive(Primitive::Char)], payload_type),
            error: None,
            value: Box::new(optional),
        };
        assert!(!vm.normal_input_is_data(&tagged, source).unwrap());
        assert!(vm.normal_input_is_data(&data, source).unwrap());
        let mut deep = Value::Bool(true);
        for _ in 0..130 {
            deep = Value::Tuple(vec![deep]);
        }
        assert_eq!(
            vm.normal_input_is_data(&deep, source).unwrap_err().kind,
            ErrorKind::DepthExhausted
        );
        assert!(vm.normal_input_is_data(&data, source).unwrap());
        let mut small = Interpreter::new(
            &package,
            Budget {
                steps: 4,
                ..Budget::default()
            },
        )
        .unwrap();
        assert_eq!(
            small
                .normal_input_is_data(&Value::Tuple(vec![Value::Unit; 10]), source)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
    }
}

#[cfg(test)]
mod block_continuation_tests {
    use super::*;

    #[test]
    fn deep_block_scopes_preserve_transfers_and_errors() {
        let source = SourceRange { start: 3, end: 7 };
        let binding = LocalId { owner: 1, index: 0 };
        let target = LocalId { owner: 2, index: 0 };
        for mode in 0..4 {
            let mut body = Expr {
                ty: Term::Primitive(Primitive::Never),
                source,
                kind: match mode {
                    0 => K::Return(None),
                    1 => K::Break(target),
                    2 => K::Continue(target),
                    _ => K::SymbolName(Box::new(Expr {
                        ty: Term::Primitive(Primitive::Bool),
                        source,
                        kind: K::Literal(Literal::Bool(true)),
                    })),
                },
            };
            for index in 1..=200 {
                let statement = Expr {
                    ty: Term::Primitive(Primitive::Void),
                    source,
                    kind: K::Let {
                        binding: LocalId { owner: 1, index },
                        value: Box::new(Expr {
                            ty: Term::Primitive(Primitive::Bool),
                            source,
                            kind: K::Literal(Literal::Bool(true)),
                        }),
                    },
                };
                body = Expr {
                    ty: Term::Primitive(Primitive::Never),
                    source,
                    kind: if index % 2 == 0 {
                        K::Block {
                            statements: vec![statement],
                            result: Some(Box::new(body)),
                        }
                    } else {
                        K::CallPreparation {
                            statements: vec![statement],
                            result: Some(Box::new(body)),
                        }
                    },
                };
            }
            let package = Package::default();
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            environment.insert(binding, Value::Bool(false));
            let result = vm.eval(&body, &mut environment);
            match mode {
                0 => assert!(matches!(result.unwrap(), Flow::Return(Value::Unit))),
                1 => assert!(matches!(result.unwrap(), Flow::Break(id) if id == target)),
                2 => assert!(matches!(result.unwrap(), Flow::Continue(id) if id == target)),
                _ => assert_eq!(result.unwrap_err().kind, ErrorKind::Type),
            }
            assert_eq!(environment.len(), 1);
            assert_eq!(environment.get(&binding), Some(&Value::Bool(false)));
            assert_eq!(vm.depth, 0);
        }
    }

    #[test]
    fn deep_block_scopes_unwind_on_success_and_fuel_failure() {
        let boolean = Term::Primitive(Primitive::Bool);
        let source = SourceRange { start: 1, end: 2 };
        let literal = || Expr {
            ty: boolean.clone(),
            source,
            kind: K::Literal(Literal::Bool(true)),
        };
        let mut body = literal();
        for index in 1..=200 {
            let statement = Expr {
                ty: Term::Primitive(Primitive::Void),
                source,
                kind: K::Let {
                    binding: LocalId { owner: 1, index },
                    value: Box::new(literal()),
                },
            };
            body = Expr {
                ty: boolean.clone(),
                source,
                kind: if index % 2 == 0 {
                    K::Block {
                        statements: vec![statement],
                        result: Some(Box::new(body)),
                    }
                } else {
                    K::CallPreparation {
                        statements: vec![statement],
                        result: Some(Box::new(body)),
                    }
                },
            };
        }
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let binding = LocalId { owner: 1, index: 0 };
        let mut environment = Environment::new();
        environment.insert(binding, Value::Bool(false));
        assert!(matches!(
            vm.eval(&body, &mut environment).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(environment.len(), 1);
        assert_eq!(environment.get(&binding), Some(&Value::Bool(false)));
        assert_eq!(vm.depth, 0);
        vm.budget.steps = 64;
        assert_eq!(
            vm.eval(&body, &mut environment).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(environment.len(), 1);
        assert_eq!(vm.depth, 0);
        vm.budget.steps = 100_000;
        assert!(matches!(
            vm.eval(&body, &mut environment).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(environment.len(), 1);
    }
}

#[cfg(test)]
mod pointer_copy_budget_tests {
    use super::*;

    #[test]
    fn pointer_write_proof_uses_live_fuel_before_mutation_and_can_retry() {
        let ty = Term::Tuple(vec![Term::Primitive(Primitive::Bool)]);
        let storage = Storage::new(Value::Tuple(vec![Value::Bool(false)]));
        let binding = LocalId { owner: 1, index: 0 };
        let source = SourceRange { start: 1, end: 2 };
        let expr = Expr {
            ty: Term::Primitive(Primitive::Void),
            source,
            kind: K::WritePointer {
                pointer: Box::new(Expr {
                    ty: Term::Pointer(Box::new(ty.clone())),
                    source,
                    kind: K::Local(binding),
                }),
                value: Box::new(Expr {
                    ty: ty.clone(),
                    source,
                    kind: K::Tuple(vec![Expr {
                        ty: Term::Primitive(Primitive::Bool),
                        source,
                        kind: K::Literal(Literal::Bool(true)),
                    }]),
                }),
            },
        };
        let mut environment = Environment::new();
        environment.insert(binding, Value::Pointer(storage.pointer(ty, vec![])));
        let observed = std::cell::Cell::new(0);
        let permission = |_: &Term, remaining: &mut u64| {
            observed.set(*remaining);
            let allowed = *remaining >= 20;
            *remaining = remaining.saturating_sub(20);
            allowed
        };
        let package = Package::default();
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 10,
                depth: 128,
            },
        )
        .unwrap()
        .with_budgeted_pointer_copy_permission(&permission);
        let before_eval = vm.remaining_steps();
        assert_eq!(
            vm.eval(&expr, &mut environment).unwrap_err().kind,
            ErrorKind::Unsupported
        );
        assert!(
            observed.get() < 10,
            "operand evaluation must precede proof acquisition"
        );
        assert_eq!(vm.remaining_steps(), 0);
        assert_eq!(
            storage.read().unwrap(),
            Value::Tuple(vec![Value::Bool(false)])
        );
        assert_eq!(vm.depth, 0);
        let operand_cost = before_eval - observed.get();
        // Both tuple inspections must finish before the write commits.
        // With three visits left, the old tuple's final field is still unchecked.
        vm.budget.steps = operand_cost + 20 + 3;
        assert_eq!(
            vm.eval(&expr, &mut environment).unwrap_err(),
            Error {
                kind: ErrorKind::StepsExhausted,
                source: Some(source)
            }
        );
        assert_eq!(
            storage.read().unwrap(),
            Value::Tuple(vec![Value::Bool(false)])
        );
        assert_eq!(vm.remaining_steps(), 0);
        vm.budget.steps = 40;
        assert!(matches!(
            vm.eval(&expr, &mut environment).unwrap(),
            Flow::Value(Value::Unit)
        ));
        assert_eq!(vm.remaining_steps(), observed.get() - 20 - 4);
        assert_eq!(
            storage.read().unwrap(),
            Value::Tuple(vec![Value::Bool(true)])
        );
    }

    #[test]
    fn pointer_reads_charge_copy_work_and_recover_without_mutation() {
        let ty = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 64]);
        let original = Value::Tuple(vec![Value::Bool(true); 64]);
        let storage = Storage::new(original.clone());
        let binding = LocalId { owner: 1, index: 0 };
        let source = SourceRange { start: 1, end: 2 };
        let expr = Expr {
            ty: ty.clone(),
            source,
            kind: K::ReadPointer(Box::new(Expr {
                ty: Term::Pointer(Box::new(ty.clone())),
                source,
                kind: K::Local(binding),
            })),
        };
        let mut environment = Environment::new();
        environment.insert(binding, Value::Pointer(storage.pointer(ty, vec![])));
        let package = Package::default();
        // Resource permission is explicit; it does not waive copying limits.
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 8,
                depth: 128,
            },
        )
        .unwrap()
        .with_pointer_copy_permission(&|_| true);
        assert_eq!(
            vm.eval(&expr, &mut environment).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(storage.read().unwrap(), original);
        assert_eq!(vm.depth, 0);
        vm.budget.steps = 1000;
        assert!(
            matches!(vm.eval(&expr,&mut environment).unwrap(),Flow::Value(value) if value==original)
        );
        assert_eq!(storage.read().unwrap(), original);
        assert!(vm.remaining_steps() < 1000 - 64);
    }
}

mod description_evidence;
mod error_results;
mod list_construction;
mod newtypes;
mod optional;
mod type_constructors;
mod type_equality;

#[cfg(test)]
mod live_graph_tests {
    use super::*;

    #[test]
    fn owned_graph_visits_shared_storage_and_cycles_once_without_cloning() {
        let storage = Storage::new(Value::Unit);
        let nested = Storage::new(Value::Tuple(vec![
            Value::Storage(storage.clone()),
            Value::Function(DeclarationId {
                package: 3,
                local: 7,
            }),
        ]));
        *storage.0.lock().unwrap() = Value::Tuple(vec![
            Value::Storage(nested.clone()),
            Value::Storage(nested.clone()),
        ]);
        let root = Value::Storage(storage.clone());
        let mut cells = 0;
        let mut functions = Vec::new();
        let result = root.visit_owned_graph(&mut 100, &mut |value, _| {
            match value {
                Value::Storage(_) => cells += 1,
                Value::Function(id) => functions.push(*id),
                _ => {}
            }
            Ok(())
        });
        // Break the test-created ownership cycle even if an assertion fails.
        *storage.0.lock().unwrap() = Value::Unit;
        result.unwrap();
        assert_eq!(cells, 2);
        assert_eq!(
            functions,
            vec![DeclarationId {
                package: 3,
                local: 7
            }]
        );
    }

    #[test]
    fn owned_graph_budget_and_locked_storage_fail_without_waiting() {
        let storage = Storage::new(Value::Bool(true));
        let root = Value::Storage(storage.clone());
        let mut seen = 0;
        assert_eq!(
            root.visit_owned_graph(&mut 0, &mut |_, _| {
                seen += 1;
                Ok(())
            }),
            Err(ErrorKind::StepsExhausted)
        );
        assert_eq!(seen, 0);
        let lock = storage.0.lock().unwrap();
        assert_eq!(
            root.visit_owned_graph(&mut 10, &mut |_, _| Ok(())),
            Err(ErrorKind::InvalidPointer)
        );
        drop(lock);
        root.visit_owned_graph(&mut 10, &mut |value, _| {
            if matches!(value, Value::Bool(true)) {
                seen += 1;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 1);
        let mut deep = Value::Unit;
        for _ in 0..130 {
            deep = Value::Tuple(vec![deep]);
        }
        assert_eq!(
            deep.visit_owned_graph(&mut 1000, &mut |_, _| Ok(())),
            Err(ErrorKind::DepthExhausted)
        );
    }
}

#[cfg(test)]
mod invocation_environment_tests {
    use super::*;

    #[test]
    fn failed_capture_preparation_restores_frame_and_can_resume_with_fresh_budget() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let binding = LocalId { owner: 1, index: 0 };
        let closure = Value::Closure(Arc::new(Closure {
            definition_source: None,
            parameters: vec![],
            capture_order: vec![binding],
            environment: Environment::from([(binding, Value::Tuple(vec![Value::Bool(false); 64]))]),
            body: Arc::new(Expr {
                ty: Term::Primitive(Primitive::Bool),
                source,
                kind: K::Literal(Literal::Bool(true)),
            }),
            ty: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        }));
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 8,
                depth: 128,
            },
        )
        .unwrap();
        vm.effect_state.invocation = 7;
        vm.effect_state.resume = Some(9);
        assert_eq!(
            vm.invoke_value(closure.clone(), vec![]).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.effect_state.invocation, 7);
        assert_eq!(vm.effect_state.resume, Some(9));
        assert!(vm.cleanup_frame.is_none());
        assert!(vm.prepared_frames.is_empty());
        vm.budget.steps = 10_000;
        assert_eq!(vm.invoke_value(closure, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.effect_state.invocation, 7);
        assert_eq!(vm.effect_state.resume, Some(9));
    }
}

#[cfg(test)]
mod nominal_environment_tests {
    use super::*;
    use nir::program::{NormalFormArgument as A, NormalFormPreparation, Pattern};

    #[test]
    fn nominal_input_and_packet_bindings_are_removed_on_every_exit() {
        let id = |local| DeclarationId { package: 1, local };
        let local = |index| LocalId { owner: 0, index };
        let ty = |primitive| Term::Primitive(primitive);
        let source = SourceRange { start: 1, end: 2 };
        let type_value = || Expr {
            source,
            ty: ty(Primitive::Type),
            kind: K::TypeValue(ty(Primitive::Bool)),
        };
        let package = Package {
            structs: vec![nir::program::Struct {
                declaration: id(20),
                name: "Pair".into(),
                type_parameters: vec![id(21), id(22)],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![],
                packed: nir::program::PackedMode::None,
            }],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::from([(local(99), Value::Bool(true))]);
        for mode in 0..3 {
            let second = match mode {
                0 => A::Value(type_value()),
                1 => A::integer(source, "7".into(), false),
                _ => A::Value(Expr {
                    source,
                    ty: ty(Primitive::Never),
                    kind: K::Return(Some(Box::new(type_value()))),
                }),
            };
            let expression = Expr {
                source,
                ty: ty(if second.is_diverging() {
                    Primitive::Never
                } else {
                    Primitive::Type
                }),
                kind: K::NominalType {
                    declaration: id(20),
                    order: None,
                    bindings: Some(vec![local(1), local(2)]),
                    arguments: vec![
                        A::Value(Expr {
                            source,
                            ty: ty(Primitive::Type),
                            kind: K::Local(local(3)),
                        }),
                        second,
                    ],
                    preparations: vec![NormalFormPreparation {
                        before: 0,
                        expression: Expr {
                            source,
                            ty: ty(Primitive::Void),
                            kind: K::LetPattern {
                                pattern: Pattern::Tuple(vec![Pattern::Binding {
                                    binding: local(3),
                                    inner: Box::new(Pattern::Wild),
                                }]),
                                value: Box::new(Expr {
                                    source,
                                    ty: Term::Tuple(vec![ty(Primitive::Type)]),
                                    kind: K::Tuple(vec![type_value()]),
                                }),
                                guard: None,
                                otherwise: None,
                            },
                        },
                    }],
                },
            };
            for _ in 0..2 {
                let result = vm.eval(&expression, &mut environment);
                match mode {
                    0 => assert!(matches!(
                        result,
                        Ok(Flow::Value(Value::Type(Term::Nominal { .. })))
                    )),
                    1 => assert_eq!(result.unwrap_err().kind, ErrorKind::Type),
                    _ => assert!(matches!(
                        result,
                        Ok(Flow::Return(Value::Type(Term::Primitive(Primitive::Bool))))
                    )),
                }
                assert_eq!(environment.len(), 1);
                assert_eq!(environment.get(&local(99)), Some(&Value::Bool(true)));
            }
        }
    }
}

#[cfg(test)]
mod metered_type_solver_tests {
    use super::*;

    #[test]
    fn host_type_proof_spends_invocation_budget_and_cannot_replenish_it() {
        let package = Package::default();
        let predicate = nir::program::TypePredicate::Equal(
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::U8),
        );
        let calls = std::cell::Cell::new(0);
        let solver = |_: &nir::program::TypePredicate, remaining: &mut u64| {
            calls.set(calls.get() + 1);
            *remaining = remaining.saturating_sub(7);
            Ok(ContractOutcome::Unknown)
        };
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10,
                depth: 128,
            },
        )
        .unwrap()
        .with_budgeted_type_solver(&solver);
        let available = vm.remaining_steps();
        assert_eq!(
            vm.solve_type_predicate(&predicate),
            Ok(ContractOutcome::Unknown)
        );
        assert_eq!(vm.remaining_steps(), available - 7);
        vm.solve_type_predicate(&predicate).unwrap();
        assert_eq!(vm.remaining_steps(), 0);
        assert_eq!(
            vm.solve_type_predicate(&predicate),
            Err(ErrorKind::StepsExhausted)
        );
        assert_eq!(calls.get(), 2);
        let replenish = |_: &nir::program::TypePredicate, remaining: &mut u64| {
            *remaining = u64::MAX;
            Ok(ContractOutcome::Satisfied)
        };
        let mut fresh = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10,
                depth: 128,
            },
        )
        .unwrap()
        .with_budgeted_type_solver(&replenish);
        let available = fresh.remaining_steps();
        assert_eq!(
            fresh.solve_type_predicate(&predicate),
            Ok(ContractOutcome::Satisfied)
        );
        assert_eq!(fresh.remaining_steps(), available);
    }
}

#[cfg(test)]
mod not_spine_tests;

// Representation-only fixtures use an opaque list head. Invocation tests must
// instead use a head registered in their package so admission checks its domain.
#[cfg(test)]
fn test_type_list(types: Vec<nir::Term>) -> Value {
    Value::List {
        ty: nir::Term::NormalForm {
            head: nir::DeclarationId {
                package: 4294967294,
                local: 0,
            },
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
        },
        values: types.into_iter().map(Value::Type).collect(),
    }
}

#[cfg(test)]
mod predicate_shape_budget_tests {
    use super::*;

    #[test]
    fn tree_predicate_preparation_reports_resource_failure_before_evaluation() {
        let source = SourceRange { start: 70, end: 80 };
        let flag = || Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(true)),
        };
        let condition = Expr {
            kind: K::Block {
                statements: (0..100).map(|_| flag()).collect(),
                result: Some(Box::new(flag())),
            },
            ..flag()
        };
        let check = Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::CheckPredicate(Box::new(condition)),
        };
        let package = Package::default();
        let mut vm = Interpreter::for_comptime_tree(
            &package,
            Budget {
                steps: 10_000,
                depth: 128,
            },
        )
        .unwrap();
        vm.budget.steps = 20;
        let Err(error) = vm.eval_check_predicate(&check, &mut Default::default()) else {
            panic!("unmetered predicate check")
        };
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(source));
        assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 20);
        assert!(vm.events.is_empty());
        vm.budget.steps = 10_000;
        assert!(matches!(
            vm.eval_check_predicate(&check, &mut Default::default()),
            Ok(Flow::Value(Value::Unit))
        ));
        assert!(matches!(
            vm.events.as_slice(),
            [Event::CheckPredicate { passed: true, .. }]
        ));
    }

    #[test]
    fn bit_write_pointer_rejects_comptime_without_mutating_storage() {
        let source = SourceRange { start: 10, end: 20 };
        let mut runtime_budget = comptime_runtime::Budget::default();
        let initial =
            comptime_runtime::bitvec::BitVec::from_words(4, &[0b0101], &mut runtime_budget)
                .unwrap();
        let rhs =
            comptime_runtime::bitvec::BitVec::from_words(2, &[0b11], &mut runtime_budget).unwrap();
        let cell = Storage::new(Value::BitVec(initial.clone()));
        let ptr_local = LocalId { owner: 1, index: 0 };
        let bv4 = Term::Primitive(Primitive::BitVec(4));
        let bv2 = Term::Primitive(Primitive::BitVec(2));
        let expr = Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::BitWritePointer {
                pointer: Box::new(Expr {
                    source,
                    ty: Term::Pointer(Box::new(bv4.clone())),
                    kind: K::Local(ptr_local),
                }),
                selections: vec![nir::program::BitSelection::Static { start: 0, end: 2 }],
                value: Box::new(Expr {
                    source,
                    ty: bv2,
                    kind: K::Literal(Literal::BitVec {
                        width: 2,
                        words: rhs.words().to_vec(),
                    }),
                }),
            },
        };
        let package = Package::default();
        let mut vm = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
        let mut env = Environment::from([(ptr_local, Value::Pointer(cell.pointer(bv4, vec![])))]);
        assert_eq!(
            vm.eval(&expr, &mut env).unwrap_err().kind,
            ErrorKind::PhaseMismatch
        );
        assert_eq!(cell.read().unwrap(), Value::BitVec(initial));
    }
}
