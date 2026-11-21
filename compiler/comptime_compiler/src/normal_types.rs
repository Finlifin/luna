//! Opaque normal-form construction metadata; no definition body is executed.
use super::*;

/// Contextual scalar inputs keep their spelling until the preceding inputs
/// determine the domain. Expression code remains outside this metadata.
#[derive(Debug)]
pub enum Input {
    Value(Term),
    Unit,
    Null,
    Symbol(String),
    Integer { spelling: String, negative: bool },
}
impl Input {
    pub fn diverges(&self) -> bool {
        matches!(self, Self::Value(Term::Primitive(Primitive::Never)))
    }
}
impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(ty) => write!(f, "(value {ty})"),
            Self::Null => write!(f, "(null)"),
            Self::Symbol(name) => write!(f, "(symbol {})", nir::QuotedDisplay(name)),
            Self::Unit => write!(f, "(unit)"),
            Self::Integer { spelling, negative } => {
                write!(f, "(integer {spelling:?} (negative {negative}))")
            }
        }
    }
}

#[derive(Debug)]
pub struct Shape {
    pub slot: Register,
    pub source: SourceRange,
    pub diverges: bool,
    pub kind: ShapeKind,
}
#[derive(Debug)]
pub enum ShapeKind {
    Scalar(Input),
    Match(Vec<ArmShape>),
    Choice {
        condition: Term,
        condition_source: SourceRange,
        then_input: Box<Shape>,
        else_input: Box<Shape>,
    },
    Block {
        result: Option<Box<Shape>>,
    },
    List(Vec<(bool, Shape)>),
    Tuple {
        items: Vec<Shape>,
    },
}
#[derive(Debug)]
pub struct ArmShape {
    pub guard: Option<(Term, SourceRange)>,
    pub body: Shape,
}
impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input-shape %{} (source {} {}) (diverges {}) ",
            self.slot, self.source.start, self.source.end, self.diverges
        )?;
        match &self.kind {
            ShapeKind::Scalar(input) => write!(f, "{input}")?,
            ShapeKind::Match(arms) => {
                write!(f, "(match")?;
                for arm in arms {
                    write!(f, " (arm (guard")?;
                    if let Some((ty, source)) = &arm.guard {
                        write!(f, " {ty} (source {} {})", source.start, source.end)?;
                    }
                    write!(f, ") (body {}))", arm.body)?;
                }
                write!(f, ")")?;
            }
            ShapeKind::Choice {
                condition,
                condition_source,
                then_input,
                else_input,
            } => write!(
                f,
                "(if (condition {condition} (source {} {})) (then {then_input}) (else {else_input}))",
                condition_source.start, condition_source.end
            )?,
            ShapeKind::Block { result } => {
                write!(f, "(block")?;
                if let Some(result) = result {
                    write!(f, " {result}")?;
                }
                write!(f, ")")?;
            }
            ShapeKind::List(items) => {
                write!(f, "(list")?;
                for (spread, item) in items {
                    write!(f, " (item (spread {spread}) {item})")?;
                }
                write!(f, ")")?;
            }
            ShapeKind::Tuple { items } => {
                write!(f, "(tuple")?;
                for item in items {
                    write!(f, " {item}")?;
                }
                write!(f, ")")?;
            }
        }
        write!(f, ")")
    }
}

