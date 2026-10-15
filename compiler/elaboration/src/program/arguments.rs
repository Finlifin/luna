//! Declaration routing and source-ordered preparation of tuple collections.
use super::*;

pub(super) struct RoutedArgument<'h> {
    pub parameter: usize,
    values: Vec<Expr<'h>>,
    positions: Vec<usize>,
    collected: bool,
    lambda: bool,
}

impl RoutedArgument<'_> {
    pub(super) fn is_collected(&self) -> bool {
        self.collected
    }
    pub(super) fn source_inputs(&self) -> impl Iterator<Item = (usize, &Expr<'_>)> {
        self.positions.iter().copied().zip(&self.values)
    }
    pub(super) fn single_position(&self) -> Option<usize> {
        (!self.collected).then(|| self.positions[0])
    }
    pub fn expression<'a>(&'a self, call: &Expr<'_>) -> Expr<'a> {
        if self.collected {
            Expr {
                hir_id: call.hir_id,
                kind: H::Tuple(&self.values),
                span: call.span,
            }
        } else if self.lambda {
            Expr {
                hir_id: self.values[0].hir_id,
                span: self.values[0].span,
                kind: H::TacticArgument(&self.values[0]),
            }
        } else {
            self.values[0].clone()
        }
    }

    pub fn require_collection_type(
        &self,
        checker: &Checker<'_, '_>,
        ty: &Term,
        span: Span,
    ) -> Result<(), Error> {
        // Collection constructs a tuple VALUE. In particular `...items: Type`
        // must not turn the synthetic tuple syntax into a type expression.
        if self.collected && !matches!(checker.structural_type(ty, span)?, Term::Tuple(_)) {
            return Err(error(
                span,
                "default variadic collection requires a tuple type",
            ));
        }
        Ok(())
    }
}

