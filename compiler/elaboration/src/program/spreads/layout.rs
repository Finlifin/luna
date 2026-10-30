//! Read-only packet layout hints. Hints select element contexts, never establish
//! a type equality or replace checking the packet expression itself.
use super::*;

// How a sole source input supplies the binding whose type is returned.
// This is a layout hint, not a substitute for call routing validation.
enum PreservedInput {
    Positional,
    Named(hir::common::Ident),
    Collected(hir::common::Ident),
}

impl Checker<'_, '_> {
    pub(in crate::program) fn spread_input_packet_contexts(
        &mut self,
        args: &[Arg<'_>],
        types: &[crate::calls::InputContext],
        span: Span,
    ) -> Result<(Vec<Option<Term>>, Vec<Option<crate::calls::InputContext>>), Error> {
        self.spread_input_packet_contexts_with_widths(args, types, span, &BTreeMap::new())
    }

    pub(in crate::program) fn checked_packet_widths(
        &self,
        packets: &BTreeMap<usize, n::Expr>,
        span: Span,
    ) -> Result<BTreeMap<usize, usize>, Error> {
        let mut widths = BTreeMap::new();
        for (index, packet) in packets {
            if let Some(width) = self.tuple_width(&packet.ty, span)? {
                widths.insert(*index, width);
            }
        }
        Ok(widths)
    }

    pub(in crate::program) fn spread_input_packet_contexts_with_widths(
        &mut self,
        args: &[Arg<'_>],
        types: &[crate::calls::InputContext],
        span: Span,
        widths: &BTreeMap<usize, usize>,
    ) -> Result<(Vec<Option<Term>>, Vec<Option<crate::calls::InputContext>>), Error> {
        use crate::calls::InputContext as C;
        let mut complete = Vec::new();
        let mut partial = Vec::new();
        for fields in self.spread_packet_layout_with_widths(args, types, span, widths)? {
            let Some(fields) = fields else {
                complete.push(None);
                partial.push(None);
                continue;
            };
            let known = fields
                .iter()
                .map(|field| match field {
                    C::Known(ty) => Some(ty.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>();
            if let Some(fields) = known {
                complete.push(Some(self.tuple_value_type(fields, span)?));
                partial.push(None);
            } else {
                complete.push(None);
                partial.push(Some(match self.protocols.tuple {
                    Some(head) => C::NormalTuple {
                        head,
                        fields,
                        list_domain: self.bound_tuple_list_domain(),
                    },
                    None => C::Tuple(fields),
                }));
            }
        }
        Ok((complete, partial))
    }

    // Keep a partial structural context when only some packet domains are
    // known. Inference state stays outside Term and never enters typed NIR.
    pub(in crate::program) fn spread_partial_packet_contexts(
        &mut self,
        args: &[Arg<'_>],
        types: &[Option<Term>],
        span: Span,
    ) -> Result<(Vec<Option<Term>>, Vec<Option<crate::calls::InputContext>>), Error> {
        use crate::calls::InputContext as C;
        self.spread_input_packet_contexts(
            args,
            &types
                .iter()
                .cloned()
                .map(|ty| ty.map_or(C::Unknown, C::Known))
                .collect::<Vec<_>>(),
            span,
        )
    }

    pub(in crate::program) fn spread_packet_contexts(
        &mut self,
        args: &[Arg<'_>],
        types: &[Term],
        span: Span,
    ) -> Result<Vec<Option<Term>>, Error> {
        self.spread_packet_layout(args, types, span)?
            .into_iter()
            .map(|fields| {
                fields
                    .map(|fields| self.tuple_value_type(fields, span))
                    .transpose()
            })
            .collect()
    }

    pub(in crate::program) fn spread_packet_layout<T: Clone>(
        &mut self,
        args: &[Arg<'_>],
        types: &[T],
        span: Span,
    ) -> Result<Vec<Option<Vec<T>>>, Error> {
        self.spread_packet_layout_with_widths(args, types, span, &BTreeMap::new())
    }

    pub(in crate::program) fn spread_packet_layout_with_widths<T: Clone>(
        &mut self,
        args: &[Arg<'_>],
        types: &[T],
        span: Span,
        settled: &BTreeMap<usize, usize>,
    ) -> Result<Vec<Option<Vec<T>>>, Error> {
        let mut fuel = 10_000;
        let mut widths = Vec::with_capacity(args.len());
        for (index, argument) in args.iter().enumerate() {
            widths.push(match argument {
                Arg::Positional(_) => Some(1),
                Arg::Named(..) | Arg::Implicit(_) | Arg::PostLambda(_) => Some(0),
                Arg::Expand(value) => match settled.get(&index) {
                    Some(width) => Some(*width),
                    None => self.spread_width(value, &mut fuel, 0)?,
                },
            });
        }
        let known = widths.iter().flatten().try_fold(0usize, |sum, width| {
            sum.checked_add(*width)
                .ok_or_else(|| error(span, "spread layout size exhausted"))
        })?;
        if known > types.len() {
            return Err(error(
                span,
                "argument count mismatch: spread tuple widths exceed positional slots",
            ));
        }
        let unknown: Vec<_> = widths
            .iter()
            .enumerate()
            .filter_map(|(index, width)| width.is_none().then_some(index))
            .collect();
        if let [index] = unknown.as_slice() {
            widths[*index] = Some(types.len() - known);
        }
        // Both ends can provide unambiguous contexts even when two interior
        // widths remain unknown. Never choose a partition for those interiors.
        let mut contexts = vec![None; args.len()];
        let mut start = 0usize;
        for (index, width) in widths.iter().enumerate() {
            let Some(width) = width else { break };
            let end = start + width;
            if let Arg::Expand(_) = args[index] {
                contexts[index] = Some(types[start..end].to_vec());
            }
            start = end;
        }
        let mut end = types.len();
        for (index, width) in widths.iter().enumerate().rev() {
            let Some(width) = width else { break };
            let start = end - width;
            if contexts[index].is_none() && matches!(args[index], Arg::Expand(_)) {
                contexts[index] = Some(types[start..end].to_vec());
            }
            end = start;
        }
        Ok(contexts)
    }

    pub(in crate::program) fn spread_width(
        &mut self,
        value: &Expr<'_>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<usize>, Error> {
        if *fuel == 0 || depth >= 128 {
            return Err(error(
                value.span,
                "spread layout inference budget exhausted",
            ));
        }
        *fuel -= 1;
        match &value.kind {
            H::Tuple(items) => Ok(Some(items.len())),
            H::Unit => Ok(Some(0)),
            // Layout is evidence shared by every branch, never a reason to
            // invoke a staged selector while describing an open definition.
            H::InlineControl(inner) => self.spread_width(inner, fuel, depth + 1),
            H::Block(block) => match block.expr {
                Some(result) => self.spread_width(result, fuel, depth + 1),
                None => Ok(None),
            },
            H::If(_, yes, Some(no)) => {
                let Some(yes) = yes.expr else { return Ok(None) };
                let left = self.spread_width(yes, fuel, depth + 1)?;
                let right = self.spread_width(no, fuel, depth + 1)?;
                Ok(left.filter(|_| left == right))
            }
            H::Match(_, arms) => {
                self.spread_branch_width(arms.iter().map(|arm| arm.body), fuel, depth + 1)
            }
            H::When(arms) => {
                self.spread_branch_width(arms.iter().map(|arm| arm.body), fuel, depth + 1)
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let Some(ty) = self.locals.get(&local(*id)).cloned() else {
                    return Ok(None);
                };
                self.tuple_width(&ty, value.span)
            }
            H::Application(callee, arguments) => {
                let head = if let H::NFApplication(head, _) = callee.kind {
                    head
                } else {
                    callee
                };
                let (signature, generic, collection, definition) = match head.kind {
                    H::ResolvedDef { def_id, .. } => {
                        let Some(owner) = self.package.definition_owner(def_id) else {
                            return Ok(None);
                        };
                        if !matches!(
                            self.package.item(owner).map(|item| &item.kind),
                            Some(ItemKind::Fn(..))
                        ) {
                            return Ok(None);
                        }
                        let collection = match self.package.item(owner).map(|item| &item.kind) {
                            Some(ItemKind::Fn(declaration, _)) => match declaration.params {
                                [parameter]
                                    if !parameter.ty.is_quote()
                                        && !parameter.ty.is_assoc()
                                        && !parameter.ty.is_lambda()
                                        && !parameter.ty.is_catch() =>
                                {
                                    match parameter.ty.kind {
                                        _ if parameter.ty.is_implicit() => {
                                            Some(PreservedInput::Named(parameter.name))
                                        }
                                        TyParamKind::Optional(..) => {
                                            Some(PreservedInput::Named(parameter.name))
                                        }
                                        TyParamKind::Positional(_)
                                        | TyParamKind::PositionalDependencyCatched(..) => {
                                            Some(PreservedInput::Positional)
                                        }
                                        TyParamKind::Varadic(..) => {
                                            Some(PreservedInput::Collected(parameter.name))
                                        }
                                        _ => None,
                                    }
                                }
                                _ => None,
                            },
                            _ => None,
                        };
                        self.ensure_signature(owner, value.span)?;
                        let Some(signature) = self.signatures.get(&owner) else {
                            return Ok(None);
                        };
                        (
                            signature.clone(),
                            self.type_parameters
                                .get(&owner)
                                .is_some_and(|types| !types.is_empty()),
                            collection,
                            Some(owner),
                        )
                    }
                    H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                        let Some(ty) = self.locals.get(&local(id)).cloned() else {
                            return Ok(None);
                        };
                        (self.structural_type(&ty, value.span)?, false, None, None)
                    }
                    _ => return Ok(None),
                };
                let signature = if let Term::Closure { signature, .. } = &signature {
                    signature.as_ref()
                } else {
                    &signature
                };
                let Term::Function { parameters, result } = signature.callable_signature() else {
                    return Ok(None);
                };
                if let (Some(owner), H::NFApplication(_, supplied)) = (definition, &callee.kind) {
                    if let Some(width) = self.explicit_result_packet_width(
                        owner,
                        result,
                        supplied,
                        fuel,
                        depth + 1,
                        value.span,
                    )? {
                        return Ok(Some(width));
                    }
                }
                if let Some(owner) = definition.filter(|_| generic) {
                    if let Some(width) =
                        self.required_packet_width(owner, result, fuel, value.span)?
                    {
                        return Ok(Some(width));
                    }
                }
                if let Term::Tuple(fields) = result.as_ref() {
                    Ok(Some(fields.len()))
                } else if let Some(fields) = self.bound_tuple_elements(result) {
                    // The explicit list fixes arity even while its element
                    // types contain this callee's uninstantiated binders.
                    Ok(Some(fields.len()))
                } else if generic && parameters.len() == 1 && parameters[0].ty == **result {
                    match collection {
                        Some(PreservedInput::Positional) => {
                            // A single ordinary binding has the source input's type.
                            if let [Arg::Positional(input)] = *arguments {
                                self.spread_width(input, fuel, depth + 1)
                            } else {
                                Ok(None)
                            }
                        }
                        Some(PreservedInput::Named(name)) => {
                            if let [Arg::Named(supplied, input)] = *arguments {
                                if supplied.name == name.name {
                                    return self.spread_width(input, fuel, depth + 1);
                                }
                            }
                            Ok(None)
                        }
                        Some(PreservedInput::Collected(name)) => {
                            // Explicit named supply is already the complete
                            // packet; it does not add a collection layer.
                            if let [Arg::Named(supplied, input)] = *arguments {
                                return if supplied.name == name.name {
                                    self.spread_width(input, fuel, depth + 1)
                                } else {
                                    Ok(None)
                                };
                            }
                            // A variadic binding has the collected packet's type.
                            // One tuple input is one field unless explicitly spread.
                            let mut width = 0usize;
                            for argument in *arguments {
                                *fuel = fuel.checked_sub(1).ok_or_else(|| {
                                    error(value.span, "spread layout inference budget exhausted")
                                })?;
                                let amount = match argument {
                                    Arg::Positional(_) => 1,
                                    Arg::Expand(input) if matches!(input.kind, H::Unit) => 0,
                                    Arg::Expand(input) => {
                                        let Some(width) =
                                            self.spread_width(input, fuel, depth + 1)?
                                        else {
                                            return Ok(None);
                                        };
                                        width
                                    }
                                    _ => return Ok(None),
                                };
                                width = width.checked_add(amount).ok_or_else(|| {
                                    error(value.span, "spread layout size exhausted")
                                })?;
                            }
                            Ok(Some(width))
                        }
                        None => Ok(None),
                    }
                } else if generic {
                    // Read only checked signature relations. An unrelated
                    // result binder remains unknown; never inspect its body.
                    if let Some(owner) = definition {
                        self.preserved_input_width(
                            owner,
                            parameters,
                            result,
                            arguments,
                            fuel,
                            depth + 1,
                            value.span,
                        )
                    } else {
                        Ok(None)
                    }
                } else {
                    self.tuple_width(result, value.span)
                }
            }
            _ => Ok(None),
        }
    }

    fn explicit_result_packet_width(
        &mut self,
        owner: OwnerId,
        result: &Term,
        arguments: &[Arg<'_>],
        fuel: &mut usize,
        depth: usize,
        span: Span,
    ) -> Result<Option<usize>, Error> {
        use hir::clause::ClauseParamKind;
        let Term::Rigid {
            declaration,
            sort: Sort::Type,
        } = result
        else {
            return Ok(None);
        };
        let Some(parameters) = self.type_parameters.get(&owner) else {
            return Ok(None);
        };
        let Some(ItemKind::Fn(signature, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        let clauses = signature.clause_params;
        let exhausted = || error(span, "spread layout inference budget exhausted");
        *fuel = fuel.checked_sub(parameters.len()).ok_or_else(exhausted)?;
        if clauses.len() != parameters.len() {
            return Ok(None);
        }
        let Some(target) = parameters
            .iter()
            .position(|parameter| parameter == declaration)
        else {
            return Ok(None);
        };
        *fuel = fuel.checked_sub(arguments.len()).ok_or_else(exhausted)?;
        let mut pending: std::collections::VecDeque<_> = arguments.iter().cloned().collect();
        let mut supplied = vec![None; clauses.len()];
        let mut positional = 0;
        while let Some(argument) = pending.pop_front() {
            *fuel = fuel.checked_sub(1).ok_or_else(exhausted)?;
            let (slot, value) = match argument {
                Arg::Positional(value) => {
                    while positional < clauses.len()
                        && matches!(clauses[positional].kind, ClauseParamKind::Optional(..))
                    {
                        *fuel = fuel.checked_sub(1).ok_or_else(exhausted)?;
                        positional += 1;
                    }
                    let slot = positional;
                    positional += 1;
                    (slot, value)
                }
                Arg::Named(name, value) => {
                    *fuel = fuel.checked_sub(clauses.len()).ok_or_else(exhausted)?;
                    let Some(slot) = clauses.iter().position(|clause| {
                        clause.name.name == name.name
                            && matches!(clause.kind, ClauseParamKind::Optional(..))
                    }) else {
                        return Ok(None);
                    };
                    (slot, value)
                }
                Arg::Expand(value) => {
                    match value.kind {
                        H::Unit => {}
                        H::Tuple(fields) => {
                            *fuel = fuel.checked_sub(fields.len()).ok_or_else(exhausted)?;
                            for field in fields.iter().rev() {
                                pending.push_front(Arg::Positional(field));
                            }
                        }
                        _ => return Ok(None),
                    }
                    continue;
                }
                _ => return Ok(None),
            };
            if slot >= clauses.len()
                || supplied[slot].is_some()
                || matches!(
                    clauses[slot].kind,
                    ClauseParamKind::Quote(_) | ClauseParamKind::Varadic(..)
                )
            {
                return Ok(None);
            }
            supplied[slot] = Some(value);
        }
        let Some(value) = supplied[target] else {
            return Ok(None);
        };
        // Read only explicit tuple type syntax. Type-valued computations are
        // left to normal argument preparation, never invoked by this hint.
        self.explicit_tuple_type_width(value, fuel, depth)
    }

    fn explicit_tuple_type_width(
        &mut self,
        value: &Expr<'_>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<usize>, Error> {
        if depth >= 128 || *fuel == 0 {
            return Err(error(
                value.span,
                "spread layout inference budget exhausted",
            ));
        }
        *fuel -= 1;
        match value.kind {
            H::Tuple(fields) => Ok(Some(fields.len())),
            H::NFApplication(head, arguments) => {
                let H::ResolvedDef { def_id, .. } = head.kind else {
                    return Ok(None);
                };
                let Some(owner) = self.package.definition_owner(def_id) else {
                    return Ok(None);
                };
                if Some(declaration(self.package, &self.package_identities, owner))
                    != self.protocols.tuple
                {
                    return Ok(None);
                }
                let mut width = 0usize;
                for argument in arguments {
                    *fuel = fuel.checked_sub(1).ok_or_else(|| {
                        error(value.span, "spread layout inference budget exhausted")
                    })?;
                    let amount = match argument {
                        Arg::Positional(_) => 1,
                        Arg::Expand(value) => {
                            let Some(width) = self.completed_list_width(value, fuel, depth + 1)?
                            else {
                                return Ok(None);
                            };
                            width
                        }
                        Arg::Named(name, value) if arguments.len() == 1 => {
                            let Some(ItemKind::Fn(signature, _)) =
                                self.package.item(owner).map(|item| &item.kind)
                            else {
                                return Ok(None);
                            };
                            if !matches!(signature.clause_params, [clause] if clause.name.name == name.name)
                            {
                                return Ok(None);
                            }
                            let Some(width) = self.completed_list_width(value, fuel, depth + 1)?
                            else {
                                return Ok(None);
                            };
                            width
                        }
                        _ => return Ok(None),
                    };
                    width = width
                        .checked_add(amount)
                        .ok_or_else(|| error(value.span, "spread layout size exhausted"))?;
                }
                Ok(Some(width))
            }
            H::Block(block) => match block.expr {
                Some(value) => self.explicit_tuple_type_width(value, fuel, depth + 1),
                None => Ok(None),
            },
            H::If(_, yes, Some(no)) => {
                let Some(yes) = yes.expr else { return Ok(None) };
                let left = self.explicit_tuple_type_width(yes, fuel, depth + 1)?;
                let right = self.explicit_tuple_type_width(no, fuel, depth + 1)?;
                Ok(left.filter(|_| left == right))
            }
            _ => Ok(None),
        }
    }

    fn completed_list_width(
        &self,
        value: &Expr<'_>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<usize>, Error> {
        if *fuel == 0 || depth >= 128 {
            return Err(error(
                value.span,
                "spread layout inference budget exhausted",
            ));
        }
        *fuel -= 1;
        let known = match value.kind {
            H::List(fields) => return Ok(Some(fields.len())),
            H::ResolvedDef { def_id, .. } => {
                let owner = self.package.definition_owner(def_id);
                let known = owner.and_then(|owner| self.constants.get(&owner));
                if let Some(ItemKind::Const(_, initializer)) = owner
                    .and_then(|owner| self.package.item(owner))
                    .map(|item| &item.kind)
                {
                    if let Some(width) = self.completed_list_width(initializer, fuel, depth + 1)? {
                        return Ok(Some(width));
                    }
                }
                known
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let binding = local(id);
                // A preceding argument can change a mutable generator binding.
                // Its current contents cannot determine a later packet's width.
                if !self.immutable.contains(&binding) {
                    return Ok(None);
                }
                self.generator_data.get(&binding).and_then(Option::as_ref)
            }
            H::Block(block) => {
                return match block.expr {
                    Some(value) => self.completed_list_width(value, fuel, depth + 1),
                    None => Ok(None),
                };
            }
            H::If(_, yes, Some(no)) => {
                let Some(yes) = yes.expr else { return Ok(None) };
                let left = self.completed_list_width(yes, fuel, depth + 1)?;
                let right = self.completed_list_width(no, fuel, depth + 1)?;
                return Ok(left.filter(|_| left == right));
            }
            _ => None,
        };
        let Some(known) = known.filter(|value| self.is_type_list_type(&value.ty)) else {
            return Ok(None);
        };
        completed_list_constructor_width(known, &p(P::Type), fuel)
            .map_err(|message| error(value.span, message))
    }

    // Conjunctive equality requirements are available at every legal call.
    // Read their exact edges without unfolding heads or executing producers;
    // the ordinary call checker still proves all instantiated requirements.
    fn required_packet_width(
        &self,
        owner: OwnerId,
        result: &Term,
        fuel: &mut usize,
        span: Span,
    ) -> Result<Option<usize>, Error> {
        let exhausted = || error(span, "spread layout inference budget exhausted");
        let Some(requirements) = self.requirements.get(&owner) else {
            return Ok(None);
        };
        *fuel = fuel.checked_sub(requirements.len()).ok_or_else(exhausted)?;
        let mut predicates: Vec<_> = requirements.iter().collect();
        let mut edges = Vec::new();
        while let Some(predicate) = predicates.pop() {
            *fuel = fuel.checked_sub(1).ok_or_else(exhausted)?;
            match predicate {
                n::TypePredicate::Equal(left, right) => edges.push((left, right)),
                n::TypePredicate::And(parts) => {
                    *fuel = fuel.checked_sub(parts.len()).ok_or_else(exhausted)?;
                    predicates.extend(parts);
                }
                // Neither a negation nor an individual disjunct is a fact.
                _ => {}
            }
        }
        if edges.is_empty() {
            return Ok(None);
        }
        let mut pending = vec![result];
        let mut visited: Vec<&Term> = Vec::new();
        let mut width = None;
        while let Some(term) = pending.pop() {
            let mut seen = false;
            for previous in &visited {
                nir::type_computations::charge_term(term, fuel).map_err(|_| exhausted())?;
                nir::type_computations::charge_term(previous, fuel).map_err(|_| exhausted())?;
                if term == *previous {
                    seen = true;
                    break;
                }
            }
            if seen {
                continue;
            }
            visited.push(term);
            let candidate = if let Term::Tuple(fields) = term {
                Some(fields.len())
            } else {
                self.bound_tuple_elements(term).map(|fields| fields.len())
            };
            if let Some(candidate) = candidate {
                if width.is_some_and(|width| width != candidate) {
                    return Ok(None);
                }
                width = Some(candidate);
            }
            for (left, right) in &edges {
                nir::type_computations::charge_term(term, fuel).map_err(|_| exhausted())?;
                nir::type_computations::charge_term(left, fuel).map_err(|_| exhausted())?;
                nir::type_computations::charge_term(right, fuel).map_err(|_| exhausted())?;
                if term == *left {
                    pending.push(*right);
                }
                if term == *right {
                    pending.push(*left);
                }
            }
        }
        Ok(width)
    }

    fn preserved_input_width(
        &mut self,
        owner: OwnerId,
        types: &[nir::Parameter],
        result: &Term,
        arguments: &[Arg<'_>],
        fuel: &mut usize,
        depth: usize,
        span: Span,
    ) -> Result<Option<usize>, Error> {
        let Some(ItemKind::Fn(signature, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        if signature.params.len() != types.len() {
            return Ok(None);
        }
        *fuel = fuel
            .checked_sub(signature.params.len().saturating_add(arguments.len()))
            .ok_or_else(|| error(span, "spread layout inference budget exhausted"))?;
        let parameters = signature.params.to_vec();
        // Quote and adaptation protocols need their own layouts.
        if parameters.iter().any(|parameter| {
            parameter.ty.is_quote()
                || parameter.ty.is_assoc()
                || parameter.ty.is_lambda()
                || parameter.ty.is_catch()
                || !matches!(
                    parameter.ty.kind,
                    TyParamKind::Positional(_)
                        | TyParamKind::PositionalDependencyCatched(..)
                        | TyParamKind::Optional(..)
                        | TyParamKind::Varadic(..)
                )
        }) {
            return Ok(None);
        }
        let collectors: Vec<_> = parameters
            .iter()
            .enumerate()
            .filter(|(_, parameter)| matches!(parameter.ty.kind, TyParamKind::Varadic(..)))
            .collect();
        if !collectors.is_empty() {
            let [(index, collector)] = collectors.as_slice() else {
                return Ok(None);
            };
            if collector.ty.is_implicit() || types[*index].ty != *result {
                return Ok(None);
            };
            let anchors = parameters
                .iter()
                .filter(|parameter| {
                    !parameter.ty.is_implicit()
                        && matches!(
                            parameter.ty.kind,
                            TyParamKind::Positional(_)
                                | TyParamKind::PositionalDependencyCatched(..)
                        )
                })
                .count();
            let mut positions = 0usize;
            let mut complete = None;
            for argument in arguments {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "spread layout inference budget exhausted"))?;
                let amount = match argument {
                    Arg::Positional(_) => 1,
                    Arg::Named(name, value) if name.name == collector.name.name => {
                        if complete.replace(*value).is_some() {
                            return Ok(None);
                        };
                        0
                    }
                    Arg::Named(..) => 0,
                    Arg::Expand(input) if matches!(input.kind, H::Unit) => 0,
                    Arg::Expand(input) => {
                        let Some(width) = self.spread_width(input, fuel, depth)? else {
                            return Ok(None);
                        };
                        width
                    }
                    _ => return Ok(None),
                };
                positions = positions
                    .checked_add(amount)
                    .ok_or_else(|| error(span, "spread layout size exhausted"))?;
            }
            return if let Some(complete) = complete {
                if positions != anchors {
                    Ok(None)
                } else {
                    self.spread_width(complete, fuel, depth)
                }
            } else {
                Ok(positions.checked_sub(anchors))
            };
        }
        if arguments
            .iter()
            .any(|arg| !matches!(arg, Arg::Positional(_) | Arg::Named(..)))
        {
            return Ok(None);
        }
        let mut positional = arguments.iter().filter_map(|arg| match arg {
            Arg::Positional(value) => Some(*value),
            _ => None,
        });
        for (parameter, ty) in parameters.iter().zip(types) {
            let input = if parameter.ty.is_implicit()
                || matches!(parameter.ty.kind, TyParamKind::Optional(..))
            {
                *fuel = fuel
                    .checked_sub(arguments.len())
                    .ok_or_else(|| error(span, "spread layout inference budget exhausted"))?;
                let mut named = arguments.iter().filter_map(|arg| match arg {
                    Arg::Named(name, value) if name.name == parameter.name.name => Some(*value),
                    _ => None,
                });
                let input = named.next();
                if named.next().is_some() {
                    return Ok(None);
                };
                input
            } else {
                positional.next()
            };
            if ty.ty == *result {
                if let Some(input) = input {
                    if let Some(width) = self.spread_width(input, fuel, depth)? {
                        return Ok(Some(width));
                    }
                }
            }
        }
        Ok(None)
    }

    fn spread_branch_width<'h>(
        &mut self,
        branches: impl Iterator<Item = &'h Expr<'h>>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<usize>, Error> {
        let mut width = None;
        for branch in branches {
            let Some(next) = self.spread_width(branch, fuel, depth)? else {
                return Ok(None);
            };
            if width.is_some_and(|previous| previous != next) {
                return Ok(None);
            }
            width = Some(next);
        }
        Ok(width)
    }

    pub(super) fn tuple_width(&self, ty: &Term, span: Span) -> Result<Option<usize>, Error> {
        Ok(match self.structural_type(ty, span)? {
            Term::Tuple(fields) => Some(fields.len()),
            _ => None,
        })
    }
}

// This is a width hint after the caller establishes the list domain, not value
// admission. Never execute element producers, follow calls or fold concatenation
// here: argument order and mutable generator state belong to ordinary checking.
fn completed_list_constructor_width(
    value: &n::Expr,
    element: &Term,
    fuel: &mut usize,
) -> Result<Option<usize>, &'static str> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("spread list width budget exhausted")?;
    match &value.kind {
        K::TypeList(fields) => {
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or("spread list width budget exhausted")?;
            Ok(fields
                .iter()
                .all(|field| field.ty == *element)
                .then_some(fields.len()))
        }
        K::TypeListValue(fields) if *element == p(P::Type) => {
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or("spread list width budget exhausted")?;
            Ok(fields
                .iter()
                .all(|field| field.sort() == Sort::Type)
                .then_some(fields.len()))
        }
        K::ComptimeValue(Term::NominalValue {
            ty,
            variant: None,
            fields,
        }) if ty.as_ref() == &value.ty => {
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or("spread list width budget exhausted")?;
            // The list identity is exact; full payload admission still occurs
            // at the ordinary value boundary, not in this layout hint.
            Ok(Some(fields.len()))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod list_width_tests {
    use super::*;

    #[test]
    fn immutable_list_width_reads_representation_without_running_producers() {
        let domain = Term::NormalForm {
            head: DeclarationId {
                package: 53,
                local: 1,
            },
            sort: Sort::Type,
            arguments: vec![p(P::Type)],
        };
        let expr = |ty, kind| n::Expr {
            source: n::SourceRange { start: 0, end: 1 },
            ty,
            kind,
        };
        let producer = expr(
            p(P::Type),
            K::Call {
                callee: Box::new(expr(
                    Term::Function {
                        parameters: vec![],
                        result: Box::new(p(P::Type)),
                    },
                    K::Function(DeclarationId {
                        package: 53,
                        local: 2,
                    }),
                )),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        );
        let ordinary = expr(domain.clone(), K::TypeList(vec![producer.clone()]));
        let frozen = expr(
            domain.clone(),
            K::ComptimeValue(Term::NominalValue {
                ty: Box::new(domain.clone()),
                variant: None,
                fields: vec![p(P::Bool)],
            }),
        );
        let compact = expr(domain.clone(), K::TypeListValue(vec![p(P::Bool)]));
        for value in [&ordinary, &frozen, &compact] {
            assert_eq!(
                completed_list_constructor_width(value, &p(P::Type), &mut 100),
                Ok(Some(1))
            );
            assert!(completed_list_constructor_width(value, &p(P::Type), &mut 1).is_err());
            assert_eq!(
                completed_list_constructor_width(value, &p(P::Type), &mut 100),
                Ok(Some(1))
            );
        }
        // Calls producing a list are unknown. Calls inside a known constructor
        // are not evaluated merely to obtain its syntactic arity.
        let call = expr(domain.clone(), producer.kind);
        assert_eq!(
            completed_list_constructor_width(&call, &p(P::Type), &mut 100),
            Ok(None)
        );
        let wrong = expr(
            domain.clone(),
            K::TypeList(vec![expr(p(P::Bool), K::Literal(n::Literal::Bool(true)))]),
        );
        assert_eq!(
            completed_list_constructor_width(&wrong, &p(P::Type), &mut 100),
            Ok(None)
        );
        let foreign = expr(
            domain.clone(),
            K::ComptimeValue(Term::NominalValue {
                ty: Box::new(Term::NormalForm {
                    head: DeclarationId {
                        package: 53,
                        local: 9,
                    },
                    sort: Sort::Type,
                    arguments: vec![p(P::Type)],
                }),
                variant: None,
                fields: vec![],
            }),
        );
        assert_eq!(
            completed_list_constructor_width(&foreign, &p(P::Type), &mut 100),
            Ok(None)
        );
        let empty = expr(domain, K::TypeList(vec![]));
        assert_eq!(
            completed_list_constructor_width(&empty, &p(P::Type), &mut 100),
            Ok(Some(0))
        );
    }
}
