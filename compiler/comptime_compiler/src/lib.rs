//! Linear invocation code. Unsupported operations are detected before execution.
//! Constants and default templates own bounded typed NIR copies; opcodes never evaluate trees.
pub mod bits;
pub mod closures;
pub mod leaf_inputs;
pub mod lists;
pub mod normal_types;
pub mod objects;
mod patterns;
pub mod scalar;
pub mod staged_inputs;
pub mod type_constructors;
use nir::program::{ArgumentPassing, Expr, ExprKind as K, FieldOwner, LocalId, SourceRange};
use nir::{Primitive, Term};
use scalar::{OwnedScalar, Scalar};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub type Register = usize;

#[derive(Debug)]
pub struct ErrorConversion {
    pub conversion: nir::error_results::Conversion,
    pub input: Term,
    pub output: Term,
}
impl fmt::Display for ErrorConversion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(error-conversion {} (input {}) (output {}))",
            self.conversion, self.input, self.output
        )
    }
}

#[derive(Debug)]
pub enum Op {
    BitOperand {
        value: Register,
        requires_bits: bool,
    },
    Bits {
        destination: Register,
        operation: bits::Operation,
        inputs: Vec<(Register, Term)>,
        output: Term,
    },
    BeginClosure {
        closure: Register,
        constructor: Arc<closures::Constructor>,
    },
    Capture {
        closure: Register,
        index: usize,
        value: Register,
    },
    Closure {
        closure: Register,
        destination: Register,
    },
    TypeInputPlan {
        destination: Register,
        expression: Arc<Expr>,
    },
    Stage(staged_inputs::Instruction),
    BeginContextualInput {
        destination: Register,
        expected: Arc<Term>,
        input: Arc<normal_types::Shape>,
    },
    BeginNormalType {
        destination: Register,
        constructor: Arc<normal_types::Constructor>,
    },
    PrepareNormalInput {
        destination: Register,
        index: usize,
        input: Arc<normal_types::Shape>,
    },
    AppendNormalContainer {
        constructor: Register,
        parent: Register,
        value: Register,
        spread: bool,
        diverges: bool,
    },
    FinishNormalContainer {
        constructor: Register,
        value: Register,
        diverges: bool,
    },
    NormalResult {
        destination: Register,
        value: Register,
        diverges: bool,
    },
    NormalLiteral {
        constructor: Register,
        destination: Register,
    },
    BindNormalInput {
        destination: Register,
        value: Register,
        diverges: bool,
    },
    FinishNormalType {
        destination: Register,
    },
    CheckListAppend {
        local: Register,
        ty: Term,
    },
    ListAppend {
        local: Register,
        suffix: Register,
        ty: Term,
    },
    PrepareListIndex(Arc<lists::Index>),
    CheckIndexedList {
        value: Register,
        contract: Arc<lists::Index>,
    },
    ListIndex {
        destination: Register,
        list: Register,
        index: Register,
    },
    LocalListIndex {
        destination: Register,
        contract: Arc<lists::Index>,
        list: Register,
        list_source: SourceRange,
        index: leaf_inputs::Selector,
    },
    PrepareList(Arc<lists::Construction>),
    List {
        destination: Register,
        list: Arc<lists::Construction>,
        values: Vec<Register>,
    },
    PrepareListOperation(Arc<lists::Operation>),
    CheckListOperand {
        value: Register,
        ty: Arc<Term>,
    },
    ListLength {
        destination: Register,
        value: Register,
        ty: Arc<Term>,
        local: bool,
        value_source: SourceRange,
    },
    ListConcat {
        destination: Register,
        left: Register,
        right: Register,
    },
    LocalObjectQuery {
        destination: Register,
        access: Arc<objects::ObjectAccess>,
        receiver: Register,
        receiver_source: SourceRange,
        selector: Option<leaf_inputs::Selector>,
    },
    PrepareTypeConstructor(Arc<type_constructors::Constructor>),
    CheckTypeConstructorInput {
        constructor: Arc<type_constructors::Constructor>,
        index: usize,
        value: Register,
    },
    TypeConstructor {
        destination: Register,
        constructor: Arc<type_constructors::Constructor>,
        inputs: Vec<Register>,
    },
    PrepareTypeEqual {
        ty: Arc<Term>,
        inputs: [Arc<Term>; 2],
    },
    TypeDescription(Register),
    TypeEqual {
        destination: Register,
        left: Register,
        right: Register,
    },
    PrepareObjectAccess(Arc<objects::ObjectAccess>),
    ObjectAccess {
        destination: Register,
        access: Arc<objects::ObjectAccess>,
        packet: Register,
    },
    PrepareObject(Arc<objects::ObjectConstructor>),
    CheckObjectEntry {
        object: Arc<objects::ObjectConstructor>,
        index: usize,
        value: Register,
    },
    Object {
        destination: Register,
        object: Arc<objects::ObjectConstructor>,
        fields: Vec<Register>,
    },
    Tuple {
        destination: Register,
        fields: Vec<Register>,
    },
    MatchPattern {
        destination: Register,
        value: Register,
        pattern: Arc<nir::program::Pattern>,
        bindings: Vec<(LocalId, Register)>,
    },
    NonExhaustiveMatch,
    InvalidControlFlow,
    CommitScope,

    PrepareOptional {
        ty: Arc<Term>,
        input: Option<(Arc<Term>, SourceRange)>,
    },
    Optional {
        destination: Register,
        ty: Arc<Term>,
        value: Option<Register>,
        value_source: SourceRange,
    },
    PrepareError(Arc<ErrorConversion>),
    ErrorConvert {
        destination: Register,
        value: Register,
        conversion: Arc<ErrorConversion>,
    },

    Variant {
        destination: Register,
        declaration: nir::DeclarationId,
        index: usize,
        ty: Term,
        fields: Vec<Register>,
    },

    Record {
        destination: Register,
        declaration: Option<nir::DeclarationId>,
        ty: Term,
        fields: Vec<Register>,
    },

    CheckBinding(Register),
    WritePlace {
        local: Register,
        value: Register,
        fields: Vec<(FieldOwner, usize)>,
    },
    Field {
        destination: Register,
        base: Register,
        local: bool,
        nominal: Option<nir::DeclarationId>,
        name: Option<String>,
        index: usize,
        ty: Term,
        base_source: SourceRange,
    },

