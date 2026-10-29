//! Opaque normal-form construction metadata; no definition body is executed.
use super::*;

#[derive(Debug)]
pub struct Constructor {
    pub head: nir::DeclarationId,
    pub ty: Term,
    pub count: usize,
    pub diverges: bool,
}
impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(normal-type (head {}) (type {}) (inputs {}) (diverges {}))",
            self.head, self.ty, self.count, self.diverges
        )
    }
}

/// Contextual scalar inputs keep their spelling until the preceding inputs
/// determine the domain. Expression code remains outside this metadata.
#[derive(Debug)]
pub enum Input {
    Value(Term),
    Unit,
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
    List(Vec<(bool, Shape)>),
    Tuple {
        head: Option<nir::DeclarationId>,
        items: Vec<Shape>,
    },
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
            ShapeKind::List(items) => {
                write!(f, "(list")?;
                for (spread, item) in items {
                    write!(f, " (item (spread {spread}) {item})")?;
                }
                write!(f, ")")?;
            }
            ShapeKind::Tuple { head, items } => {
                write!(f, "(tuple (head")?;
                if let Some(head) = head {
                    write!(f, " {head}")?;
                }
                write!(f, ")")?;
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
    constructor: Register,
    fresh: &mut impl FnMut() -> Register,
    fuel: &mut usize,
) -> Result<(Shape, Vec<Work<'a>>), Error> {
    use nir::program::NormalFormArgument as A;
    enum Task<'a> {
        Enter(&'a A, usize),
        Exit(&'a A, Register, usize),
        Append(Register, bool, SourceRange),
    }
    let mut pending = vec![Task::Enter(root, 0)];
    let mut shapes = Vec::new();
    let mut code = Vec::new();
    while let Some(task) = pending.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(root.source()))?;
        match task {
            Task::Append(parent, spread, source) => {
                let child: &Shape = shapes.last().unwrap();
                code.push(Work::Emit(
                    source,
                    Op::AppendNormalContainer {
                        constructor,
                        parent,
                        value: child.slot,
                        spread,
                        diverges: child.diverges,
                    },
                ));
            }
            Task::Exit(argument, slot, start) => {
                let items: Vec<Shape> = shapes.split_off(start);
                let diverges = items.iter().any(|item| item.diverges);
                let kind = match argument {
                    A::TypeList {
                        items: original, ..
                    } => ShapeKind::List(
                        original
                            .iter()
                            .zip(items)
                            .map(|(item, shape)| (matches!(item, A::TypeListSpread(_)), shape))
                            .collect(),
                    ),
                    A::Tuple { type_head, .. } => ShapeKind::Tuple {
                        head: *type_head,
                        items,
                    },
                    _ => unreachable!(),
                };
                code.push(Work::Emit(
                    argument.source(),
                    Op::FinishNormalContainer {
                        constructor,
                        value: slot,
                        diverges,
                    },
                ));
                shapes.push(Shape {
                    slot,
                    source: argument.source(),
                    diverges,
                    kind,
                });
            }
            Task::Enter(argument, depth) => {
                if depth >= 128 {
                    return Err(Error::Unsupported(argument.source()));
                }
                let slot = fresh();
                let scalar = match argument {
                    A::Value(value) | A::TypeListSpread(value) => {
                        code.push(Work::Expression(value, slot));
                        Some(Input::Value(copy_term(&value.ty, fuel, value.source)?))
                    }
                    A::Unit { .. } => Some(Input::Unit),
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
                    A::TypeList { items, .. } | A::Tuple { items, .. } => {
                        *fuel = fuel
                            .checked_sub(items.len())
                            .ok_or(Error::Budget(argument.source()))?;
                        pending.push(Task::Exit(argument, slot, shapes.len()));
                        for item in items.iter().rev() {
                            pending.push(Task::Append(
                                slot,
                                matches!(item, A::TypeListSpread(_)),
                                item.source(),
                            ));
                            pending.push(Task::Enter(item, depth + 1));
                        }
                        None
                    }
                    _ => return Err(Error::Unsupported(argument.source())),
                };
                if let Some(input) = scalar {
                    if argument.expression().is_none() {
                        code.push(Work::Emit(
                            argument.source(),
                            Op::NormalLiteral {
                                constructor,
                                destination: slot,
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
    fn contextual_shapes_bound_work_and_depth_before_execution() {
        let mut slots = 0;
        let mut fresh = || {
            slots += 1;
            slots
        };
        let mut input = A::Unit { source: SOURCE };
        assert!(matches!(
            compile_input(&input, 0, &mut fresh, &mut 0),
            Err(Error::Budget(_))
        ));
        for _ in 0..127 {
            input = A::Tuple {
                source: SOURCE,
                type_head: None,
                items: vec![input],
            };
        }
        let (shape, code) = compile_input(&input, 0, &mut fresh, &mut 10000).unwrap();
        assert!(!shape.diverges);
        assert_eq!(code.len(), 1 + 127 * 2);
        input = A::Tuple {
            source: SOURCE,
            type_head: None,
            items: vec![input],
        };
        assert!(matches!(
            compile_input(&input, 0, &mut fresh, &mut 10000),
            Err(Error::Unsupported(_))
        ));
        let wide = A::TypeList {
            source: SOURCE,
            items: vec![A::Unit { source: SOURCE }; 100],
        };
        assert!(matches!(
            compile_input(&wide, 0, &mut fresh, &mut 10),
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
