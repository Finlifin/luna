//! Linear invocation code. Unsupported operations are detected before execution.
//! Constants borrow immutable typed NIR leaves; no instruction evaluates a tree.
use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
use nir::{Primitive, Term};
use std::collections::BTreeMap;
use std::fmt;

pub type Register = usize;

#[derive(Debug)]
pub enum Op<'a> {
    Constant {
        destination: Register,
        value: &'a Expr,
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
        ty: &'a Term,
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
pub struct Instruction<'a> {
    pub source: SourceRange,
    pub op: Op<'a>,
}
#[derive(Debug)]
pub struct Program<'a> {
    instructions: Vec<Instruction<'a>>,
    locals: Vec<(LocalId, Register)>,
    registers: usize,
}
impl<'a> Program<'a> {
    pub fn instructions(&self) -> &[Instruction<'a>] {
        &self.instructions
    }
    pub fn locals(&self) -> &[(LocalId, Register)] {
        &self.locals
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

impl fmt::Display for Program<'_> {
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
                Op::Constant { destination, value } => {
                    write!(f, "(constant {destination} {value})")?
                }
                Op::EnterScope => write!(f, "(enter-scope)")?,
                Op::LeaveScope => write!(f, "(leave-scope)")?,
                Op::Unwind(depth) => write!(f, "(unwind {depth})")?,
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
    Emit(SourceRange, Op<'a>),
    Label(usize),
    EndLoop(LocalId),
    EndScope(SourceRange),
}

/// Compile structured control using a heap worklist, not host recursion. Fuel
/// bounds traversal and register/code allocation, including unreachable code.
pub fn compile<'a>(body: &'a Expr, fuel: &mut usize) -> Result<Program<'a>, Error> {
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
                match &expr.kind {
                    K::Unit => emit(&mut work, Op::Unit(dst)),
                    K::Literal(_) | K::TypeValue(_) | K::ComptimeValue(_) | K::TypeListValue(_) => {
                        emit(
                            &mut work,
                            Op::Constant {
                                destination: dst,
                                value: expr,
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
                    K::Assign { place, value } if place.fields.is_empty() => {
                        let slot = *locals.entry(place.root).or_insert_with(&mut fresh);
                        let src = fresh();
                        emit(&mut work, Op::Unit(dst));
                        emit(
                            &mut work,
                            Op::Store {
                                local: slot,
                                value: src,
                                initialize: false,
                            },
                        );
                        work.push(Work::Expression(value, src));
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
                                ty: &expr.ty,
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
    })
}