    BeginCall {
        call: Register,
        callee: Register,
        callee_source: SourceRange,
        normal_form: Option<(Term, usize)>,
        parameter_bindings: Vec<LocalId>,
        prepare_defaults: bool,
    },
    CheckCallArgument {
        call: Register,
        parameter: usize,
        is_default: bool,
        passing: ArgumentPassing,
        argument_source: SourceRange,
    },
    Reference {
        destination: Register,
        base: Register,
        local: bool,
        fields: Vec<(FieldOwner, usize)>,
        rebind: bool,
    },
    DefaultArgument {
        call: Register,
        destination: Register,
        value: Arc<Expr>,
        dynamic: bool,
    },
    StoreCallArgument {
        call: Register,
        parameter: usize,
        value: Register,
    },
    Invoke {
        call: Register,
        destination: Register,
    },
    PrepareIntrinsic {
        tag: nir::intrinsics::Tag,
        binding: Option<Arc<nir::intrinsics::Checked>>,
        inputs: Vec<Term>,
        output: Term,
    },
    Intrinsic {
        destination: Register,
        binding: Option<Arc<nir::intrinsics::Checked>>,
        inputs: Vec<Register>,
    },
    Constant {
        destination: Register,
        value: Arc<Expr>,
    },
    PrepareScalar(Arc<OwnedScalar>),
    CheckScalarOperand {
        scalar: Arc<OwnedScalar>,
        value: Register,
    },
    Scalar {
        destination: Register,
        scalar: Arc<OwnedScalar>,
        left: Register,
        right: Option<Register>,
    },
    Unit(Register),
    EnterScope,
    LeaveScope,
    Unwind(usize),
    Read {
        destination: Register,
        local: Register,
    },
    Store {
        local: Register,
        value: Register,
        initialize: bool,
    },
    Not {
        destination: Register,
        value: Register,
        ty: Term,
    },
    Jump(usize),
    Branch {
        condition: Register,
        when: bool,
        target: usize,
    },
    Finish {
        value: Register,
        explicit: bool,
    },
}
#[derive(Debug)]
pub struct Instruction {
    pub source: SourceRange,
    pub op: Op,
}
#[derive(Debug)]
pub struct Program {
    instructions: Vec<Instruction>,
    locals: Vec<(LocalId, Register)>,
    registers: usize,
    storage_work: usize,
}
impl Program {
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
    pub fn locals(&self) -> &[(LocalId, Register)] {
        &self.locals
    }
    /// Conservative retention charge, including copied payloads and lowering work.
    /// These units bound storage; they are not a byte-size or timing measurement.
    pub fn storage_work(&self) -> usize {
        self.storage_work
    }
    pub fn registers(&self) -> usize {
        self.registers
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported(SourceRange),
    Budget(SourceRange),
    InvalidControl(SourceRange),
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(engine-code (registers {}) (locals", self.registers)?;
        for (local, slot) in &self.locals {
            write!(f, " ({local} {slot})")?;
        }
        write!(f, ") (instructions")?;
        for (pc, instruction) in self.instructions.iter().enumerate() {
            write!(
                f,
                " ({pc} (source {} {}) ",
                instruction.source.start, instruction.source.end
            )?;
            match &instruction.op {
                Op::BitOperand {
                    value,
                    requires_bits,
                } => write!(f, "(bit-operand %{value} (requires-bits {requires_bits}))")?,
                Op::Bits {
                    destination,
                    operation,
                    inputs,
                    output,
                } => {
                    write!(f, "(bits %{destination} {operation} (inputs")?;
                    for (slot, ty) in inputs {
                        write!(f, " (%{slot} {ty})")?;
                    }
                    write!(f, ") (output {output}))")?;
                }
                Op::BeginClosure {
                    closure,
                    constructor,
                } => write!(f, "(begin-closure %{closure} {constructor})")?,
                Op::Capture {
                    closure,
                    index,
                    value,
                } => write!(f, "(capture %{closure} (index {index}) (value %{value}))")?,
                Op::Closure {
                    closure,
                    destination,
                } => write!(f, "(closure %{destination} (prepared %{closure}))")?,
                Op::TypeInputPlan {
                    destination,
                    expression,
                } => write!(f, "(type-input-plan %{destination} {expression})")?,
                Op::CheckListAppend { local, ty } => {
                    write!(f, "(check-list-append %{local} {ty})")?
                }
                Op::ListAppend { local, suffix, ty } => {
                    write!(f, "(list-append %{local} %{suffix} {ty})")?
                }
                Op::PrepareListIndex(contract) => write!(f, "(prepare-list-index {contract})")?,
                Op::CheckIndexedList { value, contract } => {
                    write!(f, "(check-indexed-list %{value} {contract})")?
                }
                Op::ListIndex {
                    destination,
                    list,
                    index,
                } => write!(f, "(list-index %{destination} %{list} %{index})")?,
                Op::LocalListIndex {
                    destination,
                    contract,
                    list,
                    list_source,
                    index,
                } => write!(
                    f,
                    "(local-list-index %{destination} {contract} (list %{list} (source {} {})) (index {index}))",
                    list_source.start, list_source.end
                )?,
                Op::PrepareList(list) => write!(f, "(prepare-list {list})")?,
                Op::List {
                    destination,
                    list,
                    values,
                } => {
                    write!(f, "(list %{destination} {list} (values")?;
                    for value in values {
                        write!(f, " %{value}")?;
                    }
                    write!(f, "))")?;
                }
                Op::PrepareListOperation(operation) => {
                    write!(f, "(prepare-list-operation {operation})")?
                }
                Op::CheckListOperand { value, ty } => {
                    write!(f, "(check-list-operand %{value} {ty})")?
                }
                Op::ListLength {
                    destination,
                    value,
                    ty,
                    local,
                    value_source,
                } => write!(
                    f,
                    "(list-length %{destination} %{value} (type {ty}) (local {local}) (source {} {}))",
                    value_source.start, value_source.end
                )?,
                Op::ListConcat {
                    destination,
                    left,
                    right,
                } => write!(f, "(list-concat %{destination} %{left} %{right})")?,
                Op::LocalObjectQuery {
                    destination,
                    access,
                    receiver,
                    receiver_source,
                    selector,
                } => {
                    write!(
                        f,
                        "(local-object-query %{destination} {access} (receiver %{receiver} (source {} {})) (selector",
                        receiver_source.start, receiver_source.end
                    )?;
                    if let Some(selector) = selector {
                        write!(f, " {selector}")?;
                    }
                    write!(f, "))")?;
                }
                Op::Stage(operation) => write!(f, "{operation}")?,
                Op::BeginContextualInput {
                    destination,
                    expected,
                    input,
                } => write!(
                    f,
                    "(begin-contextual-input %{destination} (expected {expected}) {input})"
                )?,
                Op::BeginNormalType {
                    destination,
                    constructor,
                } => write!(f, "(begin-normal-type %{destination} {constructor})")?,
                Op::PrepareNormalInput {
                    destination,
                    index,
                    input,
                } => write!(f, "(prepare-normal-input %{destination} {index} {input})")?,
                Op::AppendNormalContainer {
                    constructor,
                    parent,
                    value,
                    spread,
                    diverges,
                } => write!(
                    f,
                    "(append-normal-container %{constructor} %{parent} %{value} (spread {spread}) (diverges {diverges}))"
                )?,
                Op::FinishNormalContainer {
                    constructor,
                    value,
                    diverges,
                } => write!(
                    f,
                    "(finish-normal-container %{constructor} %{value} (diverges {diverges}))"
                )?,
                Op::NormalResult {
                    destination,
                    value,
                    diverges,
                } => write!(
                    f,
                    "(normal-result %{destination} %{value} (diverges {diverges}))"
                )?,
                Op::NormalLiteral {
                    constructor,
                    destination,
                } => write!(f, "(normal-literal %{destination} %{constructor})")?,
                Op::BindNormalInput {
                    destination,
                    value,
                    diverges,
                } => write!(
                    f,
                    "(bind-normal-input %{destination} %{value} (diverges {diverges}))"
                )?,
                Op::FinishNormalType { destination } => {
                    write!(f, "(finish-normal-type %{destination})")?
                }
                Op::PrepareTypeConstructor(constructor) => {
                    write!(f, "(prepare-type-constructor {constructor})")?
                }
                Op::CheckTypeConstructorInput {
                    constructor,
                    index,
                    value,
                } => write!(
                    f,
                    "(check-type-constructor-input {constructor} {index} %{value})"
                )?,
                Op::TypeConstructor {
                    destination,
                    constructor,
                    inputs,
                } => {
                    write!(f, "(construct-type %{destination} {constructor} (inputs")?;
                    for input in inputs {
                        write!(f, " %{input}")?;
                    }
                    write!(f, "))")?;
                }
                Op::PrepareTypeEqual { ty, inputs } => write!(
                    f,
                    "(prepare-type-equal (type {ty}) (left {}) (right {}))",
                    inputs[0], inputs[1]
                )?,
                Op::TypeDescription(value) => write!(f, "(type-description %{value})")?,
                Op::TypeEqual {
                    destination,
                    left,
                    right,
                } => write!(f, "(type-equal %{destination} %{left} %{right})")?,
                Op::PrepareObjectAccess(access) => write!(f, "(prepare-object-access {access})")?,
                Op::ObjectAccess {
                    destination,
                    access,
                    packet,
                } => {
                    write!(f, "(object-access %{destination} {access} %{packet})")?;
                }
                Op::PrepareObject(object) => write!(f, "(prepare-object {object})")?,
                Op::CheckObjectEntry {
                    object,
                    index,
                    value,
                } => write!(f, "(check-object-entry {index} {value} {object})")?,
                Op::Object {
                    destination,
                    object,
                    fields,
                } => {
                    write!(f, "(object {destination} {object} (inputs")?;
                    for field in fields {
                        write!(f, " {field}")?;
                    }
                    write!(f, "))")?;
                }
                Op::Tuple {
                    destination,
                    fields,
                } => {
                    write!(f, "(tuple {destination} (inputs")?;
                    for field in fields {
                        write!(f, " {field}")?;
                    }
                    write!(f, "))")?;
                }
                Op::MatchPattern {
                    destination,
                    value,
                    pattern,
                    bindings,
                } => {
                    write!(
                        f,
                        "(match-pattern {destination} {value} {pattern} (bindings"
                    )?;
                    for (id, slot) in bindings {
                        write!(f, " (binding {id} {slot})")?;
                    }
                    write!(f, "))")?;
                }
                Op::NonExhaustiveMatch => write!(f, "(non-exhaustive-match)")?,
                Op::InvalidControlFlow => write!(f, "(invalid-control-flow)")?,
                Op::CommitScope => write!(f, "(commit-scope)")?,
                Op::PrepareOptional { ty, input } => {
                    write!(f, "(prepare-optional {ty} (input")?;
                    if let Some((ty, at)) = input {
                        write!(f, " {ty} (source {} {})", at.start, at.end)?;
                    }
                    write!(f, "))")?;
                }
                Op::Optional {
                    destination,
                    ty,
                    value,
                    value_source,
                } => {
                    write!(f, "(optional {destination} {ty} (input")?;
                    if let Some(value) = value {
                        write!(f, " {value}")?;
                    }
                    write!(
                        f,
                        ") (value-source {} {}))",
                        value_source.start, value_source.end
                    )?;
                }
                Op::PrepareError(conversion) => write!(f, "(prepare-error {conversion})")?,
                Op::ErrorConvert {
                    destination,
                    value,
                    conversion,
                } => write!(f, "(error-convert {destination} {value} {conversion})")?,

                Op::Variant {
                    destination,
                    declaration,
                    index,
                    ty,
                    fields,
                } => {
                    write!(
                        f,
                        "(variant {destination} (declaration {declaration}) (index {index}) (type {ty}) (fields"
                    )?;
                    for field in fields {
                        write!(f, " {field}")?;
                    }
                    write!(f, "))")?;
                }

                Op::Record {
                    destination,
                    declaration,
                    ty,
                    fields,
                } => {
                    write!(f, "(record {destination} (declaration")?;
                    if let Some(declaration) = declaration {
                        write!(f, " {declaration}")?;
                    }
                    write!(f, ") (type {ty}) (fields")?;
                    for field in fields {
                        write!(f, " {field}")?;
                    }
                    write!(f, "))")?;
                }

                Op::CheckBinding(local) => write!(f, "(check-binding {local})")?,
                Op::WritePlace {
                    local,
                    value,
                    fields,
                } => {
                    write!(f, "(write-place {local} {value} (fields")?;
                    for (owner, index) in fields {
                        write!(f, " (field {owner} {index})")?;
                    }
                    write!(f, "))")?;
                }
                Op::Field {
                    destination,
                    base,
                    local,
                    nominal,
                    name,
                    index,
                    ty,
                    base_source,
                } => {
                    write!(f, "(field {destination} {base} (local {local}) (owner")?;
                    if let Some(owner) = nominal {
                        write!(f, " {owner}")?;
                    }
                    write!(f, ") (name")?;
                    if let Some(name) = name {
                        write!(f, " {:?}", name)?;
                    }
                    write!(
                        f,
                        ") (index {index}) (type {ty}) (base-source {} {}))",
                        base_source.start, base_source.end
                    )?;
                }

                Op::BeginCall {
                    call,
                    callee,
                    callee_source,
                    normal_form,
                    parameter_bindings,
                    prepare_defaults,
                } => {
                    write!(
                        f,
                        "(begin-call {call} {callee} (callee-source {} {}) (prepare-defaults {prepare_defaults}) (parameters",
                        callee_source.start, callee_source.end
                    )?;
                    for binding in parameter_bindings {
                        write!(f, " {binding}")?;
                    }
                    write!(f, ")")?;
                    if let Some((ty, count)) = normal_form {
                        write!(f, " (normal-form (type {ty}) (inputs {count}))")?;
                    }
                    write!(f, ")")?;
                }
                Op::CheckCallArgument {
                    call,
                    parameter,
                    is_default,
                    passing,
                    argument_source,
                } => write!(
                    f,
                    "(check-call-argument {call} {parameter} {is_default} {passing} (argument-source {} {}))",
                    argument_source.start, argument_source.end
                )?,
                Op::Reference {
                    destination,
                    base,
                    local,
                    fields,
                    rebind,
                } => {
                    write!(
                        f,
                        "(value-reference {destination} (base {base} (local {local})) (fields"
                    )?;
                    for (owner, index) in fields {
                        write!(f, " (field {owner} {index})")?;
                    }
                    write!(f, ") (rebind {rebind}))")?;
                }
                Op::DefaultArgument {
                    call,
                    destination,
                    value,
                    dynamic,
                } => write!(
                    f,
                    "(default-argument {call} {destination} (dynamic {dynamic}) {value})"
                )?,
                Op::StoreCallArgument {
                    call,
                    parameter,
                    value,
                } => write!(f, "(store-call-argument {call} {parameter} {value})")?,
                Op::Invoke { call, destination } => write!(f, "(invoke {call} {destination})")?,
                Op::PrepareIntrinsic {
                    tag,
                    binding,
                    inputs,
                    output,
                } => {
                    write!(f, "(prepare-intrinsic {tag} (binding")?;
                    if let Some(binding) = binding {
                        write!(f, " {binding}")?;
                    }
                    write!(f, ") (inputs")?;
                    for input in inputs {
                        write!(f, " {input}")?;
                    }
                    write!(f, ") (output {output}))")?;
                }
                Op::Intrinsic {
                    destination,
                    binding,
                    inputs,
                } => {
                    write!(f, "(intrinsic {destination} (binding")?;
                    if let Some(binding) = binding {
                        write!(f, " {binding}")?;
                    }
                    write!(f, ") (inputs")?;
                    for input in inputs {
                        write!(f, " {input}")?;
                    }
                    write!(f, "))")?;
                }
                Op::Constant { destination, value } => {
                    write!(f, "(constant {destination} {value})")?
                }
                Op::EnterScope => write!(f, "(enter-scope)")?,
                Op::LeaveScope => write!(f, "(leave-scope)")?,
                Op::Unwind(depth) => write!(f, "(unwind {depth})")?,
                Op::PrepareScalar(scalar) => write!(f, "(prepare-scalar {scalar})")?,
                Op::CheckScalarOperand { scalar, value } => {
                    write!(f, "(check-scalar-operand {value} {scalar})")?
                }
                Op::Scalar {
                    destination,
                    scalar,
                    left,
                    right,
                } => {
                    write!(f, "(scalar {destination} {scalar} (inputs {left}")?;
                    if let Some(right) = right {
                        write!(f, " {right}")?;
                    }
                    write!(f, "))")?;
                }
                Op::Unit(dst) => write!(f, "(unit {dst})")?,
                Op::Read { destination, local } => write!(f, "(read {destination} {local})")?,
                Op::Store {
                    local,
                    value,
                    initialize,
                } => write!(f, "(store {local} {value} {initialize})")?,
                Op::Not {
                    destination,
                    value,
                    ty,
                } => write!(f, "(not {destination} {value} {ty})")?,
                Op::Jump(target) => write!(f, "(jump {target})")?,
                Op::Branch {
                    condition,
                    when,
                    target,
                } => write!(f, "(branch {condition} {when} {target})")?,
                Op::Finish { value, explicit } => write!(f, "(finish {value} {explicit})")?,
            }
            write!(f, ")")?;
        }
        write!(f, "))")
    }
}

enum Work<'a> {
    Expression(&'a Expr, Register),
    Emit(SourceRange, Op),
    Label(usize),
    EndLoop(LocalId),
    BeginScope(SourceRange),
    EndScope(SourceRange),
}

/// Compile structured control using a heap worklist, not host recursion. Fuel
/// bounds traversal and register/code allocation, including unreachable code.
pub fn compile(body: &Expr, fuel: &mut usize) -> Result<Program, Error> {
    compile_root(Root::Expression(body), fuel).map(|(program, _)| program)
}

/// Compile operands while retaining contextual literal/container placeholders.
/// The returned shape uses stable register keys, never borrowed NIR addresses.
pub fn compile_staged_input(
    input: &nir::program::NormalFormArgument,
    fuel: &mut usize,
) -> Result<(Program, normal_types::Shape), Error> {
    compile_root(Root::Staged(input), fuel).map(|(program, shape)| (program, shape.unwrap()))
}

/// Compile an input with a known domain, prechecking its entire contextual
/// shape before any operand executes. This constructs a value, not an NF head.
pub fn compile_contextual_input(
    input: &nir::program::NormalFormArgument,
    expected: &Term,
    fuel: &mut usize,
) -> Result<Program, Error> {
    compile_root(Root::Contextual(input, expected), fuel).map(|(program, _)| program)
}

enum Root<'a> {
    Expression(&'a Expr),
    Staged(&'a nir::program::NormalFormArgument),
    Contextual(&'a nir::program::NormalFormArgument, &'a Term),
}
fn compile_root(
    root: Root<'_>,
    fuel: &mut usize,
) -> Result<(Program, Option<normal_types::Shape>), Error> {
    let source = match root {
        Root::Expression(body) => body.source,
        Root::Staged(input) | Root::Contextual(input, _) => input.source(),
    };
    let initial_fuel = *fuel;
    let mut instructions = Vec::new();
    let mut locals = BTreeMap::new();
    let mut registers = 1;
    let mut labels = Vec::new();
    let mut loops = BTreeMap::new();
    let mut scope_depth = 0;
    let (mut work, shape) = match root {
        Root::Expression(body) => (
            vec![
                Work::Emit(
                    source,
                    Op::Finish {
                        value: 0,
                        explicit: false,
                    },
                ),
                Work::Expression(body, 0),
            ],
            None,
        ),
        Root::Contextual(input, expected) => {
            let mut fresh = || {
                let slot = registers;
                registers += 1;
                slot
            };
            let (shape, code) = normal_types::compile_input(
                input,
                0,
                &mut fresh,
                fuel,
                &mut labels,
                &mut locals,
                false,
            )?;
            let mut work = vec![
                Work::Emit(
                    source,
                    Op::Finish {
                        value: 0,
                        explicit: false,
                    },
                ),
                Work::Emit(
                    source,
                    Op::NormalResult {
                        destination: 0,
                        value: shape.slot,
                        diverges: shape.diverges,
                    },
                ),
            ];
            work.extend(code.into_iter().rev());
            work.push(Work::Emit(
                source,
                Op::BeginContextualInput {
                    destination: 0,
                    expected: Arc::new(copy_term(expected, fuel, source)?),
                    input: Arc::new(shape),
                },
            ));
            (work, None)
        }
        Root::Staged(input) => {
            let mut fresh = || {
                let slot = registers;
                registers += 1;
                slot
            };
            let (shape, code) = normal_types::compile_input(
                input,
                0,
                &mut fresh,
                fuel,
                &mut labels,
                &mut locals,
                true,
            )?;
            let code = staged_inputs::lower(code, &shape, fuel)?;
            let mut work = vec![Work::Emit(
                source,
                Op::Stage(staged_inputs::Instruction::Finish {
                    value: shape.slot,
                    diverges: shape.diverges,
                }),
            )];
            work.extend(code.into_iter().rev());
            (work, Some(shape))
        }
    };
    while let Some(task) = work.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(source))?;
        match task {
            Work::Emit(source, op) => instructions.push(Instruction { source, op }),
            Work::Label(label) => labels[label] = Some(instructions.len()),
            Work::EndLoop(id) => {
                loops.remove(&id);
            }
            Work::BeginScope(source) => {
                scope_depth += 1;
                instructions.push(Instruction {
                    source,
                    op: Op::EnterScope,
                });
            }
            Work::EndScope(source) => {
                scope_depth -= 1;
                instructions.push(Instruction {
                    source,
                    op: Op::LeaveScope,
                });
            }
            Work::Expression(expr, dst) => {
                let source = expr.source;
                let mut fresh = || {
                    let slot = registers;
                    registers += 1;
                    slot
                };
                let emit = |work: &mut Vec<_>, op| work.push(Work::Emit(source, op));
                if let Some((scalar, left, right)) = Scalar::expression(expr) {
                    let scalar = Arc::new(OwnedScalar::copy(scalar, fuel, source)?);
                    let a = fresh();
                    let b = right.map(|_| fresh());
                    emit(
                        &mut work,
                        Op::Scalar {
                            destination: dst,
                            scalar: scalar.clone(),
                            left: a,
                            right: b,
                        },
                    );
                    if let Some((right, b)) = right.zip(b) {
                        emit(
                            &mut work,
                            Op::CheckScalarOperand {
                                scalar: scalar.clone(),
                                value: b,
                            },
                        );
                        work.push(Work::Expression(right, b));
                    }
                    emit(
                        &mut work,
                        Op::CheckScalarOperand {
                            scalar: scalar.clone(),
                            value: a,
                        },
                    );
                    work.push(Work::Expression(left, a));
                    emit(&mut work, Op::PrepareScalar(scalar));
                    continue;
                }
                match &expr.kind {
                    K::BitSlice { .. }
                    | K::BitIndex(..)
                    | K::BitWindow { .. }
                    | K::BitConcat(..)
                    | K::BitEqual(..)
                    | K::BitConvert(..)
                    | K::TakeBits(..) => {
                        use bits::Operation as B;
                        let (operation, values): (_, Vec<&Expr>) = match &expr.kind {
                            K::BitSlice { value, start, end } => (
                                B::Slice {
                                    start: *start,
                                    end: *end,
                                },
                                vec![value],
                            ),
                            K::BitIndex(value, index) => (B::Index, vec![value, index]),
                            K::BitWindow { value, start, end } => {
                                (B::Window, vec![value, start, end])
                            }
                            K::BitConcat(left, right) => (B::Concat, vec![left, right]),
                            K::BitEqual(left, right) => (B::Equal, vec![left, right]),
                            K::BitConvert(value) => (B::Convert, vec![value]),
                            K::TakeBits(value) => (B::Take, vec![value]),
                            _ => unreachable!(),
                        };
                        for ty in values
                            .iter()
                            .map(|value| &value.ty)
                            .chain(std::iter::once(&expr.ty))
                        {
                            nir::type_computations::charge_term(ty, fuel)
                                .map_err(|_| Error::Budget(source))?;
                        }
                        let slots: Vec<_> = values.iter().map(|_| fresh()).collect();
                        emit(
                            &mut work,
                            Op::Bits {
                                destination: dst,
                                operation,
                                inputs: slots
                                    .iter()
                                    .zip(&values)
                                    .map(|(slot, value)| (*slot, value.ty.clone()))
                                    .collect(),
                                output: expr.ty.clone(),
                            },
                        );
                        for (index, (value, slot)) in values.iter().zip(slots).enumerate().rev() {
                            emit(
                                &mut work,
                                Op::BitOperand {
                                    value: slot,
                                    requires_bits: operation.requires_bits(index),
                                },
                            );
                            work.push(Work::Expression(value, slot));
                        }
                    }
                    K::Closure {
                        parameters,
                        captures,
                        body,
                    } => {
                        nir::copy_budget::charge_substitution(expr, &[], &[], fuel)
                            .map_err(|error| copy_error(error, source))?;
                        let closure = fresh();
                        emit(
                            &mut work,
                            Op::Closure {
                                closure,
                                destination: dst,
                            },
                        );
                        for (index, capture) in captures.iter().enumerate().rev() {
                            let value = fresh();
                            emit(
                                &mut work,
                                Op::Capture {
                                    closure,
                                    index,
                                    value,
                                },
                            );
                            if let Some(expression) = &capture.value {
                                work.push(Work::Expression(expression, value));
                            } else {
                                let local =
                                    *locals.entry(capture.binding).or_insert_with(&mut fresh);
                                emit(
                                    &mut work,
                                    Op::Read {
                                        destination: value,
                                        local,
                                    },
                                );
                            }
                        }
                        emit(
                            &mut work,
                            Op::BeginClosure {
                                closure,
                                constructor: Arc::new(closures::Constructor {
                                    ty: expr.ty.clone(),
                                    parameters: parameters.clone(),
                                    captures: captures
                                        .iter()
                                        .map(|capture| (capture.binding, capture.ty.clone()))
                                        .collect(),
                                    body: body.clone(),
                                }),
                            },
                        );
                    }
                    K::Call {
                        callee,
                        arguments,
                        parameter_bindings,
                    } if !callee.ty.is_normal_form_callable() || parameter_bindings.is_empty() => {
                        *fuel = fuel
                            .checked_sub(arguments.len() + parameter_bindings.len())
                            .ok_or(Error::Budget(source))?;
                        let call = fresh();
                        let target = fresh();
                        emit(
                            &mut work,
                            Op::Invoke {
                                call,
                                destination: dst,
                            },
                        );
                        for argument in arguments.iter().rev() {
                            let value = fresh();
                            work.push(Work::Emit(
                                if callee.ty.is_normal_form_callable() {
                                    argument.value.source
                                } else {
                                    source
                                },
                                Op::StoreCallArgument {
                                    call,
                                    parameter: argument.parameter,
                                    value,
                                },
                            ));
                            if argument.is_default {
                                nir::copy_budget::charge_substitution(
                                    &argument.value,
                                    &[],
                                    &[],
                                    fuel,
                                )
                                .map_err(|error| copy_error(error, argument.value.source))?;
                                emit(
                                    &mut work,
                                    Op::DefaultArgument {
                                        call,
                                        destination: value,
                                        value: Arc::new(argument.value.clone()),
                                        dynamic: !matches!(
                                            callee.kind,
                                            K::Function(_)
                                                | K::FunctionInstance { .. }
                                                | K::TraitMethod { .. }
                                        ),
                                    },
                                );
                            } else if argument.passing != ArgumentPassing::Value {
                                let mut base = &argument.value;
                                let mut fields = Vec::new();
                                loop {
                                    let (next, field) = match &base.kind {
                                        K::Field {
                                            value,
                                            declaration,
                                            index,
                                        } => (
                                            value.as_ref(),
                                            (FieldOwner::Nominal(*declaration), *index),
                                        ),
                                        K::AnonymousField { value, index, .. } => {
                                            (value.as_ref(), (FieldOwner::Anonymous, *index))
                                        }
                                        _ => break,
                                    };
                                    *fuel =
                                        fuel.checked_sub(1).ok_or(Error::Budget(base.source))?;
                                    fields.push(field);
                                    base = next;
                                }
                                fields.reverse();
                                let (base_slot, local) = match base.kind {
                                    K::Local(binding) => {
                                        (*locals.entry(binding).or_insert_with(&mut fresh), true)
                                    }
                                    _ => (fresh(), false),
                                };
                                work.push(Work::Emit(
                                    argument.value.source,
                                    Op::Reference {
                                        destination: value,
                                        base: base_slot,
                                        local,
                                        fields,
                                        rebind: argument.passing
                                            == ArgumentPassing::ComptimeReference,
                                    },
                                ));
                                if !local {
                                    work.push(Work::Expression(base, base_slot));
                                }
                            } else {
                                work.push(Work::Expression(&argument.value, value));
                            }
                            emit(
                                &mut work,
                                Op::CheckCallArgument {
                                    call,
                                    parameter: argument.parameter,
                                    is_default: argument.is_default,
                                    passing: argument.passing,
                                    argument_source: argument.value.source,
                                },
                            );
                        }
                        emit(
                            &mut work,
                            Op::BeginCall {
                                call,
                                callee: target,
                                callee_source: callee.source,
                                normal_form: if callee.ty.is_normal_form_callable() {
                                    nir::type_computations::charge_term(&callee.ty, fuel)
                                        .map_err(|_| Error::Budget(source))?;
                                    Some((callee.ty.clone(), arguments.len()))
                                } else {
                                    None
                                },
                                parameter_bindings: parameter_bindings.clone(),
                                prepare_defaults: arguments.iter().any(|a| a.is_default),
                            },
                        );
                        work.push(Work::Expression(callee, target));
                    }
                    K::Field { value, .. } | K::AnonymousField { value, .. } => {
                        let (nominal, name, index) = match &expr.kind {
                            K::Field {
                                declaration, index, ..
                            } => (Some(*declaration), None, *index),
                            K::AnonymousField { name, index, .. } => {
                                *fuel =
                                    fuel.checked_sub(name.len()).ok_or(Error::Budget(source))?;
                                (None, Some(name.clone()), *index)
                            }
                            _ => unreachable!(),
                        };
                        let (base, local) = match value.kind {
                            K::Local(binding) => {
                                (*locals.entry(binding).or_insert_with(&mut fresh), true)
                            }
                            _ => (fresh(), false),
                        };
                        emit(
                            &mut work,
                            Op::Field {
                                destination: dst,
                                base,
                                local,
                                nominal,
                                name,
                                index,
                                ty: copy_term(&expr.ty, fuel, source)?,
                                base_source: value.source,
                            },
                        );
                        if !local {
                            work.push(Work::Expression(value, base));
                        }
                    }
                    K::Record { fields, .. } | K::AnonymousRecord { fields } => {
                        *fuel = fuel
                            .checked_sub(fields.len())
                            .ok_or(Error::Budget(source))?;
                        let slots: Vec<_> = fields.iter().map(|_| fresh()).collect();
                        let declaration = match expr.kind {
                            K::Record { declaration, .. } => Some(declaration),
                            _ => None,
                        };
                        emit(
                            &mut work,
                            Op::Record {
                                destination: dst,
                                declaration,
                                ty: copy_term(&expr.ty, fuel, source)?,
                                fields: slots.clone(),
                            },
                        );
                        for (field, slot) in fields.iter().zip(slots).rev() {
                            work.push(Work::Expression(field, slot));
                        }
                    }
                    K::Variant {
                        declaration,
                        index,
                        fields,
                    } => {
                        *fuel = fuel
                            .checked_sub(fields.len())
                            .ok_or(Error::Budget(source))?;
                        let slots: Vec<_> = fields.iter().map(|_| fresh()).collect();
                        emit(
                            &mut work,
                            Op::Variant {
                                destination: dst,
                                declaration: *declaration,
                                index: *index,
                                ty: copy_term(&expr.ty, fuel, source)?,
                                fields: slots.clone(),
                            },
                        );
                        for (field, slot) in fields.iter().zip(slots).rev() {
                            work.push(Work::Expression(field, slot));
                        }
                    }
                    K::Tuple(fields) => {
                        *fuel = fuel
                            .checked_sub(fields.len())
                            .ok_or(Error::Budget(source))?;
                        let slots: Vec<_> = fields.iter().map(|_| fresh()).collect();
                        emit(
                            &mut work,
                            Op::Tuple {
                                destination: dst,
                                fields: slots.clone(),
                            },
                        );
                        for (field, slot) in fields.iter().zip(slots).rev() {
                            work.push(Work::Expression(field, slot));
                        }
                    }
                    K::NormalFormType { plan: Some(_), .. }
                    | K::NominalType { .. }
                    | K::CheckTypeAlias { .. } => {
                        // The template shares closure bodies. Type rewriting is
                        // charged only when the admitted plan specializes them.
                        let mut copy_steps = *fuel as u64;
                        let copied = nir::copy_budget::charge_expression(expr, &mut copy_steps);
                        *fuel = copy_steps as usize;
                        copied.map_err(|error| copy_error(error, source))?;
                        for binding in
                            nir::free_bindings::collect(expr, fuel).map_err(|message| {
                                if message.contains("budget") {
                                    Error::Budget(source)
                                } else {
                                    Error::Unsupported(source)
                                }
                            })?
                        {
                            locals.entry(binding).or_insert_with(&mut fresh);
                        }
                        emit(
                            &mut work,
                            Op::TypeInputPlan {
                                destination: dst,
                                expression: Arc::new(expr.clone()),
                            },
                        );
                    }
                    K::NormalFormType {
                        head,
                        arguments,
                        plan: None,
                    } => {
                        *fuel = fuel
                            .checked_sub(arguments.len())
                            .ok_or(Error::Budget(source))?;
                        let mut inputs = Vec::with_capacity(arguments.len());
                        for argument in arguments {
                            inputs.push(normal_types::compile_input(
                                argument,
                                dst,
                                &mut fresh,
                                fuel,
                                &mut labels,
                                &mut locals,
                                false,
                            )?);
                        }
                        let constructor = Arc::new(normal_types::Constructor {
                            head: *head,
                            ty: copy_term(&expr.ty, fuel, source)?,
                            count: inputs.len(),
                            diverges: inputs.iter().any(|(shape, _)| shape.diverges),
                        });
                        emit(&mut work, Op::FinishNormalType { destination: dst });
                        for (index, (input, code)) in inputs.into_iter().enumerate().rev() {
                            work.push(Work::Emit(
                                input.source,
                                Op::BindNormalInput {
                                    destination: dst,
                                    value: input.slot,
                                    diverges: input.diverges,
                                },
                            ));
                            work.extend(code.into_iter().rev());
                            work.push(Work::Emit(
                                input.source,
                                Op::PrepareNormalInput {
                                    destination: dst,
                                    index,
                                    input: Arc::new(input),
                                },
                            ));
                        }
                        emit(
                            &mut work,
                            Op::BeginNormalType {
                                destination: dst,
                                constructor,
                            },
                        );
                    }
                    K::FunctionType { .. }
                    | K::FunctionResultType { .. }
                    | K::OptionalType(_)
                    | K::PointerType(_)
                    | K::ClosureType { .. } => {
                        use type_constructors::{Constructor, Kind};
                        let count = match &expr.kind {
                            K::FunctionType {
                                comptime_parameters,
                                parameters,
                                ..
                            } => comptime_parameters.len().saturating_add(parameters.len()),
                            K::ClosureType { environment, .. } => {
                                environment.len().saturating_add(1)
                            }
                            _ => 2,
                        };
                        *fuel = fuel.checked_sub(count).ok_or(Error::Budget(source))?;
                        let (kind, fields): (_, Vec<&Expr>) = match &expr.kind {
                            K::FunctionType {
                                comptime,
                                comptime_parameters,
                                parameters,
                            } => (
                                Kind::Function {
                                    comptime: *comptime,
                                    parameters: comptime_parameters.clone(),
                                },
                                parameters.iter().collect(),
                            ),
                            K::FunctionResultType { signature, result } => {
                                (Kind::Result, vec![signature, result])
                            }
                            K::OptionalType(inner) => (Kind::Optional, vec![inner]),
                            K::PointerType(inner) => (Kind::Pointer, vec![inner]),
                            K::ClosureType {
                                environment,
                                signature,
                            } => (
                                Kind::Closure,
                                environment
                                    .iter()
                                    .chain(std::iter::once(signature.as_ref()))
                                    .collect(),
                            ),
                            _ => unreachable!(),
                        };
                        let constructor = Arc::new(Constructor {
                            kind,
                            ty: copy_term(&expr.ty, fuel, source)?,
                            inputs: fields
                                .iter()
                                .map(|field| copy_term(&field.ty, fuel, field.source))
                                .collect::<Result<_, _>>()?,
                        });
                        let inputs: Vec<_> = fields.iter().map(|_| fresh()).collect();
                        emit(
                            &mut work,
                            Op::TypeConstructor {
                                destination: dst,
                                constructor: constructor.clone(),
                                inputs: inputs.clone(),
                            },
                        );
                        for (index, (field, value)) in fields.iter().zip(inputs).enumerate().rev() {
                            work.push(Work::Emit(
                                field.source,
                                Op::CheckTypeConstructorInput {
                                    constructor: constructor.clone(),
                                    index,
                                    value,
                                },
                            ));
                            work.push(Work::Expression(field, value));
                        }
                        emit(&mut work, Op::PrepareTypeConstructor(constructor));
                    }
                    K::TypeListIndex(list, index) => {
                        let contract = Arc::new(lists::Index {
                            ty: copy_term(&expr.ty, fuel, source)?,
                            list: copy_term(&list.ty, fuel, list.source)?,
                            index: copy_term(&index.ty, fuel, index.source)?,
                        });
                        if let K::Local(binding) = list.kind
                            && matches!(
                                index.kind,
                                K::Local(_) | K::Literal(nir::program::Literal::Integer(_))
                            )
                        {
                            let slot = *locals.entry(binding).or_insert_with(&mut fresh);
                            let index = match index.kind {
                                K::Local(binding) => leaf_inputs::Selector::Local(
                                    *locals.entry(binding).or_insert_with(&mut fresh),
                                    index.source,
                                ),
                                _ => {
                                    nir::copy_budget::charge_substitution(index, &[], &[], fuel)
                                        .map_err(|error| copy_error(error, index.source))?;
                                    leaf_inputs::Selector::Literal(Arc::new(index.as_ref().clone()))
                                }
                            };
                            emit(
                                &mut work,
                                Op::LocalListIndex {
                                    destination: dst,
                                    contract: contract.clone(),
                                    list: slot,
                                    list_source: list.source,
                                    index,
                                },
                            );
                        } else {
                            let list_slot = fresh();
                            let index_slot = fresh();
                            emit(
                                &mut work,
                                Op::ListIndex {
                                    destination: dst,
                                    list: list_slot,
                                    index: index_slot,
                                },
                            );
                            work.push(Work::Expression(index, index_slot));
                            emit(
                                &mut work,
                                Op::CheckIndexedList {
                                    value: list_slot,
                                    contract: contract.clone(),
                                },
                            );
                            work.push(Work::Expression(list, list_slot));
                        }
                        emit(&mut work, Op::PrepareListIndex(contract));
                    }
                    K::TypeList(fields) => {
                        *fuel = fuel
                            .checked_sub(fields.len())
                            .ok_or(Error::Budget(source))?;
                        let list = Arc::new(lists::Construction {
                            ty: copy_term(&expr.ty, fuel, source)?,
                            inputs: fields
                                .iter()
                                .map(|field| copy_term(&field.ty, fuel, field.source))
                                .collect::<Result<_, _>>()?,
                        });
                        let values: Vec<_> = fields.iter().map(|_| fresh()).collect();
                        emit(
                            &mut work,
                            Op::List {
                                destination: dst,
                                list: list.clone(),
                                values: values.clone(),
                            },
                        );
                        for (field, slot) in fields.iter().zip(values).rev() {
                            if expr.ty != Term::Primitive(Primitive::Never)
                                && field.ty == Term::Primitive(Primitive::Type)
                            {
                                work.push(Work::Emit(field.source, Op::TypeDescription(slot)));
                            }
                            work.push(Work::Expression(field, slot));
                        }
                        emit(&mut work, Op::PrepareList(list));
                    }
                    K::ListLength(left) | K::ListConcat(left, _) => {
                        let right = if let K::ListConcat(_, right) = &expr.kind {
                            Some(right.as_ref())
                        } else {
                            None
                        };
                        let operation = Arc::new(lists::Operation {
                            ty: copy_term(&expr.ty, fuel, source)?,
                            left: copy_term(&left.ty, fuel, left.source)?,
                            right: right
                                .map(|right| copy_term(&right.ty, fuel, right.source))
                                .transpose()?,
                        });
                        let local = if right.is_none() {
                            if let K::Local(binding) = left.kind {
                                Some(*locals.entry(binding).or_insert_with(&mut fresh))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        let left_slot = local.unwrap_or_else(&mut fresh);
                        if let Some(right) = right {
                            let right_slot = fresh();
                            emit(
                                &mut work,
                                Op::ListConcat {
                                    destination: dst,
                                    left: left_slot,
                                    right: right_slot,
                                },
                            );
                            emit(
                                &mut work,
                                Op::CheckListOperand {
                                    value: right_slot,
                                    ty: Arc::new(operation.right.as_ref().unwrap().clone()),
                                },
                            );
                            work.push(Work::Expression(right, right_slot));
                        } else {
                            emit(
                                &mut work,
                                Op::ListLength {
                                    destination: dst,
                                    value: left_slot,
                                    ty: Arc::new(operation.left.clone()),
                                    local: local.is_some(),
                                    value_source: left.source,
                                },
                            );
                        }
                        if local.is_none() {
                            emit(
                                &mut work,
                                Op::CheckListOperand {
                                    value: left_slot,
                                    ty: Arc::new(operation.left.clone()),
                                },
                            );
                            work.push(Work::Expression(left, left_slot));
                        }
                        emit(&mut work, Op::PrepareListOperation(operation));
                    }
                    K::TypeEqual(left, right) => {
                        let left_slot = fresh();
                        let right_slot = fresh();
                        emit(
                            &mut work,
                            Op::TypeEqual {
                                destination: dst,
                                left: left_slot,
                                right: right_slot,
                            },
                        );
                        emit(&mut work, Op::TypeDescription(right_slot));
                        work.push(Work::Expression(right, right_slot));
                        emit(&mut work, Op::TypeDescription(left_slot));
                        work.push(Work::Expression(left, left_slot));
                        emit(
                            &mut work,
                            Op::PrepareTypeEqual {
                                ty: Arc::new(copy_term(&expr.ty, fuel, source)?),
                                inputs: [
                                    Arc::new(copy_term(&left.ty, fuel, source)?),
                                    Arc::new(copy_term(&right.ty, fuel, source)?),
                                ],
                            },
                        );
                    }
                    K::ObjectQuery { subject, .. } | K::ObjectEdit { subject, .. } => {
                        let operation = match &expr.kind {
                            K::ObjectQuery { query, .. } => objects::ObjectOperation::Query(*query),
                            K::ObjectEdit { operation, .. } => {
                                objects::ObjectOperation::Edit(*operation)
                            }
                            _ => unreachable!(),
                        };
                        let access = Arc::new(objects::ObjectAccess {
                            operation,
                            ty: copy_term(&expr.ty, fuel, source)?,
                            input: copy_term(&subject.ty, fuel, subject.source)?,
                        });
                        let local = match (&expr.kind, &subject.kind) {
                            (K::ObjectQuery { query, .. }, K::Tuple(fields))
                                if fields.len()
                                    == if *query == nir::program::ObjectQueryKind::Length {
                                        1
                                    } else {
                                        2
                                    }
                                    && matches!(fields[0].kind, K::Local(_))
                                    && fields.get(1).is_none_or(|field| {
                                        matches!(field.kind, K::Local(_) | K::Literal(_))
                                    }) =>
                            {
                                Some(fields)
                            }
                            _ => None,
                        };
                        if let Some(fields) = local {
                            let K::Local(binding) = fields[0].kind else {
                                unreachable!()
                            };
                            let receiver = *locals.entry(binding).or_insert_with(&mut fresh);
                            let selector = fields
                                .get(1)
                                .map(|field| match field.kind {
                                    K::Local(binding) => Ok(leaf_inputs::Selector::Local(
                                        *locals.entry(binding).or_insert_with(&mut fresh),
                                        field.source,
                                    )),
                                    K::Literal(_) => {
                                        nir::copy_budget::charge_substitution(
                                            field,
                                            &[],
                                            &[],
                                            fuel,
                                        )
                                        .map_err(|error| copy_error(error, field.source))?;
                                        Ok(leaf_inputs::Selector::Literal(Arc::new(field.clone())))
                                    }
                                    _ => unreachable!(),
                                })
                                .transpose()?;
                            emit(
                                &mut work,
                                Op::LocalObjectQuery {
                                    destination: dst,
                                    access: access.clone(),
                                    receiver,
                                    receiver_source: fields[0].source,
                                    selector,
                                },
                            );
                        } else {
                            let packet = fresh();
                            emit(
                                &mut work,
                                Op::ObjectAccess {
                                    destination: dst,
                                    access: access.clone(),
                                    packet,
                                },
                            );
                            work.push(Work::Expression(subject, packet));
                        }
                        emit(&mut work, Op::PrepareObjectAccess(access));
                    }
                    K::Object { names, fields } => {
                        let mut weight = names.len().saturating_add(fields.len());
                        for name in names.iter().flatten() {
                            weight = weight.saturating_add(name.len());
                        }
                        *fuel = fuel.checked_sub(weight).ok_or(Error::Budget(source))?;
                        let object = Arc::new(objects::ObjectConstructor {
                            ty: copy_term(&expr.ty, fuel, source)?,
                            names: names.clone(),
                            fields: fields
                                .iter()
                                .map(|field| copy_term(&field.ty, fuel, field.source))
                                .collect::<Result<_, _>>()?,
                        });
                        let slots: Vec<_> = fields.iter().map(|_| fresh()).collect();
                        emit(
                            &mut work,
                            Op::Object {
                                destination: dst,
                                object: object.clone(),
                                fields: slots.clone(),
                            },
                        );
                        for (index, (field, slot)) in fields.iter().zip(slots).enumerate().rev() {
                            work.push(Work::Emit(
                                field.source,
                                Op::CheckObjectEntry {
                                    object: object.clone(),
                                    index,
                                    value: slot,
                                },
                            ));
                            work.push(Work::Expression(field, slot));
                        }
                        emit(&mut work, Op::PrepareObject(object));
                    }
                    K::Match { value, arms } => {
                        let subject = fresh();
                        let condition = fresh();
                        let done = labels.len();
                        labels.push(None);
                        let mut sequence = vec![Work::Expression(value, subject)];
                        for arm in arms {
                            let candidate = patterns::prepare(&arm.pattern, fuel, source)?;
                            let bindings = candidate
                                .bindings
                                .into_iter()
                                .map(|id| (id, *locals.entry(id).or_insert_with(&mut fresh)))
                                .collect();
                            let next = labels.len();
                            labels.push(None);
                            sequence.push(Work::BeginScope(source));
                            sequence.push(Work::Emit(
                                source,
                                Op::MatchPattern {
                                    destination: condition,
                                    value: subject,
                                    pattern: candidate.pattern,
                                    bindings,
                                },
                            ));
                            sequence.push(Work::Emit(
                                source,
                                Op::Branch {
                                    condition,
                                    when: false,
                                    target: next,
                                },
                            ));
                            for guard in candidate.guards.into_iter().chain(arm.guard.iter()) {
                                sequence.push(Work::Expression(guard, condition));
                                sequence.push(Work::Emit(
                                    guard.source,
                                    Op::Branch {
                                        condition,
                                        when: false,
                                        target: next,
                                    },
                                ));
                            }
                            sequence.push(Work::Expression(&arm.body, dst));
                            sequence.push(Work::Emit(source, Op::LeaveScope));
                            sequence.push(Work::Emit(source, Op::Jump(done)));
                            sequence.push(Work::Label(next));
                            sequence.push(Work::EndScope(source));
                        }
                        sequence.push(Work::Emit(source, Op::NonExhaustiveMatch));
                        sequence.push(Work::Label(done));
                        work.extend(sequence.into_iter().rev());
                    }
                    K::OptionalPresent(_) | K::OptionalAbsent => {
                        let operand = match &expr.kind {
                            K::OptionalPresent(v) => Some(v.as_ref()),
                            _ => None,
                        };
                        let ty = Arc::new(copy_term(&expr.ty, fuel, source)?);
                        let slot = operand.map(|_| fresh());
                        emit(
                            &mut work,
                            Op::Optional {
                                destination: dst,
                                ty: ty.clone(),
                                value: slot,
                                value_source: operand.map_or(source, |v| v.source),
                            },
                        );
                        if let Some((operand, slot)) = operand.zip(slot) {
                            work.push(Work::Expression(operand, slot));
                        }
                        let input = operand
                            .map(|v| {
                                Ok::<_, Error>((
                                    Arc::new(copy_term(&v.ty, fuel, v.source)?),
                                    v.source,
                                ))
                            })
                            .transpose()?;
                        emit(&mut work, Op::PrepareOptional { ty, input });
                    }
                    K::ErrorConvert { conversion, value } => {
                        let conversion = Arc::new(ErrorConversion {
                            conversion: *conversion,
                            input: copy_term(&value.ty, fuel, source)?,
                            output: copy_term(&expr.ty, fuel, source)?,
                        });
                        let slot = fresh();
                        emit(
                            &mut work,
                            Op::ErrorConvert {
                                destination: dst,
                                value: slot,
                                conversion: conversion.clone(),
                            },
                        );
                        work.push(Work::Expression(value, slot));
                        emit(&mut work, Op::PrepareError(conversion));
                    }
                    K::UnsafeScope(body) => work.push(Work::Expression(body, dst)),
                    K::Intrinsic {
                        tag,
                        binding,
                        inputs,
                    } => {
                        *fuel = fuel
                            .checked_sub(inputs.len())
                            .ok_or(Error::Budget(source))?;
                        let binding = binding
                            .as_ref()
                            .map(|binding| {
                                binding
                                    .charge_copy(fuel)
                                    .map_err(|error| copy_error(error, source))?;
                                Ok::<_, Error>(Arc::new(binding.clone()))
                            })
                            .transpose()?;
                        let slots: Vec<_> = inputs.iter().map(|_| fresh()).collect();
                        emit(
                            &mut work,
                            Op::Intrinsic {
                                destination: dst,
                                binding: binding.clone(),
                                inputs: slots.clone(),
                            },
                        );
                        for (input, slot) in inputs.iter().zip(slots).rev() {
                            work.push(Work::Expression(input, slot));
                        }
                        emit(
                            &mut work,
                            Op::PrepareIntrinsic {
                                tag: *tag,
                                binding: binding.clone(),
                                inputs: inputs
                                    .iter()
                                    .map(|input| copy_term(&input.ty, fuel, source))
                                    .collect::<Result<_, _>>()?,
                                output: copy_term(&expr.ty, fuel, source)?,
                            },
                        );
                    }
                    K::Unit => emit(&mut work, Op::Unit(dst)),
                    K::Literal(_)
                    | K::TypeValue(_)
                    | K::ComptimeValue(_)
                    | K::TypeListValue(_)
                    | K::Function(_)
                    | K::FunctionInstance { .. } => {
                        nir::copy_budget::charge_substitution(expr, &[], &[], fuel)
                            .map_err(|error| copy_error(error, source))?;
                        emit(
                            &mut work,
                            Op::Constant {
                                destination: dst,
                                value: Arc::new(expr.clone()),
                            },
                        )
                    }
                    K::Local(id) => {
                        let slot = *locals.entry(*id).or_insert_with(&mut fresh);
                        emit(
                            &mut work,
                            Op::Read {
                                destination: dst,
                                local: slot,
                            },
                        );
                    }
                    K::LetPattern {
                        pattern,
                        value,
                        guard,
                        otherwise,
                    } => {
                        let candidate = patterns::prepare(pattern, fuel, source)?;
                        let bindings = candidate
                            .bindings
                            .into_iter()
                            .map(|id| (id, *locals.entry(id).or_insert_with(&mut fresh)))
                            .collect();
                        let subject = fresh();
                        let condition = fresh();
                        let failed = labels.len();
                        labels.push(None);
                        let done = labels.len();
                        labels.push(None);
                        let mut sequence = vec![
                            Work::Expression(value, subject),
                            Work::BeginScope(source),
                            Work::Emit(
                                source,
                                Op::MatchPattern {
                                    destination: condition,
                                    value: subject,
                                    pattern: candidate.pattern,
                                    bindings,
                                },
                            ),
                            Work::Emit(
                                source,
                                Op::Branch {
                                    condition,
                                    when: false,
                                    target: failed,
                                },
                            ),
                        ];
                        for guard in candidate.guards.into_iter().chain(guard.as_deref()) {
                            sequence.push(Work::Expression(guard, condition));
                            sequence.push(Work::Emit(
                                guard.source,
                                Op::Branch {
                                    condition,
                                    when: false,
                                    target: failed,
                                },
                            ));
                        }
                        sequence.push(Work::Emit(source, Op::CommitScope));
                        sequence.push(Work::Emit(source, Op::Unit(dst)));
                        sequence.push(Work::Emit(source, Op::Jump(done)));
                        sequence.push(Work::Label(failed));
                        sequence.push(Work::EndScope(source));
                        if let Some(otherwise) = otherwise {
                            sequence.push(Work::Expression(otherwise, dst));
                            sequence.push(Work::Emit(source, Op::InvalidControlFlow));
                        } else {
                            sequence.push(Work::Emit(source, Op::NonExhaustiveMatch));
                        }
                        sequence.push(Work::Label(done));
                        work.extend(sequence.into_iter().rev());
                    }
                    K::Let { binding, value } => {
                        let slot = *locals.entry(*binding).or_insert_with(&mut fresh);
                        let src = fresh();
                        emit(&mut work, Op::Unit(dst));
                        emit(
                            &mut work,
                            Op::Store {
                                local: slot,
                                value: src,
                                initialize: true,
                            },
                        );
                        work.push(Work::Expression(value, src));
                    }
                    K::Assign { place, value } => {
                        *fuel = fuel
                            .checked_sub(place.fields.len())
                            .ok_or(Error::Budget(source))?;
                        let slot = *locals.entry(place.root).or_insert_with(&mut fresh);
                        let src = fresh();
                        if let K::ListConcat(left, right) = &value.kind
                            && place.fields.is_empty()
                            && matches!(left.kind, K::Local(binding) if binding == place.root)
                            && expr.ty == Term::Primitive(Primitive::Void)
                            && left.ty == value.ty
                            && right.ty == value.ty
                            && lists::inert_suffix(right, fuel)?
                        {
                            emit(&mut work, Op::Unit(dst));
                            work.push(Work::Emit(
                                value.source,
                                Op::ListAppend {
                                    local: slot,
                                    suffix: src,
                                    ty: copy_term(&value.ty, fuel, value.source)?,
                                },
                            ));
                            work.push(Work::Expression(right, src));
                            work.push(Work::Emit(
                                left.source,
                                Op::CheckListAppend {
                                    local: slot,
                                    ty: copy_term(&value.ty, fuel, value.source)?,
                                },
                            ));
                            emit(&mut work, Op::CheckBinding(slot));
                            continue;
                        }
                        emit(&mut work, Op::Unit(dst));
                        emit(
                            &mut work,
                            Op::WritePlace {
                                local: slot,
                                value: src,
                                fields: place.fields.clone(),
                            },
                        );
                        work.push(Work::Expression(value, src));
                        emit(&mut work, Op::CheckBinding(slot));
                    }
                    K::Block { statements, result } | K::CallPreparation { statements, result } => {
                        scope_depth += 1;
                        instructions.push(Instruction {
                            source,
                            op: Op::EnterScope,
                        });
                        work.push(Work::EndScope(source));
                        if let Some(value) = result {
                            work.push(Work::Expression(value, dst));
                        } else {
                            emit(&mut work, Op::Unit(dst));
                        }
                        let scratch = fresh();
                        *fuel = fuel
                            .checked_sub(statements.len())
                            .ok_or(Error::Budget(source))?;
                        for statement in statements.iter().rev() {
                            work.push(Work::Expression(statement, scratch));
                        }
                    }
                    K::Discard(value) => {
                        let scratch = fresh();
                        emit(&mut work, Op::Unit(dst));
                        work.push(Work::Expression(value, scratch));
                    }
                    K::Not(value) => {
                        let src = fresh();
                        emit(
                            &mut work,
                            Op::Not {
                                destination: dst,
                                value: src,
                                ty: copy_term(&expr.ty, fuel, source)?,
                            },
                        );
                        work.push(Work::Expression(value, src));
                    }
                    K::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        let otherwise = labels.len();
                        labels.push(None);
                        let done = labels.len();
                        labels.push(None);
                        let src = fresh();
                        work.push(Work::Label(done));
                        if let Some(value) = else_branch {
                            work.push(Work::Expression(value, dst));
                        } else {
                            emit(&mut work, Op::Unit(dst));
                        }
                        work.push(Work::Label(otherwise));
                        emit(&mut work, Op::Jump(done));
                        work.push(Work::Expression(then_branch, dst));
                        emit(
                            &mut work,
                            Op::Branch {
                                condition: src,
                                when: false,
                                target: otherwise,
                            },
                        );
                        work.push(Work::Expression(condition, src));
                    }
                    K::And(left, right) | K::Or(left, right)
                        if left.ty == Term::Primitive(Primitive::Bool) =>
                    {
                        let done = labels.len();
                        labels.push(None);
                        work.push(Work::Label(done));
                        work.push(Work::Expression(right, dst));
                        emit(
                            &mut work,
                            Op::Branch {
                                condition: dst,
                                when: matches!(expr.kind, K::Or(..)),
                                target: done,
                            },
                        );
                        work.push(Work::Expression(left, dst));
                    }
                    K::Loop { identity, body } => {
                        if loops.contains_key(identity) {
                            return Err(Error::InvalidControl(source));
                        }
                        let start = labels.len();
                        labels.push(None);
                        let done = labels.len();
                        labels.push(None);
                        loops.insert(*identity, (start, done, scope_depth));
                        work.push(Work::EndLoop(*identity));
                        emit(&mut work, Op::Unit(dst));
                        work.push(Work::Label(done));
                        emit(&mut work, Op::Jump(start));
                        work.push(Work::Expression(body, fresh()));
                        work.push(Work::Label(start));
                    }
                    K::Break(id) | K::Continue(id) => {
                        let (start, done, depth) =
                            loops.get(id).ok_or(Error::InvalidControl(source))?;
                        emit(
                            &mut work,
                            Op::Jump(if matches!(expr.kind, K::Break(_)) {
                                *done
                            } else {
                                *start
                            }),
                        );
                        emit(&mut work, Op::Unwind(*depth));
                    }
                    K::Return(value) => {
                        let src = fresh();
                        emit(
                            &mut work,
                            Op::Finish {
                                value: src,
                                explicit: true,
                            },
                        );
                        if let Some(value) = value {
                            work.push(Work::Expression(value, src));
                        } else {
                            emit(&mut work, Op::Unit(src));
                        }
                    }
                    _ => return Err(Error::Unsupported(source)),
                }
            }
        }
    }
    for instruction in &mut instructions {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(source))?;
        let target = match &mut instruction.op {
            Op::Jump(target) | Op::Branch { target, .. } => target,
            _ => continue,
        };
        *target = labels[*target].ok_or(Error::InvalidControl(instruction.source))?;
    }
    Ok((
        Program {
            instructions,
            locals: locals.into_iter().collect(),
            registers,
            storage_work: initial_fuel - *fuel,
        },
        shape,
    ))
}

fn copy_error(error: nir::copy_budget::CopyBudgetExhausted, source: SourceRange) -> Error {
    match error {
        nir::copy_budget::CopyBudgetExhausted::Steps => Error::Budget(source),
        nir::copy_budget::CopyBudgetExhausted::Depth => Error::Budget(source),
    }
}
fn copy_term(ty: &Term, fuel: &mut usize, source: SourceRange) -> Result<Term, Error> {
    nir::copy_budget::charge_type_substitution(ty, &[], &[], fuel)
        .map_err(|error| copy_error(error, source))?;
    Ok(ty.clone())
}