// Function values currently retain only the ordinary callable type. Refuse to
// silently erase declaration-side routing until that metadata has a carrier.
pub(super) fn require_function_value_parameters(
    parameters: &[hir::common::FnParam<'_>],
    span: Span,
) -> Result<(), Error> {
    for parameter in parameters {
        let kind = if parameter.ty.is_lambda() {
            "lambda-marked"
        } else if parameter.ty.is_implicit() {
            "implicit"
        } else {
            match parameter.ty.kind {
                TyParamKind::Optional(..) => "optional",
                TyParamKind::Varadic(..) => "variadic",
                _ => continue,
            }
        };
        return Err(error(
            span,
            format!(
                "{kind} function values require parameter protocol metadata not yet implemented"
            ),
        ));
    }
    Ok(())
}

pub(super) fn post_lambda_target(
    args: &[Arg<'_>],
    declarations: Option<&[hir::common::FnParam<'_>]>,
    count: usize,
    span: Span,
) -> Result<Option<usize>, Error> {
    let supplied = args
        .iter()
        .filter(|arg| matches!(arg, Arg::PostLambda(_)))
        .count();
    if supplied == 0 {
        return Ok(None);
    }
    if supplied != 1 {
        return Err(error(span, "post do parameter supplied more than once"));
    }
    if let Some(parameters) = declarations {
        let mut marked = parameters
            .iter()
            .enumerate()
            .filter(|(_, parameter)| parameter.ty.is_lambda());
        if let Some((index, _)) = marked.next() {
            if marked.next().is_some() {
                return Err(error(span, "ambiguous post do lambda parameter"));
            }
            return Ok(Some(index));
        }
        return parameters
            .iter()
            .enumerate()
            .rev()
            .find(|(_, parameter)| {
                !parameter.ty.is_implicit()
                    && matches!(
                        parameter.ty.kind,
                        TyParamKind::Positional(_) | TyParamKind::PositionalDependencyCatched(..)
                    )
            })
            .map(|(index, _)| Some(index))
            .ok_or_else(|| error(span, "post do has no anchored parameter target"));
    }
    count
        .checked_sub(1)
        .map(Some)
        .ok_or_else(|| error(span, "post do has no anchored parameter target"))
}

pub(super) fn route_arguments<'h>(
    args: &[Arg<'h>],
    declarations: Option<&[hir::common::FnParam<'h>]>,
    count: usize,
    span: Span,
) -> Result<(Vec<RoutedArgument<'h>>, Vec<bool>), Error> {
    let post_target = post_lambda_target(args, declarations, count, span)?;
    let kind = |index: usize| declarations.map(|params| &params[index].ty.kind);
    let implicit = |index: usize| declarations.is_some_and(|params| params[index].ty.is_implicit());
    let optional =
        |index| implicit(index) || matches!(kind(index), Some(TyParamKind::Optional(..)));
    let variadic = |index| matches!(kind(index), Some(TyParamKind::Varadic(..)));
    let named: std::collections::HashMap<_, _> = declarations
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, p)| {
            p.ty.is_implicit()
                || matches!(
                    p.ty.kind,
                    TyParamKind::Optional(..) | TyParamKind::Varadic(..)
                )
        })
        .map(|(index, p)| (p.name.name, index))
        .collect();
    // Whole-value supply reserves its entry before positional routing, even
    // when written later. It cannot also be a positional collection target.
    let mut directly_supplied = vec![false; count];
    for argument in args {
        if let Arg::Named(name, _) = argument {
            let index = named.get(&name.name).copied().ok_or_else(|| {
                error(name.span, format!("no named parameter entry {}", name.name))
            })?;
            if std::mem::replace(&mut directly_supplied[index], true) {
                return Err(error(name.span, "parameter supplied more than once"));
            }
        }
    }
    let mut assigned = vec![false; count];
    let mut slots = vec![None; count];
    let mut routes: Vec<RoutedArgument<'h>> = Vec::new();
    let mut positional = 0;
    let mut remaining = args
        .iter()
        .filter(|arg| matches!(arg, Arg::Positional(_)))
        .count();
    let mut anchored_suffix = vec![0; count + 1];
    let mut variadic_suffix = vec![0; count + 1];
    for index in (0..count).rev() {
        variadic_suffix[index] =
            variadic_suffix[index + 1] + usize::from(variadic(index) && !directly_supplied[index]);
        anchored_suffix[index] = anchored_suffix[index + 1]
            + usize::from(!optional(index) && !variadic(index) && post_target != Some(index));
    }
    for (position, argument) in args.iter().enumerate() {
        let (parameter, value, collected) = match argument {
            Arg::Positional(value) => {
                while positional < count
                    && (post_target == Some(positional)
                        || optional(positional)
                        || (variadic(positional)
                            && (directly_supplied[positional]
                                || remaining <= anchored_suffix[positional + 1])))
                {
                    positional += 1;
                }
                if positional == count {
                    if (0..count).any(|index| variadic(index) && directly_supplied[index]) {
                        return Err(error(
                            span,
                            "parameter supplied more than once: positional inputs cannot append to a directly supplied collection",
                        ));
                    }
                    return Err(error(
                        span,
                        "argument count mismatch: too many positional inputs",
                    ));
                }
                let index = positional;
                let collected = variadic(index);
                if collected && variadic_suffix[index + 1] != 0 {
                    return Err(error(
                        span,
                        "multiple variadic positional collections require a routing protocol not yet implemented; supply complete named packages",
                    ));
                }
                if !collected {
                    positional += 1;
                }
                remaining -= 1;
                (index, *value, collected)
            }
            Arg::Named(name, value) => {
                let index = named.get(&name.name).copied().ok_or_else(|| {
                    error(name.span, format!("no named parameter entry {}", name.name))
                })?;
                (index, *value, false)
            }
            Arg::PostLambda(value) => (
                post_target.expect("post do target was checked"),
                *value,
                false,
            ),
            _ => {
                return Err(error(
                    span,
                    "expanded and implicit argument routing is not implemented",
                ));
            }
        };
        if let Some(slot) = slots[parameter] {
            let previous: &mut RoutedArgument<'h> = &mut routes[slot];
            if !collected || !previous.collected {
                return Err(error(value.span, "parameter supplied more than once"));
            }
            previous.values.push(value.clone());
            previous.positions.push(position);
        } else {
            slots[parameter] = Some(routes.len());
            routes.push(RoutedArgument {
                parameter,
                values: vec![value.clone()],
                positions: vec![position],
                collected,
                lambda: declarations.is_some_and(|parameters| parameters[parameter].ty.is_lambda()),
            });
        }
        assigned[parameter] = true;
    }
    for index in 0..count {
        if !assigned[index] && implicit(index) {
            return Err(error(
                span,
                "implicit parameter requires explicit named supply; automatic supply protocol is not yet implemented",
            ));
        } else if !assigned[index] && variadic(index) {
            routes.push(RoutedArgument {
                parameter: index,
                values: Vec::new(),
                positions: Vec::new(),
                collected: true,
                lambda: false,
            });
            assigned[index] = true;
        } else if !assigned[index] && !optional(index) {
            return Err(error(
                span,
                "argument count mismatch: missing anchored parameter",
            ));
        }
    }
    Ok((routes, assigned))
}

impl Checker<'_, '_> {
    pub(super) fn stage_collected_call(
        &mut self,
        mut call: n::Expr,
        routes: &[RoutedArgument<'_>],
        offset: usize,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if !routes.iter().any(|route| route.collected) {
            return Ok(call);
        }
        let K::Call {
            callee, arguments, ..
        } = &mut call.kind
        else {
            unreachable!()
        };
        // Only declarations carry collection protocols. These callee forms
        // have no runtime evaluation effects; the receiver is a prefix input.
        debug_assert!(matches!(
            callee.kind,
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
        ));
        // Generated bookkeeping has an empty source anchor. It must not share
        // a complete user expression's cleanup boundary with that expression.
        let anchor = n::SourceRange {
            start: call.source.start,
            end: call.source.start,
        };
        callee.source = anchor;
        let mut argument_slots = vec![0; arguments.len()];
        for (slot, argument) in arguments.iter().enumerate() {
            argument_slots[argument.parameter] = slot;
        }
        let input_count = offset
            + routes
                .iter()
                .map(|route| route.positions.len())
                .sum::<usize>();
        let mut statements = vec![None; input_count];
        let mut stage = |value: &mut n::Expr, position: usize| -> Result<(), Error> {
            let index = self.next_temporary_binding;
            self.next_temporary_binding = index
                .checked_add(1)
                .ok_or_else(|| error(span, "call temporary binding space exhausted"))?;
            let binding = n::LocalId {
                owner: u32::MAX,
                index,
            };
            let local = n::Expr {
                ty: value.ty.clone(),
                source: anchor,
                kind: K::Local(binding),
            };
            let input = std::mem::replace(value, local);
            statements[position] = Some(n::Expr {
                ty: if is_never(&input.ty) {
                    p(P::Never)
                } else {
                    p(P::Void)
                },
                source: anchor,
                kind: K::Let {
                    binding,
                    value: Box::new(input),
                },
            });
            Ok(())
        };
        for argument in arguments.iter_mut().take(offset) {
            stage(&mut argument.value, argument.parameter)?;
        }
        for route in routes {
            let argument = &mut arguments[argument_slots[offset + route.parameter]];
            if route.collected {
                argument.value.source = anchor;
                let K::Tuple(items) = &mut argument.value.kind else {
                    return Err(error(
                        span,
                        "default variadic collection requires a tuple value",
                    ));
                };
                debug_assert_eq!(items.len(), route.positions.len());
                for (item, position) in items.iter_mut().zip(&route.positions) {
                    stage(item, offset + position)?;
                }
            } else {
                stage(&mut argument.value, offset + route.positions[0])?;
            }
        }
        Ok(n::Expr {
            ty: call.ty.clone(),
            source: call.source,
            kind: K::CallPreparation {
                statements: statements
                    .into_iter()
                    .map(|statement| statement.expect("every explicit input is staged once"))
                    .collect(),
                result: Some(Box::new(call)),
            },
        })
    }
}
