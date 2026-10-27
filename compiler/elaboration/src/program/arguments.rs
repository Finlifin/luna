//! Declaration routing and source-ordered preparation of tuple collections.
use super::*;

/// A preparation may exit before delivering an already checked operation.
/// Its completion type stays Never, while the operation's formed result can
/// still constrain generic inputs. Do not infer a type from an arbitrary
/// divergent expression or inspect function bodies.
pub(super) fn inference_type(mut value: &n::Expr) -> Option<&Term> {
    for _ in 0..64 {
        if !is_never(&value.ty) {
            return Some(&value.ty);
        }
        let K::CallPreparation {
            result: Some(result),
            ..
        } = &value.kind
        else {
            return None;
        };
        value = result;
    }
    None
}

pub(super) struct RoutedArgument<'h> {
    pub parameter: usize,
    values: Vec<Expr<'h>>,
    positions: Vec<usize>,
    collected: bool,
    lambda: bool,
}

impl<'h> RoutedArgument<'h> {
    /// A literal whole packet has no producer around its fields. Prepare those
    /// fields in the ordinary collection loop without constructing a second
    /// tuple layer. Callers must first establish that this is a variadic slot.
    pub(super) fn expand_literal_packet(&mut self, remaining: &mut u64) -> Result<(), Error> {
        if self.collected || self.lambda || self.values.len() != 1 {
            return Ok(());
        }
        let H::Tuple(fields) = self.values[0].kind else {
            return Ok(());
        };
        *remaining = remaining.checked_sub(fields.len() as u64).ok_or_else(|| {
            error(
                self.values[0].span,
                "literal packet preparation budget exhausted",
            )
        })?;
        let position = self.positions[0];
        self.values = fields.to_vec();
        self.positions = vec![position; fields.len()];
        self.collected = true;
        Ok(())
    }