/// Build owned domain metadata and evaluation tasks with explicit worklists.
/// Container append tasks immediately follow their corresponding child code.
pub(super) fn compile_input<'a>(
    root: &'a nir::program::NormalFormArgument,
    fresh: &mut impl FnMut() -> Register,
    fuel: &mut usize,
    labels: &mut Vec<Option<usize>>,
    locals: &mut BTreeMap<LocalId, Register>,
    staged: bool,
) -> Result<(Shape, Vec<Work<'a>>), Error> {
    use nir::program::NormalFormArgument as A;
    enum Task<'a> {
        Enter(&'a A, usize, usize),
        Code(Work<'a>),
        Transfer(Register, bool, SourceRange),
        Choice(&'a A, Register, usize),
        Match(&'a A, Register, usize),
        Block(&'a A, Register, bool),
        Exit(&'a A, Register, usize, usize),
        Append(Register, bool, SourceRange),
    }
    let mut path = Vec::new();
    let retained_path = |indices: &[usize], fuel: &mut usize, source| {
        *fuel = fuel
            .checked_sub(indices.len())
            .ok_or(Error::Budget(source))?;
        Ok::<_, Error>(nir::program::InputPath::from_indices(indices))
    };
    let transfer = |destination, value, diverges| {
        if staged {
            Op::Stage(staged_inputs::Instruction::Transfer {
                destination,
                value,
                diverges,
            })
        } else {
            Op::NormalResult {
                destination,
                value,
                diverges,
            }
        }
    };
    let mut pending = vec![Task::Enter(root, 0, 0)];
    let mut shapes = Vec::new();
    let mut code = Vec::new();
    while let Some(task) = pending.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(root.source()))?;
        match task {
            Task::Code(work) => code.push(work),
            Task::Transfer(destination, forced, source) => {
                let child: &Shape = shapes.last().unwrap();
                code.push(Work::Emit(
                    source,
                    transfer(destination, child.slot, forced || child.diverges),
                ));
            }
            Task::Match(argument, slot, start) => {
                let items: Vec<Shape> = shapes.split_off(start);
                let A::Match { arms, .. } = argument else {
                    unreachable!()
                };
                let diverges =
                    argument.has_diverging_operand() || items.iter().all(|item| item.diverges);
                let arms = arms
                    .iter()
                    .zip(items)
                    .map(|(arm, body)| {
                        Ok(ArmShape {
                            guard: arm
                                .guard
                                .as_ref()
                                .map(|guard| {
                                    Ok::<_, Error>((
                                        copy_term(&guard.ty, fuel, guard.source)?,
                                        guard.source,
                                    ))
                                })
                                .transpose()?,
                            body,
                        })
                    })
                    .collect::<Result<_, Error>>()?;
                shapes.push(Shape {
                    slot,
                    source: argument.source(),
                    diverges,
                    kind: ShapeKind::Match(arms),
                });
            }
            Task::Choice(argument, slot, start) => {
                let mut items: Vec<Shape> = shapes.split_off(start);
                let else_input = Box::new(items.pop().unwrap());
                let then_input = Box::new(items.pop().unwrap());
                let A::If { condition, .. } = argument else {
                    unreachable!()
                };
                let diverges = argument.has_diverging_operand()
                    || (then_input.diverges && else_input.diverges);
                shapes.push(Shape {
                    slot,
                    source: argument.source(),
                    diverges,
                    kind: ShapeKind::Choice {
                        condition: copy_term(&condition.ty, fuel, condition.source)?,
                        condition_source: condition.source,
                        then_input,
                        else_input,
                    },
                });
            }
            Task::Block(argument, slot, forced) => {
                let A::Block { result, .. } = argument else {
                    unreachable!()
                };
                let result = result.as_ref().map(|_| Box::new(shapes.pop().unwrap()));
                let diverges = forced || result.as_ref().is_some_and(|result| result.diverges);
                if let Some(result) = &result {
                    code.push(Work::Emit(
                        argument.source(),
                        transfer(slot, result.slot, diverges),
                    ));
                } else {
                    let unit = if staged {
                        Op::Stage(staged_inputs::Instruction::Unit(slot))
                    } else {
                        Op::Unit(slot)
                    };
                    code.push(Work::Emit(argument.source(), unit));
                    if diverges {
                        code.push(Work::Emit(
                            argument.source(),
                            transfer(slot, slot, diverges),
                        ));
                    }
                }
                code.push(Work::EndScope(argument.source()));
                shapes.push(Shape {
                    slot,
                    source: argument.source(),
                    diverges,
                    kind: ShapeKind::Block { result },
                });
            }
            Task::Append(parent, spread, source) => {
                let child: &Shape = shapes.last().unwrap();
                code.push(Work::Emit(
                    source,
                    if staged {
                        Op::Stage(staged_inputs::Instruction::Append {
                            parent,
                            value: child.slot,
                            spread,
                            diverges: child.diverges,
                        })
                    } else {
                        Op::AppendNormalContainer {
                            parent,
                            value: child.slot,
                            spread,
                            diverges: child.diverges,
                        }
                    },
                ));
            }
            Task::Exit(argument, slot, start, depth) => {
                let items: Vec<Shape> = shapes.split_off(start);
                let diverges = items.iter().any(|item| item.diverges);
                let kind = match argument {
                    A::List {
                        items: original, ..
                    } => ShapeKind::List(
                        original
                            .iter()
                            .zip(items)
                            .map(|(item, shape)| (matches!(item, A::ListSpread(_)), shape))
                            .collect(),
                    ),
                    A::Tuple { .. } => ShapeKind::Tuple { items },
                    _ => unreachable!(),
                };
                code.push(Work::Emit(
                    argument.source(),
                    if staged {
                        Op::Stage(staged_inputs::Instruction::Container {
                            path: retained_path(&path[..depth], fuel, argument.source())?,
                            value: slot,
                            list: matches!(kind, ShapeKind::List(_)),
                            diverges,
                        })
                    } else {
                        Op::FinishNormalContainer {
                            value: slot,
                            diverges,
                        }
                    },
                ));
                shapes.push(Shape {
                    slot,
                    source: argument.source(),
                    diverges,
                    kind,
                });
            }
            Task::Enter(argument, depth, index) => {
                if depth >= 128 {
                    return Err(Error::Depth(argument.source()));
                }
                if staged {
                    if depth > 0 {
                        path.truncate(depth - 1);
                        path.push(index);
                    }
                    code.push(Work::Emit(
                        argument.source(),
                        Op::Stage(staged_inputs::Instruction::Enter),
                    ));
                    pending.push(Task::Code(Work::Emit(
                        argument.source(),
                        Op::Stage(staged_inputs::Instruction::Leave),
                    )));
                }
                let slot = fresh();
                let scalar = match argument {
                    A::Value(value) | A::ListSpread(value) => {
                        code.push(Work::Expression(value, slot));
                        Some(Input::Value(copy_term(&value.ty, fuel, value.source)?))
                    }
                    A::Unit { .. } => Some(Input::Unit),
                    A::Null { .. } => Some(Input::Null),
                    A::Symbol { source, name } => {
                        *fuel = fuel.checked_sub(name.len()).ok_or(Error::Budget(*source))?;
                        Some(Input::Symbol(name.clone()))
                    }
                    A::Integer {
                        spelling, negative, ..
                    } => {
                        *fuel = fuel
                            .checked_sub(spelling.len())
                            .ok_or(Error::Budget(argument.source()))?;
                        Some(Input::Integer {
                            spelling: spelling.clone(),
                            negative: *negative,
                        })
                    }
                    A::Match {
                        source,
                        value,
                        arms,
                    } => {
                        *fuel = fuel.checked_sub(arms.len()).ok_or(Error::Budget(*source))?;
                        let subject = fresh();
                        let condition = fresh();
                        let done = labels.len();
                        labels.push(None);
                        code.push(Work::Expression(value, subject));
                        pending.push(Task::Match(argument, slot, shapes.len()));
                        pending.push(Task::Code(Work::Label(done)));
                        pending.push(Task::Code(Work::Emit(*source, Op::NonExhaustiveMatch)));
                        let diverges = argument.has_diverging_operand();
                        for (index, arm) in arms.iter().enumerate().rev() {
                            let next = labels.len();
                            labels.push(None);
                            pending.push(Task::Code(Work::EndScope(*source)));
                            pending.push(Task::Code(Work::Label(next)));
                            pending.push(Task::Code(Work::Emit(*source, Op::Jump(done))));
                            pending.push(Task::Code(Work::Emit(*source, Op::LeaveScope)));
                            pending.push(Task::Transfer(slot, diverges, arm.body.source()));
                            pending.push(Task::Enter(&arm.body, depth + 1, index));
                            for guard in arm.guard.iter().rev() {
                                pending.push(Task::Code(Work::Emit(
                                    guard.source,
                                    Op::Branch {
                                        condition,
                                        when: false,
                                        target: next,
                                    },
                                )));
                                pending.push(Task::Code(Work::Expression(guard, condition)));
                            }
                            for step in patterns::lower(
                                &arm.pattern,
                                subject,
                                condition,
                                next,
                                locals,
                                fresh,
                                labels,
                                fuel,
                                *source,
                            )?
                            .into_iter()
                            .rev()
                            {
                                pending.push(Task::Code(step));
                            }
                            pending.push(Task::Code(Work::BeginScope(*source)));
                        }
                        None
                    }
                    A::If {
                        condition,
                        then_input,
                        else_input,
                        ..
                    } => {
                        let otherwise = labels.len();
                        labels.push(None);
                        let done = labels.len();
                        labels.push(None);
                        let test = fresh();
                        let diverges = argument.has_diverging_operand();
                        code.push(Work::Expression(condition, test));
                        code.push(Work::Emit(
                            condition.source,
                            Op::Branch {
                                condition: test,
                                when: false,
                                target: otherwise,
                            },
                        ));
                        pending.push(Task::Choice(argument, slot, shapes.len()));
                        pending.push(Task::Code(Work::Label(done)));
                        pending.push(Task::Transfer(slot, diverges, else_input.source()));
                        pending.push(Task::Enter(else_input, depth + 1, 1));
                        pending.push(Task::Code(Work::Label(otherwise)));
                        pending.push(Task::Code(Work::Emit(argument.source(), Op::Jump(done))));
                        pending.push(Task::Transfer(slot, diverges, then_input.source()));
                        pending.push(Task::Enter(then_input, depth + 1, 0));
                        None
                    }
                    A::Block {
                        statements, result, ..
                    } => {
                        *fuel = fuel
                            .checked_sub(statements.len())
                            .ok_or(Error::Budget(argument.source()))?;
                        code.push(Work::BeginScope(argument.source()));
                        let scratch = fresh();
                        for statement in statements {
                            code.push(Work::Expression(statement, scratch));
                        }
                        pending.push(Task::Block(
                            argument,
                            slot,
                            argument.has_diverging_operand(),
                        ));
                        if let Some(result) = result {
                            pending.push(Task::Enter(result, depth + 1, 0));
                        }
                        None
                    }
                    A::List { items, .. } | A::Tuple { items, .. } => {
                        *fuel = fuel
                            .checked_sub(items.len())
                            .ok_or(Error::Budget(argument.source()))?;
                        pending.push(Task::Exit(argument, slot, shapes.len(), depth));
                        for (index, item) in items.iter().enumerate().rev() {
                            pending.push(Task::Append(
                                slot,
                                matches!(item, A::ListSpread(_)),
                                item.source(),
                            ));
                            pending.push(Task::Enter(item, depth + 1, index));
                        }
                        None
                    }
                };
                if let Some(input) = scalar {
                    if argument.expression().is_none() {
                        code.push(Work::Emit(
                            argument.source(),
                            if staged {
                                Op::Stage(staged_inputs::Instruction::Literal {
                                    value: slot,
                                    path: retained_path(&path, fuel, argument.source())?,
                                })
                            } else {
                                Op::NormalLiteral { destination: slot }
                            },
                        ));
                    }
                    let diverges = input.diverges();
                    shapes.push(Shape {
                        slot,
                        source: argument.source(),
                        diverges,
                        kind: ShapeKind::Scalar(input),
                    });
                }
            }
        }
    }
    Ok((shapes.pop().unwrap(), code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::NormalFormArgument as A;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };

    #[test]
    fn staged_symbol_owns_its_spelling_and_charges_it_before_copying() {
        let name = "tag\"\\\n雪".repeat(32);
        let input = A::Symbol {
            source: SOURCE,
            name: name.clone(),
        };
        let (_, shape) = compile_staged_input(&input, &mut 10_000).unwrap();
        let quoted = nir::QuotedDisplay(&name).to_string();
        assert!(input.to_string().contains(&quoted));
        drop(input);
        assert!(shape.to_string().contains(&format!("(symbol {quoted})")));
        let long = A::Symbol {
            source: SOURCE,
            name: "x".repeat(1024),
        };
        assert!(matches!(
            compile_staged_input(&long, &mut 512),
            Err(Error::Budget(SOURCE))
        ));
    }

    #[test]
    fn contextual_shapes_bound_work_and_depth_before_execution() {
        let mut slots = 0;
        let mut fresh = || {
            slots += 1;
            slots
        };
        let mut input = A::Unit { source: SOURCE };
        assert!(matches!(
            compile_input(
                &input,
                &mut fresh,
                &mut 0,
                &mut vec![],
                &mut BTreeMap::new(),
                false
            ),
            Err(Error::Budget(_))
        ));
        for _ in 0..127 {
            input = A::Tuple {
                source: SOURCE,
                items: vec![input],
            };
        }
        let (shape, code) = compile_input(
            &input,
            &mut fresh,
            &mut 10000,
            &mut vec![],
            &mut BTreeMap::new(),
            false,
        )
        .unwrap();
        assert!(!shape.diverges);
        assert_eq!(code.len(), 1 + 127 * 2);
        input = A::Tuple {
            source: SOURCE,
            items: vec![input],
        };
        assert!(matches!(
            compile_input(
                &input,
                &mut fresh,
                &mut 10000,
                &mut vec![],
                &mut BTreeMap::new(),
                false
            ),
            Err(Error::Depth(_))
        ));
        let wide = A::List {
            source: SOURCE,
            items: vec![A::Unit { source: SOURCE }; 100],
        };
        assert!(matches!(
            compile_input(
                &wide,
                &mut fresh,
                &mut 10,
                &mut vec![],
                &mut BTreeMap::new(),
                false
            ),
            Err(Error::Budget(_))
        ));
        let escaped = Shape {
            slot: 1,
            source: SOURCE,
            diverges: false,
            kind: ShapeKind::Scalar(Input::Integer {
                spelling: "1\"\n".into(),
                negative: false,
            }),
        };
        assert!(escaped.to_string().contains("\"1\\\"\\n\""));
    }
}

#[cfg(test)]
mod contextual_fragment_tests {
    use super::*;
    use nir::program::NormalFormArgument as A;

    #[test]
    fn contextual_fragment_owns_domain_and_prechecks_before_operands() {
        let source = SourceRange { start: 1, end: 2 };
        let (program, work) = {
            let input = A::Integer {
                source,
                spelling: "17".into(),
                negative: false,
            };
            let expected = Term::Primitive(Primitive::U8);
            let program = compile_contextual_input(&input, &expected, &mut 10000).unwrap();
            let work = program.storage_work();
            let mut insufficient = work - 1;
            assert!(matches!(
                compile_contextual_input(&input, &expected, &mut insufficient),
                Err(Error::Budget(_))
            ));
            (program, work)
        };
        assert!(work > 0);
        assert!(matches!(
            program.instructions().first().unwrap().op,
            Op::BeginContextualInput { .. }
        ));
        let dump = program.to_string();
        assert!(dump.contains(&format!("(expected {})", Term::Primitive(Primitive::U8))));
        assert!(dump.contains("(normal-literal "));
        assert!(
            !program
                .instructions()
                .iter()
                .any(|instruction| matches!(instruction.op, Op::TypeInputPlan { .. }))
        );
    }
}
