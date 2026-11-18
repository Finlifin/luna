//! Source-ordered fields of list and tuple inputs. A completed field stays in
//! its original category while the active region checks the next source child.
use super::*;
use driver::{Request, Step};

enum Fields {
    Contextual {
        depth: usize,
        values: Vec<n::NormalFormArgument>,
    },
    Ordinary {
        domains: Domains,
        values: Vec<n::Expr>,
    },
}

enum Domains {
    List {
        element: Destination,
        result: Term,
    },
    Tuple {
        fields: std::vec::IntoIter<Term>,
        expected: Option<Term>,
    },
    PartialTuple(Destination),
}

pub(super) struct Frame<'b, 'h> {
    expr: &'b Expr<'h>,
    items: &'h [Expr<'h>],
    fields: Fields,
}

impl<'b, 'h> Frame<'b, 'h> {
    pub(super) fn contextual(expr: &'b Expr<'h>, depth: usize) -> Result<Self, Error> {
        if depth >= 128 {
            return Err(error(expr.span, "contextual input nesting limit exceeded"));
        }
        let (H::List(items) | H::Tuple(items)) = expr.kind else {
            unreachable!("contextual aggregate")
        };
        Ok(Self {
            expr,
            items,
            fields: Fields::Contextual {
                depth,
                values: vec![],
            },
        })
    }

    pub(super) fn partial_tuple(
        expr: &'b Expr<'h>,
        items: &'h [Expr<'h>],
        destination: Destination,
    ) -> Result<Self, Error> {
        let Some(
            crate::calls::InputContext::Tuple(fields)
            | crate::calls::InputContext::NormalTuple { fields, .. },
        ) = destination.structural_input()
        else {
            unreachable!("tuple input context")
        };
        if fields.len() != items.len() {
            return Err(error(expr.span, "tuple argument arity mismatch"));
        }
        Ok(Self {
            expr,
            items,
            fields: Fields::Ordinary {
                domains: Domains::PartialTuple(destination),
                values: vec![],
            },
        })
    }

    pub(super) fn tuple(
        checker: &mut Checker<'_, '_>,
        expr: &'b Expr<'h>,
        items: &'h [Expr<'h>],
        expected: Option<&Term>,
    ) -> Result<Self, Error> {
        // Structural evidence supplies element contexts. Admission of the
        // complete value still uses the caller's original (possibly opaque) type.
        let shape = expected
            .map(|ty| checker.structural_type(ty, expr.span))
            .transpose()?;
        let expected = expected
            .map(|ty| checker.copy_alias_type(ty, expr.span))
            .transpose()?;
        Ok(Self {
            expr,
            items,
            fields: Fields::Ordinary {
                domains: Domains::Tuple {
                    fields: match shape {
                        Some(Term::Tuple(types)) => types,
                        _ => vec![],
                    }
                    .into_iter(),
                    expected,
                },
                values: vec![],
            },
        })
    }

    pub(super) fn list(
        checker: &mut Checker<'_, '_>,
        expr: &'b Expr<'h>,
        items: &'h [Expr<'h>],
        expected: &Term,
        element: &Term,
    ) -> Result<Self, Error> {
        checker.required_steps = checker
            .required_steps
            .checked_sub(items.len() as u64)
            .ok_or_else(|| error(expr.span, "type-list construction budget exhausted"))?;
        let element = Destination::ordinary(
            checker,
            OrdinaryTail {
                expected: Some(element),
                partial: None,
                input: None,
            },
            expr.span,
        )?;
        let result = checker.copy_alias_type(expected, expr.span)?;
        Ok(Self {
            expr,
            items,
            fields: Fields::Ordinary {
                domains: Domains::List { element, result },
                values: vec![],
            },
        })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        child: Option<CheckedBlock>,
    ) -> Result<Step<'b, 'h>, Error> {
        let span = self.expr.span;
        match &mut self.fields {
            Fields::Contextual { depth, values } => {
                if let Some(child) = child {
                    values.push(child.contextual(span)?);
                }
                if let Some(item) = self.items.get(values.len()) {
                    return Ok(Step::Child(Request::Expression(
                        item,
                        Destination::Contextual(ContextualTail { depth: *depth }),
                    )));
                }
                let items = std::mem::take(values);
                let source = source(span);
                let value = match self.expr.kind {
                    H::List(_) => n::NormalFormArgument::List { source, items },
                    H::Tuple(_) => n::NormalFormArgument::Tuple { source, items },
                    _ => unreachable!("contextual aggregate"),
                };
                Ok(Step::Completed(CheckedBlock::Contextual(value)))
            }
            Fields::Ordinary { domains, values } => {
                if let Some(child) = child {
                    values.push(child.ordinary(span)?);
                }
                if let Some(item) = self.items.get(values.len()) {
                    let destination = domains.element(checker, values.len(), item.span)?;
                    return Ok(Step::Child(Request::Expression(item, destination)));
                }
                let values = std::mem::take(values);
                let diverges = values.iter().any(|value| is_never(&value.ty));
                let (expected, ty, kind) = match domains {
                    Domains::List { result, .. } => (
                        None,
                        if diverges {
                            p(P::Never)
                        } else {
                            checker.copy_alias_type(result, span)?
                        },
                        K::TypeList(values),
                    ),
                    Domains::Tuple { .. } | Domains::PartialTuple(_) => {
                        let ty = if diverges {
                            p(P::Never)
                        } else {
                            checker.tuple_value_type(
                                values.iter().map(|value| value.ty.clone()).collect(),
                                span,
                            )?
                        };
                        let expected = match domains {
                            Domains::Tuple { expected, .. } => expected.as_ref(),
                            _ => None,
                        };
                        (expected, ty, K::Tuple(values))
                    }
                };
                Ok(Step::Completed(CheckedBlock::Ordinary(
                    checker.finish_value_expression(self.expr, expected, ty, kind)?,
                )))
            }
        }
    }
}