    pub(super) fn is_collected(&self) -> bool {
        self.collected
    }
    pub(super) fn property_fields(&self) -> Option<&[hir::expr::FieldExpr<'h>]> {
        if self.collected {
            return None;
        }
        match self.values.first()?.kind {
            H::CollectedProperties(fields) => Some(fields),
            _ => None,
        }
    }
    pub(super) fn has_collection_fields(&self) -> bool {
        self.collected || self.property_fields().is_some()
    }
    pub(super) fn retained_collection_context(
        &self,
        checker: &Checker<'_, '_>,
        values: &[Option<n::Expr>],
        domain: &crate::calls::InputContext,
    ) -> crate::calls::InputContext {
        use crate::calls::InputContext as C;
        let fields = values
            .iter()
            .map(|value| {
                value
                    .as_ref()
                    .filter(|value| !is_never(&value.ty))
                    .map_or(C::Unknown, |value| C::Known(value.ty.clone()))
            })
            .collect::<Vec<_>>();
        match self.property_fields() {
            Some(properties) => {
                let fields: Vec<_> = properties
                    .iter()
                    .zip(fields)
                    .map(|(property, field)| (property.ident.name.to_string(), field))
                    .collect();
                let target_names: Option<Vec<&str>> = match domain {
                    C::Record(target) => {
                        Some(target.iter().map(|(name, _)| name.as_str()).collect())
                    }
                    C::Known(Term::Record(target)) => {
                        Some(target.iter().map(|(name, _)| name.as_str()).collect())
                    }
                    _ => None,
                };
                if let Some(target) = target_names {
                    let names: BTreeSet<_> = fields.iter().map(|(name, _)| name.as_str()).collect();
                    if names.len() == fields.len()
                        && target.len() == fields.len()
                        && target.iter().copied().collect::<BTreeSet<_>>() == names
                    {
                        let mut by_name: BTreeMap<_, _> = fields.into_iter().collect();
                        return C::Record(
                            target
                                .iter()
                                .map(|name| (name.to_string(), by_name.remove(*name).unwrap()))
                                .collect(),
                        );
                    }
                }
                C::Record(fields)
            }
            None => match checker.protocols.tuple {
                Some(head) => C::NormalTuple { head, fields },
                None => C::Tuple(fields),
            },
        }
    }
    pub(super) fn source_inputs(&self) -> impl Iterator<Item = (usize, &Expr<'h>)> {
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

    pub(super) fn local_collection_context(
        &self,
        checker: &Checker<'_, '_>,
    ) -> Option<crate::calls::InputContext> {
        use crate::calls::InputContext as C;
        if !self.collected {
            return None;
        }
        let fields = self
            .source_inputs()
            .map(|(_, input)| {
                let ty = match input.kind {
                    H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                        checker.locals.get(&local(id))
                    }
                    _ => None,
                };
                ty.filter(|ty| !is_never(ty))
                    .map_or(C::Unknown, |ty| C::Known(ty.clone()))
            })
            .collect::<Vec<_>>();
        if fields.iter().all(|field| matches!(field, C::Unknown)) {
            return None;
        }
        Some(match checker.protocols.tuple {
            Some(head) => C::NormalTuple { head, fields },
            None => C::Tuple(fields),
        })
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
    pub(super) fn attempt_collection_fields(
        &mut self,
        call: &Expr<'_>,
        route: &RoutedArgument<'_>,
        context: &crate::calls::InputContext,
        values: &mut Vec<Option<n::Expr>>,
        staged: bool,
        allow_defaults: bool,
    ) -> Result<(Option<n::Expr>, bool), Error> {
        use crate::calls::InputContext as C;
        let properties = route.property_fields();
        let inputs = match properties {
            Some(properties) => properties
                .iter()
                .map(|property| property.expr)
                .collect::<Vec<_>>(),
            None => route.source_inputs().map(|(_, input)| input).collect(),
        };
        let count = inputs.len();
        let mut record_order = (0..count).collect::<Vec<_>>();
        let mut record_context = |fields: Vec<(String, C)>| -> Result<Vec<C>, Error> {
            let Some(properties) = properties else {
                return Err(error(
                    call.span,
                    "default variadic collection requires a tuple type",
                ));
            };
            let names = fields
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            record_order =
                super::extended::layout::property_order(properties, Some(&names), call.span)?;
            let mut fields: BTreeMap<_, _> = fields.into_iter().collect();
            Ok(properties
                .iter()
                .map(|property| fields.remove(property.ident.name.as_str()).unwrap())
                .collect())
        };
        let fields = match context {
            C::Known(ty) => match self.structural_type(ty, call.span)? {
                Term::Record(fields) if properties.is_some() => record_context(
                    fields
                        .into_iter()
                        .map(|(name, ty)| (name, C::Known(ty)))
                        .collect(),
                )?,
                Term::Tuple(fields) if properties.is_none() => {
                    fields.into_iter().map(C::Known).collect()
                }
                _ => {
                    return Err(error(
                        call.span,
                        "collection domain does not match its input protocol",
                    ));
                }
            },
            C::Record(fields) => record_context(fields.clone())?,
            C::Tuple(fields) | C::NormalTuple { fields, .. } if properties.is_none() => {
                fields.clone()
            }
            C::Unknown => vec![C::Unknown; count],
            _ => {
                return Err(error(
                    call.span,
                    "collection domain does not match its input protocol",
                ));
            }
        };
        if fields.len() != count {
            return Err(error(
                call.span,
                "variadic collection argument count mismatch",
            ));
        }
        values.resize_with(count, || None);
        let mut progress = false;
        for (input, (value, context)) in inputs.into_iter().zip(values.iter_mut().zip(&fields)) {
            if value.is_some() {
                continue;
            }
            let expected = match context {
                C::Known(ty) => Some(ty),
                _ => None,
            };
            *value = self.with_staged_input(staged, |this| {
                this.argument_attempt_with_input(
                    input,
                    expected,
                    None,
                    allow_defaults,
                    Some(context),
                )
            })?;
            if value.is_some() {
                progress = true;
                // Recompute the candidate before contextualizing or defaulting
                // another element; a peer can determine its domain.
                break;
            }
        }
        if values.iter().any(Option::is_none) {
            return Ok((None, progress));
        }
        let values = values
            .iter()
            .map(|value| value.as_ref().unwrap().clone())
            .collect::<Vec<_>>();
        let (ty, kind) = if let Some(properties) = properties {
            let types = properties
                .iter()
                .zip(values.iter().zip(&fields))
                .map(|(property, (value, context))| {
                    let ty = if is_never(&value.ty) {
                        match context {
                            C::Known(ty) => ty.clone(),
                            _ => value.ty.clone(),
                        }
                    } else {
                        value.ty.clone()
                    };
                    (property.ident.name.to_string(), ty)
                })
                .collect();
            return Ok((
                Some(self.finish_property_packet(call.span, types, values, &record_order)?),
                true,
            ));
        } else {
            let ty = if values.iter().any(|value| is_never(&value.ty)) {
                p(P::Never)
            } else {
                self.tuple_value_type(
                    values.iter().map(|value| value.ty.clone()).collect(),
                    call.span,
                )?
            };
            (ty, K::Tuple(values))
        };
        Ok((
            Some(n::Expr {
                ty,
                source: source(call.span),
                kind,
            }),
            true,
        ))
    }

    pub(super) fn stage_collected_call(
        &mut self,
        mut call: n::Expr,
        routes: &[RoutedArgument<'_>],
        offset: usize,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let completed_defaults = matches!(&call.kind, K::Call { arguments, .. }
            if arguments.iter().any(|argument| argument.parameter >= offset
                && !argument.is_default
                && !routes.iter().any(|route| offset + route.parameter == argument.parameter)));
        if !routes.iter().any(|route| route.collected) && !completed_defaults {
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

/// Flatten only syntax-level tuple packets. Element expressions retain their
/// destination phase; computed packets need the checked preparation path.
pub(super) fn literal_spread_arguments<'hir>(args: &[Arg<'hir>]) -> Option<Vec<Arg<'hir>>> {
    let mut flat = Vec::new();
    for arg in args {
        match arg {
            Arg::Expand(value) => match value.kind {
                H::Tuple(items) => flat.extend(items.iter().map(Arg::Positional)),
                H::Unit => {}
                _ => return None,
            },
            other => flat.push(other.clone()),
        }
    }
    Some(flat)
}
