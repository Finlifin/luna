//! Linear invocation code. Unsupported operations are detected before execution.
//! Constants and default templates own bounded typed NIR copies; opcodes never evaluate trees.
pub mod objects;
mod patterns;
pub mod scalar;
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
                    parameter_bindings,
                    prepare_defaults,
                } => {
                    write!(
                        f,
                        "(begin-call {call} {callee} (prepare-defaults {prepare_defaults}) (parameters"
                    )?;
                    for binding in parameter_bindings {
                        write!(f, " {binding}")?;
                    }
                    write!(f, "))")?;
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
    let initial_fuel = *fuel;
    let mut instructions = Vec::new();
    let mut locals = BTreeMap::new();
    let mut registers = 1;
    let mut labels = Vec::new();
    let mut loops = BTreeMap::new();
    let mut scope_depth = 0;
    let mut work = vec![
        Work::Emit(
            body.source,
            Op::Finish {
                value: 0,
                explicit: false,
            },
        ),
        Work::Expression(body, 0),
    ];
    while let Some(task) = work.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(body.source))?;
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
                    K::Call {
                        callee,
                        arguments,
                        parameter_bindings,
                    } if !callee.ty.is_normal_form_callable()
                        && matches!(
                            callee.kind,
                            K::Function(_) | K::FunctionInstance { .. } | K::Local(_)
                        ) =>
                    {
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
                            emit(
                                &mut work,
                                Op::StoreCallArgument {
                                    call,
                                    parameter: argument.parameter,
                                    value,
                                },
                            );
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
                                        dynamic: matches!(callee.kind, K::Local(_)),
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
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(body.source))?;
        let target = match &mut instruction.op {
            Op::Jump(target) | Op::Branch { target, .. } => target,
            _ => continue,
        };
        *target = labels[*target].ok_or(Error::InvalidControl(instruction.source))?;
    }
    Ok(Program {
        instructions,
        locals: locals.into_iter().collect(),
        registers,
        storage_work: initial_fuel - *fuel,
    })
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
