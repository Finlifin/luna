//! Caller operands acquired before an omitted input determines their destination.
use super::*;
use comptime_interpretor::Value;
use std::borrow::Cow;

enum Operand {
    Literal,
    Completed {
        ty: Term,
        value: Value,
        packet: Option<n::LocalId>,
    },
}

#[derive(Default)]
pub(super) struct PendingInputs {
    slots: BTreeMap<usize, BTreeMap<usize, Operand>>,
}

impl PendingInputs {
    pub(super) fn contains(&self, slot: usize) -> bool {
        self.slots.contains_key(&slot)
    }

    pub(super) fn acquire(
        &mut self,
        checker: &mut Checker<'_, '_>,
        slot: usize,
        field: usize,
        input: &Expr<'_>,
    ) -> Result<(), Error> {
        let fields = self.slots.entry(slot).or_default();
        if fields.contains_key(&field) {
            return Ok(());
        }
        // Literals can wait for contextual checking. Executable producers must
        // run here, at their source position, before any declaration default.
        let operand = if checker.is_closed_input_literal(input)? {
            Operand::Literal
        } else {
            let checked = checker
                .with_staged_input(true, |this| this.argument_attempt(input, None, None))?
                .ok_or_else(|| {
                    context_error(
                        input.span,
                        "operand needs its pending parameter type before execution",
                    )
                })?;
            let ty = checked.ty.clone();
            let packet = match checked.kind {
                K::Local(binding) => checker
                    .spread_input_value(binding, input)?
                    .map(|v| (v, binding)),
                _ => None,
            };
            let (value, packet) = match packet {
                Some((value, binding)) => (value, Some(binding)),
                None => (
                    checker.required_normal_input(input, checked, vec![])?.0,
                    None,
                ),
            };
            Operand::Completed { ty, value, packet }
        };
        fields.insert(field, operand);
        Ok(())
    }

    /// Supply inference or gradual admission with caller-side type evidence.
    /// Only a delayed literal can still need context; never revisit a producer.
    pub(super) fn actual_domain(
        &self,
        checker: &mut Checker<'_, '_>,
        route: &arguments::RoutedArgument<'_>,
        span: Span,
        allow_numeric_defaults: bool,
    ) -> Result<Option<Term>, Error> {
        let mut types = Vec::new();
        for (field, (_, input)) in route.source_inputs().enumerate() {
            let ty = match &self.slots[&route.parameter][&field] {
                Operand::Completed { ty, .. } => ty.clone(),
                Operand::Literal => {
                    let Some(checked) = checker.with_staged_input(true, |this| {
                        this.argument_attempt_with_defaults(
                            input,
                            None,
                            None,
                            allow_numeric_defaults,
                        )
                    })?
                    else {
                        return Ok(None);
                    };
                    checked.ty
                }
            };
            types.push(ty);
        }
        if route.is_collected() {
            checker.tuple_value_type(types, span).map(Some)
        } else {
            Ok(types.pop())
        }
    }

    pub(super) fn deliver(
        &mut self,
        checker: &mut Checker<'_, '_>,
        route: &arguments::RoutedArgument<'_>,
        ty: &Term,
        call: &Expr<'_>,
    ) -> Result<(n::Expr, Option<Term>, Value), Error> {
        route.require_collection_type(checker, ty, call.span)?;
        let mut operands = self.slots.remove(&route.parameter).expect("pending slot");
        let mut domains = if route.is_collected() {
            let Term::Tuple(fields) = checker.structural_type(ty, call.span)? else {
                unreachable!("checked collection domain")
            };
            if fields.len() != operands.len() {
                return Err(error(
                    call.span,
                    "variadic collection argument count mismatch",
                ));
            }
            fields.into_iter()
        } else {
            vec![ty.clone()].into_iter()
        };
        let mut fields = Vec::new();
        let mut symbols = Vec::new();
        let mut values = Vec::new();
        for (field, (_, input)) in route.source_inputs().enumerate() {
            let domain = domains.next().expect("checked arity");
            let operand = operands.remove(&field).expect("acquired source field");
            let (expr, value) = match operand {
                Operand::Literal => {
                    let expr =
                        checker.with_staged_input(true, |this| this.expr(input, Some(&domain)))?;
                    let (value, actual) = checker.required_normal_input(input, expr, vec![])?;
                    (
                        checker.materialize_local_constant(value.clone(), actual, input.span)?,
                        value,
                    )
                }
                Operand::Completed { ty, value, packet } => {
                    let binding = match packet {
                        Some(binding) => binding,
                        None => checker.spread_binding(input.span)?,
                    };
                    let local = n::Expr {
                        kind: K::Local(binding),
                        ty: ty.clone(),
                        source: source(input.span),
                    };
                    let adapted = checker.deliver_computed_value(&local, &domain, input.span)?;
                    let value = match &adapted {
                        Cow::Borrowed(_) => value,
                        Cow::Owned(expr) => {
                            checker
                                .required_live_input(
                                    input,
                                    expr.clone(),
                                    vec![],
                                    vec![(binding, ty, value)],
                                )?
                                .0
                        }
                    };
                    let expr = if packet.is_some() {
                        adapted.into_owned()
                    } else {
                        checker.materialize_local_constant(value.clone(), domain, input.span)?
                    };
                    (expr, value)
                }
            };
            let symbol = checker.optional_normal_term(value.clone(), input.span)?;
            if !route.is_collected() {
                return Ok((expr, symbol, value));
            }
            fields.push(expr);
            symbols.push(symbol);
            values.push(value);
        }
        Ok((
            n::Expr {
                kind: K::Tuple(fields),
                ty: ty.clone(),
                source: source(call.span),
            },
            symbols
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .map(Term::TupleValue),
            Value::Tuple(values),
        ))
    }
}
