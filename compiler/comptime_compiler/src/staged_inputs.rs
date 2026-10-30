//! Code for evaluating contextual operands before their domain is available.
//! Literals and containers retain register keys; no invocation addresses are cached.
use super::*;

#[derive(Debug)]
pub enum Instruction {
    Enter,
    Leave,
    Literal(Register),
    Unit(Register),
    Append {
        parent: Register,
        value: Register,
        spread: bool,
        diverges: bool,
    },
    Container {
        value: Register,
        list: bool,
        diverges: bool,
    },
    Transfer {
        destination: Register,
        value: Register,
        diverges: bool,
    },
    Finish {
        value: Register,
        diverges: bool,
    },
}
impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enter => write!(f, "(stage-enter)"),
            Self::Leave => write!(f, "(stage-leave)"),
            Self::Unit(slot) => write!(f, "(stage-unit %{slot})"),
            Self::Literal(slot) => write!(f, "(stage-literal %{slot})"),
            Self::Append {
                parent,
                value,
                spread,
                diverges,
            } => write!(
                f,
                "(stage-append %{parent} %{value} (spread {spread}) (diverges {diverges}))"
            ),
            Self::Container {
                value,
                list,
                diverges,
            } => write!(
                f,
                "(stage-container %{value} (list {list}) (diverges {diverges}))"
            ),
            Self::Transfer {
                destination,
                value,
                diverges,
            } => write!(
                f,
                "(stage-transfer %{destination} %{value} (diverges {diverges}))"
            ),
            Self::Finish { value, diverges } => {
                write!(f, "(stage-finish %{value} (diverges {diverges}))")
            }
        }
    }
}

pub(super) fn lower<'a>(
    code: Vec<Work<'a>>,
    shape: &normal_types::Shape,
    fuel: &mut usize,
) -> Result<Vec<Work<'a>>, Error> {
    use normal_types::ShapeKind as K;
    let mut pending = vec![shape];
    let mut lists = std::collections::BTreeSet::new();
    while let Some(shape) = pending.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget(shape.source))?;
        match &shape.kind {
            K::Scalar(_) => {}
            K::Block { result } => pending.extend(result.iter().map(Box::as_ref)),
            K::Choice {
                then_input,
                else_input,
                ..
            } => {
                pending.push(else_input);
                pending.push(then_input);
            }
            K::Match(arms) => pending.extend(arms.iter().map(|arm| &arm.body)),
            K::List(items) => {
                lists.insert(shape.slot);
                pending.extend(items.iter().map(|(_, child)| child));
            }
            K::Tuple { items, .. } => pending.extend(items),
        }
    }
    code.into_iter()
        .map(|work| {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget(shape.source))?;
            Ok(match work {
                Work::Emit(source, op) => Work::Emit(
                    source,
                    match op {
                        Op::Unit(slot) => Op::Stage(Instruction::Unit(slot)),
                        Op::NormalLiteral { destination, .. } => {
                            Op::Stage(Instruction::Literal(destination))
                        }
                        Op::NormalResult {
                            destination,
                            value,
                            diverges,
                        } => Op::Stage(Instruction::Transfer {
                            destination,
                            value,
                            diverges,
                        }),
                        Op::AppendNormalContainer {
                            parent,
                            value,
                            spread,
                            diverges,
                            ..
                        } => Op::Stage(Instruction::Append {
                            parent,
                            value,
                            spread,
                            diverges,
                        }),
                        Op::FinishNormalContainer {
                            value, diverges, ..
                        } => Op::Stage(Instruction::Container {
                            value,
                            list: lists.contains(&value),
                            diverges,
                        }),
                        op => op,
                    },
                ),
                work => work,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::NormalFormArgument as A;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };

    #[test]
    fn staged_code_owns_literals_and_bounds_all_compilation_work() {
        let input = A::List {
            source: SOURCE,
            items: vec![
                A::Integer {
                    source: SOURCE,
                    spelling: "17".into(),
                    negative: false,
                },
                A::Tuple {
                    source: SOURCE,
                    type_head: None,
                    items: vec![A::Unit { source: SOURCE }],
                },
            ],
        };
        let (code, shape) = compile_staged_input(&input, &mut 10000).unwrap();
        let work = code.storage_work();
        for mut fuel in 0..work {
            assert!(matches!(
                compile_staged_input(&input, &mut fuel),
                Err(Error::Budget(_))
            ));
        }
        let mut exact = work;
        assert!(compile_staged_input(&input, &mut exact).is_ok());
        let dump = code.to_string();
        assert!(dump.contains("(stage-literal "));
        assert!(dump.contains("(stage-append "));
        assert!(dump.contains("(stage-container "));
        assert!(dump.contains("(stage-finish "));
        assert!(!dump.contains("(prepare-normal-input "));
        assert_eq!(
            dump,
            compile_staged_input(&input, &mut 10000)
                .unwrap()
                .0
                .to_string()
        );
        drop(input);
        assert!(shape.to_string().contains("\"17\""));
        assert_eq!(code.to_string(), dump);
    }
}