impl Domains {
    fn element(
        &mut self,
        checker: &mut Checker<'_, '_>,
        index: usize,
        span: Span,
    ) -> Result<Destination, Error> {
        match self {
            Self::List { element, .. } => Ok(element.clone()),
            Self::Tuple { fields, .. } => Ok(Destination::from_expected(fields.next())),
            Self::PartialTuple(destination) => {
                let Some(
                    crate::calls::InputContext::Tuple(fields)
                    | crate::calls::InputContext::NormalTuple { fields, .. },
                ) = destination.structural_input()
                else {
                    unreachable!("tuple input context")
                };
                Destination::ordinary(
                    checker,
                    OrdinaryTail {
                        expected: None,
                        partial: None,
                        input: Some(&fields[index]),
                    },
                    span,
                )
            }
        }
    }
}

impl std::fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(aggregate-frame (kind {}) (source {}) (arity {}) ",
            if matches!(self.expr.kind, H::List(_)) {
                "list"
            } else {
                "tuple"
            },
            source(self.expr.span),
            self.items.len()
        )?;
        match &self.fields {
            Fields::Contextual { depth, values } => {
                write!(f, "(contextual-depth {depth}) (fields")?;
                for value in values {
                    write!(f, " {value}")?;
                }
            }
            Fields::Ordinary { domains, values } => {
                match domains {
                    Domains::List { element, result } => write!(f, "(list {element} {result})")?,
                    Domains::Tuple { expected, .. } => {
                        f.write_str("(tuple")?;
                        if let Some(ty) = expected {
                            write!(f, " {ty}")?;
                        }
                        f.write_str(")")?;
                    }
                    Domains::PartialTuple(destination) => write!(f, "(tuple {destination})")?,
                }
                f.write_str(" (fields")?;
                for value in values {
                    write!(f, " {value}")?;
                }
            }
        }
        f.write_str("))")
    }
}
