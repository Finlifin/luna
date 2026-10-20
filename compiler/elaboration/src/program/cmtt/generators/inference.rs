//! Bounded type evidence for staged calls. This pass reads existing types and
//! static syntax; it never prepares argument values or invokes producers.
//! Actual calls still validate constraints, phases, contracts and evaluation.
use super::*;

impl Checker<'_, '_> {
    // Keep incomplete aggregate evidence private to the call candidate. A
    // numeric literal with no completed target contributes no default type.
    pub(super) fn generator_partial_input_hint(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
        parameters: &[DeclarationId],
        packets: &spreads::SpreadPlan<'_>,
        depth: usize,
    ) -> Result<crate::calls::InputContext, Error> {
        use crate::calls::InputContext as C;
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(value.span, "generator aggregate evidence budget exhausted"))?;
        if depth >= 64 {
            return Ok(C::Unknown);
        }
        if let H::ResolvedLocal(_, id) = value.kind {
            if let Some(ty) = packets.field_type(local(id)) {
                return self.copy_alias_type(ty, value.span).map(C::Known);
            }
        }
        if let Some(ty) = self.generator_expression_type_hint(value, depth)? {
            return Ok(C::Known(ty));
        }
        let expected = expected
            .map(|ty| self.structural_type(ty, value.span))
            .transpose()?;
        match &value.kind {
            H::Tuple(fields) => {
                let targets = match &expected {Some(Term::Tuple(fields))=>Some(fields), _=>None};
                let mut contexts=Vec::with_capacity(fields.len());
                for (index,field) in fields.iter().enumerate() {
                    contexts.push(self.generator_partial_input_hint(field,targets.and_then(|fields|fields.get(index)),parameters,packets,depth+1)?);
                }
                Ok(match self.protocols.tuple {Some(head)=>C::NormalTuple{head,fields:contexts},None=>C::Tuple(contexts)})
            }
            H::CollectedProperties(fields) => {
                let targets = match &expected {Some(Term::Record(fields))=>Some(fields), _=>None};
                let mut contexts=Vec::with_capacity(fields.len());
                for field in *fields {
                    let target=targets.and_then(|fields|fields.iter().find(|(name,_)|name==field.ident.name.as_str()).map(|(_,ty)|ty));
                    contexts.push((field.ident.name.to_string(),self.generator_partial_input_hint(field.expr,target,parameters,packets,depth+1)?));
                }
                Ok(C::Record(contexts))
            }
            H::Lit(_) if expected.as_ref().is_none_or(|ty| matches!(ty,Term::Rigid{declaration,sort:Sort::Type} if parameters.contains(declaration))) => {
                Ok(self.generator_intrinsic_input_hint(value,depth)?.map_or(C::Unknown,C::Known))
            }
            _=>Ok(C::Unknown),
        }
    }

    // Use only established caller-local layouts or closed declaration results.
    // A packet is still evaluated and checked at its source position later.
    pub(in crate::program::cmtt) fn generator_expression_type_hint(
        &mut self,
        value: &Expr<'_>,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
            error(
                value.span,
                "generator packet type evidence budget exhausted",
            )
        })?;
        if depth >= 64 {
            return Ok(None);
        }
        let ty = match &value.kind {
            H::Block(block) => {
                return self.generator_block_type_hint(block, depth + 1);
            }
            H::InlineControl(inner) => {
                return self.generator_expression_type_hint(inner, depth + 1);
            }
            H::If(_, yes, Some(no)) => {
                let Some(yes) = yes.expr else { return Ok(None) };
                return self.generator_branch_type_hint([yes, *no], depth + 1);
            }
            H::Match(subject, arms) => {
                return self.generator_match_type_hint(subject, arms, depth + 1);
            }
            H::When(arms) => {
                if !arms.iter().any(|arm| arm.cond.is_none()) {
                    return Ok(None);
                }
                return self.generator_branch_type_hint(arms.iter().map(|arm| arm.body), depth + 1);
            }
            H::Ref(inner) => {
                return Ok(self
                    .generator_intrinsic_input_hint(inner, depth + 1)?
                    .map(|ty| Term::Pointer(Box::new(ty))));
            }
            H::Deref(inner) => {
                return Ok(
                    match self.generator_intrinsic_input_hint(inner, depth + 1)? {
                        Some(Term::Pointer(pointee)) => Some(*pointee),
                        _ => None,
                    },
                );
            }
            H::Projection(base, field) => {
                return self.generator_field_type_hint(
                    base,
                    field.name.as_str(),
                    value.span,
                    depth + 1,
                );
            }
            H::ResolvedDef { .. } => {
                return self.generator_explicit_signature_hint(value, &[], depth + 1, false);
            }
            H::NFApplication(head, arguments) => {
                return self.generator_explicit_signature_hint(head, arguments, depth + 1, false);
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => self.locals.get(&local(*id)),
            H::SelfValue => {
                let Some(item) = self.package.item(value.hir_id.owner) else {
                    return Ok(None);
                };
                let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) = &item.kind
                else {
                    return Ok(None);
                };
                let Some(parameter) = signature
                    .params
                    .first()
                    .filter(|parameter| matches!(parameter.ty.kind, TyParamKind::Itself { .. }))
                else {
                    return Ok(None);
                };
                self.locals.get(&local(parameter.hir_id))
            }
            H::ExtendedApplication(callee, arguments) => {
                return self.generator_property_result_hint(value, callee, arguments, depth + 1);
            }
            H::Application(callee, arguments) => {
                if let H::NFApplication(head, arguments) = callee.kind {
                    return self.generator_explicit_signature_hint(
                        head,
                        arguments,
                        depth + 1,
                        true,
                    );
                }
                let (signature, generic, definition) = match callee.kind {
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
                        self.ensure_signature(owner, value.span)?;
                        let Some(signature) = self.signatures.get(&owner) else {
                            return Ok(None);
                        };
                        (signature, self.type_parameters.get(&owner), Some(owner))
                    }
                    H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                        let Some(signature) = self.locals.get(&local(id)) else {
                            return Ok(None);
                        };
                        (signature, None, None)
                    }
                    _ => {
                        let Some(signature) =
                            self.generator_expression_type_hint(callee, depth + 1)?
                        else {
                            return Ok(None);
                        };
                        let Term::Function { result, .. } = signature.callable_signature() else {
                            return Ok(None);
                        };
                        // The result is evidence only when it does not depend
                        // on this invocation's ordinary argument values.
                        if result.check_scope(&[]).is_err() {
                            return Ok(None);
                        }
                        return self.copy_alias_type(result, value.span).map(Some);
                    }
                };
                let signature = match signature {
                    Term::Closure { signature, .. } => signature.as_ref(),
                    _ => signature,
                };
                let Term::Function { result, .. } = signature.callable_signature() else {
                    return Ok(None);
                };
                // Callee-bound dependent inputs and uninstantiated generic
                // parameters are not caller type evidence. A generic producer
                // may still have a fixed result independent of those inputs.
                if result.check_scope(&[]).is_err() {
                    return Ok(None);
                }
                if generic.is_some_and(|ids| ids.iter().any(|id| result.mentions_rigid(*id))) {
                    return self.generator_inferred_result_hint(
                        definition.unwrap(),
                        arguments,
                        value,
                        depth + 1,
                    );
                }
                Some(result.as_ref())
            }
            _ => None,
        };
        let Some(ty) = ty else { return Ok(None) };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let bounded = nir::type_computations::charge_term(ty, &mut fuel);
        self.required_steps = fuel as u64;
        bounded.map_err(|e| error(value.span, e.to_string()))?;
        Ok(Some(ty.clone()))
    }

    fn generator_match_type_hint(
        &mut self,
        subject: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        if depth >= 64 {
            return Ok(None);
        }
        let Some(subject_type) = self.generator_intrinsic_input_hint(subject, depth + 1)? else {
            return self.generator_branch_type_hint(arms.iter().map(|arm| arm.body), depth + 1);
        };
        let mut common = None;
        for arm in arms {
            let mut saved = Vec::new();
            let result = (|| {
                self.generator_pattern_type_hints(&arm.pat, &subject_type, depth + 1, &mut saved)?;
                self.generator_expression_type_hint(arm.body, depth + 1)
            })();
            for (binding, previous) in saved.into_iter().rev() {
                match previous {
                    Some(ty) => {
                        self.locals.insert(binding, ty);
                    }
                    None => {
                        self.locals.remove(&binding);
                    }
                }
            }
            let Some(ty) = result? else {
                return Ok(None);
            };
            if common.as_ref().is_some_and(|previous| previous != &ty) {
                return Ok(None);
            }
            common = Some(ty);
        }
        Ok(common)
    }

    fn generator_block_type_hint(
        &mut self,
        block: &hir::expr::Block<'_>,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        if depth >= 64 {
            return Ok(None);
        }
        let mut saved = Vec::new();
        let result = (|| {
            for statement in block.stmts {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(statement.span, "generator block evidence budget exhausted")
                })?;
                let H::Let(declaration) = &statement.kind else {
                    continue;
                };
                let ty = if let Some(annotation) = declaration.ty {
                    self.generator_static_type_hint(annotation, depth + 1, None)?
                } else if let Some(value) = declaration.init {
                    self.generator_intrinsic_input_hint(value, depth + 1)?
                } else {
                    None
                };
                if let Some(ty) = ty {
                    self.generator_pattern_type_hints(
                        &declaration.pattern,
                        &ty,
                        depth + 1,
                        &mut saved,
                    )?;
                }
            }
            match block.expr {
                Some(tail) => self.generator_expression_type_hint(tail, depth + 1),
                None => Ok(None),
            }
        })();
        for (binding, previous) in saved.into_iter().rev() {
            match previous {
                Some(ty) => {
                    self.locals.insert(binding, ty);
                }
                None => {
                    self.locals.remove(&binding);
                }
            }
        }
        result
    }

    fn generator_pattern_type_hints(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        ty: &Term,
        depth: usize,
        saved: &mut Vec<(n::LocalId, Option<Term>)>,
    ) -> Result<(), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(pattern.span, "generator binding evidence budget exhausted"))?;
        if depth >= 64 {
            return Ok(());
        }
        match &pattern.kind {
            hir::pattern::PatternKind::Binding(hir::common::BindingMode::ByValue, _, inner) => {
                if let Some(inner) = inner {
                    self.generator_pattern_type_hints(inner, ty, depth + 1, saved)?;
                }
                let ty = self.copy_alias_type(ty, pattern.span)?;
                let binding = local(pattern.hir_id);
                saved.push((binding, self.locals.insert(binding, ty)));
            }
            hir::pattern::PatternKind::Guard(inner, _) => {
                self.generator_pattern_type_hints(inner, ty, depth + 1, saved)?;
            }
            hir::pattern::PatternKind::Or(alternatives) => {
                // Only export evidence agreed by every alternative. Each probe
                // has its own scope; the first alternative owns canonical IDs,
                // as in normal pattern elaboration.
                let mut common: Option<BTreeMap<String, (n::LocalId, Term)>> = None;
                for alternative in *alternatives {
                    let mut temporary = Vec::new();
                    let result = (|| {
                        self.generator_pattern_type_hints(
                            alternative,
                            ty,
                            depth + 1,
                            &mut temporary,
                        )?;
                        let mut evidence = BTreeMap::new();
                        for (name, binding) in pattern_bindings(alternative)? {
                            if let Some(ty) = self.locals.remove(&binding) {
                                let copied = self.copy_alias_type(&ty, alternative.span);
                                self.locals.insert(binding, ty);
                                evidence.insert(name, (binding, copied?));
                            }
                        }
                        Ok::<_, Error>(evidence)
                    })();
                    for (binding, previous) in temporary.into_iter().rev() {
                        match previous {
                            Some(ty) => {
                                self.locals.insert(binding, ty);
                            }
                            None => {
                                self.locals.remove(&binding);
                            }
                        }
                    }
                    let evidence = result?;
                    if let Some(common) = &mut common {
                        common.retain(|name, (_, ty)| {
                            evidence.get(name).is_some_and(|(_, other)| other == ty)
                        });
                    } else {
                        common = Some(evidence);
                    }
                }
                for (_, (binding, ty)) in common.into_iter().flatten() {
                    saved.push((binding, self.locals.insert(binding, ty)));
                }
            }
            hir::pattern::PatternKind::Tuple(patterns) => {
                if let Term::Tuple(fields) = self.structural_type(ty, pattern.span)? {
                    if patterns.len() == fields.len() {
                        for (pattern, ty) in patterns.iter().zip(&fields) {
                            self.generator_pattern_type_hints(pattern, ty, depth + 1, saved)?;
                        }
                    }
                }
            }
            hir::pattern::PatternKind::Struct(head, fields, false)
                if matches!(head.kind, hir::pattern::PatternKind::Wild) =>
            {
                for field in *fields {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(
                            field.span,
                            "generator field binding evidence budget exhausted",
                        )
                    })?;

                    if let Some(ty) = self.generator_layout_field_type_hint(
                        ty,
                        field.ident.name.as_str(),
                        field.span,
                    )? {
                        self.generator_pattern_type_hints(&field.pat, &ty, depth + 1, saved)?;
                    }
                }
            }
            hir::pattern::PatternKind::AppTuple(head, patterns) => {
                if let Some((n::VariantShape::Tuple, fields)) =
                    self.generator_variant_field_hints(head, ty, pattern.span)?
                {
                    if patterns.len() == fields.len() {
                        for (pattern, ty) in patterns.iter().zip(&fields) {
                            self.generator_pattern_type_hints(pattern, ty, depth + 1, saved)?;
                        }
                    }
                }
            }
            hir::pattern::PatternKind::Struct(head, patterns, false) => {
                if let Some((n::VariantShape::Named(names), fields)) =
                    self.generator_variant_field_hints(head, ty, pattern.span)?
                {
                    for field in *patterns {
                        self.required_steps =
                            self.required_steps.checked_sub(1).ok_or_else(|| {
                                error(field.span, "generator variant evidence budget exhausted")
                            })?;
                        if let Some(index) = names
                            .iter()
                            .position(|name| name == field.ident.name.as_str())
                        {
                            self.generator_pattern_type_hints(
                                &field.pat,
                                &fields[index],
                                depth + 1,
                                saved,
                            )?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn generator_variant_field_hints(
        &mut self,
        head: &hir::pattern::Pattern<'_>,
        ty: &Term,
        span: Span,
    ) -> Result<Option<(n::VariantShape, Vec<Term>)>, Error> {
        let hir::pattern::PatternKind::Const(expression) = &head.kind else {
            return Ok(None);
        };
        let Some(name) = enum_groups::symbol_path(expression) else {
            return Ok(None);
        };
        let Term::Nominal {
            declaration,
            arguments,
        } = self.structural_type(ty, span)?
        else {
            return Ok(None);
        };
        let Some(definition) = self.enums.get(&declaration) else {
            return Ok(None);
        };
        let Some(variant) = definition
            .variants
            .iter()
            .find(|variant| variant.name == name)
        else {
            return Ok(None);
        };
        let shape_work = match &variant.shape {
            n::VariantShape::Named(names) => names
                .iter()
                .try_fold(0u64, |work, name| work.checked_add(name.len() as u64 + 1)),
            _ => Some(1),
        };
        self.required_steps = shape_work
            .and_then(|work| self.required_steps.checked_sub(work))
            .ok_or_else(|| error(span, "generator variant evidence budget exhausted"))?;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let fields = nir::normal_forms::instantiate_value_fields(
            &definition.type_parameters,
            &definition.parameter_types,
            &arguments,
            variant.fields.iter(),
            &mut fuel,
        );
        self.required_steps -= (before - fuel) as u64;
        Ok(fields.map(|fields| (variant.shape.clone(), fields)))
    }

    // Built-in owned fields have a layout-derived type. Reading that layout
    // does not read the object, invoke its producer, or select a method/getter.
    fn generator_field_type_hint(
        &mut self,
        base: &Expr<'_>,
        name: &str,
        span: Span,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        let Some(ty) = self.generator_expression_type_hint(base, depth)? else {
            return Ok(None);
        };
        self.generator_layout_field_type_hint(&ty, name, span)
    }

    fn generator_layout_field_type_hint(
        &mut self,
        ty: &Term,
        name: &str,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let shape = self.structural_type(ty, span)?;
        match shape {
            Term::Record(fields) => Ok(fields
                .into_iter()
                .find(|(field, _)| field == name)
                .map(|(_, ty)| ty)),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let Some(definition) = self.structs.get(&declaration) else {
                    return Ok(None);
                };
                let Some((_, field)) = definition.fields.iter().find(|(field, _)| field == name)
                else {
                    return Ok(None);
                };
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let fields = nir::normal_forms::instantiate_value_fields(
                    &definition.type_parameters,
                    &definition.parameter_types,
                    &arguments,
                    std::iter::once(field),
                    &mut fuel,
                );
                self.required_steps -= (before - fuel) as u64;
                Ok(fields.and_then(|mut fields| fields.pop()))
            }
            _ => Ok(None),
        }
    }

    fn generator_property_result_hint(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        if depth >= 64 {
            return Ok(None);
        }
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        if self
            .package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Struct(_) | ItemKind::Enum(_)))
        {
            return self.generator_static_type_hint(callee, depth + 1, None);
        }
        let Some(ItemKind::Fn(signature, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        if signature.modifiers.is_normal_form_head {
            return Ok(None);
        }
        if !arguments
            .iter()
            .any(|argument| matches!(argument, Arg::Named(..)))
        {
            let call = Expr {
                kind: H::Application(callee, arguments),
                ..site.clone()
            };
            return self.generator_expression_type_hint(&call, depth + 1);
        }
        self.required_steps = self
            .required_steps
            .checked_sub(arguments.len() as u64 + signature.params.len() as u64)
            .ok_or_else(|| error(site.span, "generator property evidence budget exhausted"))?;
        let collectors: Vec<_> = signature
            .params
            .iter()
            .filter(|param| matches!(param.ty.kind, TyParamKind::Varadic(..)))
            .map(|param| (param.name, param.index as usize))
            .collect();
        let Ok((collector, _)) =
            self.property_collector(owner, &collectors, None, None, None, site.span)
        else {
            return Ok(None);
        };
        let fields: Vec<_> = arguments
            .iter()
            .filter_map(|argument| match argument {
                Arg::Named(name, value) => Some(hir::expr::FieldExpr {
                    ident: *name,
                    expr: value,
                    span: value.span,
                }),
                _ => None,
            })
            .collect();
        let packet = Expr {
            kind: H::CollectedProperties(&fields),
            ..site.clone()
        };
        let mut inserted = false;
        let inputs: Vec<_> = arguments
            .iter()
            .filter_map(|argument| match argument {
                Arg::Named(..) if !inserted => {
                    inserted = true;
                    Some(Arg::Named(collector, &packet))
                }
                Arg::Named(..) => None,
                other => Some(other.clone()),
            })
            .collect();
        let call = Expr {
            kind: H::Application(callee, &inputs),
            ..site.clone()
        };
        self.generator_expression_type_hint(&call, depth + 1)
    }

    fn generator_inferred_result_hint(
        &mut self,
        owner: OwnerId,
        arguments: &[Arg<'_>],
        site: &Expr<'_>,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        let Some(ItemKind::Fn(declaration, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        self.required_steps = self
            .required_steps
            .checked_sub(arguments.len() as u64)
            .ok_or_else(|| error(site.span, "generator input evidence budget exhausted"))?;
        let mut flat = Vec::new();
        let mut packet_types = BTreeMap::new();
        for argument in arguments {
            match argument {
                Arg::Expand(packet) => {
                    if let H::Tuple(fields) = packet.kind {
                        self.required_steps = self
                            .required_steps
                            .checked_sub(fields.len() as u64)
                            .ok_or_else(|| {
                                error(packet.span, "generator input evidence budget exhausted")
                            })?;
                        flat.extend(fields.iter().map(|field| (None, field.clone(), false)));
                    } else if !matches!(packet.kind, H::Unit) {
                        let Some(fields) = self.generator_packet_type_hint(packet, depth + 1)?
                        else {
                            return Ok(None);
                        };
                        for ty in fields {
                            let binding = self.spread_binding(packet.span)?;
                            let id = hir::HirId::new(
                                hir::OwnerId::INVALID,
                                hir::ItemLocalId::new(binding.index),
                            );
                            packet_types.insert(local(id), ty);
                            flat.push((
                                None,
                                Expr {
                                    hir_id: id,
                                    span: packet.span,
                                    kind: H::ResolvedLocal(
                                        hir::Symbol::intern("$generator-type-evidence"),
                                        id,
                                    ),
                                },
                                false,
                            ));
                        }
                    }
                }
                Arg::Positional(value) => flat.push((None, (**value).clone(), false)),
                Arg::PostLambda(value) => flat.push((None, (**value).clone(), true)),
                Arg::Named(name, value) => flat.push((Some(*name), (**value).clone(), false)),
                _ => return Ok(None),
            }
        }
        let arguments: Vec<_> = flat
            .iter()
            .map(|(name, value, post)| {
                if *post {
                    Arg::PostLambda(value)
                } else if let Some(name) = name {
                    Arg::Named(*name, value)
                } else {
                    Arg::Positional(value)
                }
            })
            .collect();
        let signature = &self.signatures[&owner];
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let bounded = nir::type_computations::charge_term(signature, &mut fuel);
        self.required_steps = fuel as u64;
        bounded.map_err(|e| error(site.span, e.to_string()))?;
        let signature = signature.callable_signature().clone();
        let all_parameters = self.type_parameters[&owner].clone();
        if declaration.clause_params.len() != all_parameters.len() {
            return Ok(None);
        }
        let binders = self.input_binders(&all_parameters);
        let mut parameters = Vec::new();
        let mut assigned = Vec::new();
        for (index, clause) in declaration.clause_params.iter().enumerate() {
            let ty = match clause.kind {
                hir::clause::ClauseParamKind::Optional(_, _, default) => {
                    let bindings = Some((all_parameters.as_slice(), assigned.as_slice()));
                    let hint = if binders[index].sort == Sort::Type {
                        self.generator_static_type_hint(default, depth + 1, bindings)?
                    } else {
                        let Some(domain) = self.trait_input_domain(all_parameters[index]) else {
                            return Ok(None);
                        };
                        let domain = domain
                            .instantiate_parameters(&binders[..index], &assigned)
                            .map_err(|e| error(default.span, e.to_string()))?;
                        self.generator_static_value_hint(default, &domain, bindings)?
                    };
                    let Some(ty) = hint else { return Ok(None) };
                    ty
                }
                _ => {
                    parameters.push(all_parameters[index]);
                    Term::Rigid {
                        declaration: all_parameters[index],
                        sort: binders[index].sort,
                    }
                }
            };
            assigned.push(ty);
        }
        // Fix static defaults before inference. A dependent default can retain
        // an earlier mandatory parameter, but never becomes a fresh variable.
        let signature = signature
            .instantiate_parameters(&binders, &assigned)
            .map_err(|e| error(site.span, e.to_string()))?;
        let Term::Function {
            parameters: inputs, ..
        } = &signature
        else {
            return Ok(None);
        };
        let (routes, _) = route_arguments(
            &arguments,
            Some(declaration.params),
            inputs.len(),
            site.span,
        )?;
        let mut evidence = vec![None; inputs.len()];
        for route in routes {
            let expression = route.expression(site);
            let unconstrained = matches!(&inputs[route.parameter].ty,
                Term::Rigid { declaration, sort: Sort::Type } if parameters.contains(declaration));
            let input_hint = |this: &mut Self, value: &Expr<'_>| {
                if let H::ResolvedLocal(_, id) = value.kind
                    && let Some(ty) = packet_types.get(&local(id))
                {
                    return this.copy_alias_type(ty, value.span).map(Some);
                }
                if unconstrained {
                    this.generator_intrinsic_input_hint(value, depth + 1)
                } else {
                    this.generator_expression_type_hint(value, depth + 1)
                }
            };
            evidence[route.parameter] = if route.is_collected() {
                let mut fields = Vec::new();
                let mut complete = true;
                for (_, input) in route.source_inputs() {
                    let Some(ty) = input_hint(self, input)? else {
                        complete = false;
                        break;
                    };
                    fields.push(ty);
                }
                if complete {
                    Some(self.tuple_value_type(fields, site.span)?)
                } else {
                    None
                }
            } else {
                input_hint(self, &expression)?
            };
        }
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&parameters)
                .iter()
                .map(|p| p.sort)
                .collect(),
            type_parameters: parameters.len(),
            signature: signature
                .abstract_parameters(&self.input_binders(&parameters))
                .map_err(|e| error(site.span, e.to_string()))?,
        };
        // Incomplete or conflicting provisional evidence must not replace the
        // actual call's contextual checking and diagnostics.
        let Ok(candidate) = scheme.infer_partial_types_in_context(
            &evidence,
            None,
            &self.context,
            crate::ptc::Budget::default(),
        ) else {
            return Ok(None);
        };
        Ok(candidate.result_type().ok().flatten())
    }

    // Only use intrinsic literal types when the producer's parameter itself
    // is unconstrained. Concrete destination types must keep contextual literal
    // checking, and numeric defaults are never chosen by evidence discovery.
    pub(super) fn generator_intrinsic_input_hint(
        &mut self,
        value: &Expr<'_>,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        if let Some(ty) = self.generator_expression_type_hint(value, depth)? {
            return Ok(Some(ty));
        }
        if depth >= 64 {
            return Ok(None);
        }
        match &value.kind {
            H::Lit(_) => Ok(self
                .checking_attempt(false, false, |this| this.expr(value, None))?
                .map(|checked| checked.ty)),
            H::Tuple(fields) => {
                let mut types = Vec::with_capacity(fields.len());
                for field in *fields {
                    let Some(ty) = self.generator_intrinsic_input_hint(field, depth + 1)? else {
                        return Ok(None);
                    };
                    types.push(ty);
                }
                self.tuple_value_type(types, value.span).map(Some)
            }
            H::CollectedProperties(fields) => {
                let mut names = BTreeSet::new();
                let mut types = Vec::with_capacity(fields.len());
                for field in *fields {
                    if !names.insert(field.ident.name.to_string()) {
                        return Ok(None);
                    }
                    let Some(ty) = self.generator_intrinsic_input_hint(field.expr, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    types.push((field.ident.name.to_string(), ty));
                }
                Ok(Some(Term::Record(types)))
            }
            _ => Ok(None),
        }
    }

    fn generator_explicit_signature_hint(
        &mut self,
        head: &Expr<'_>,
        arguments: &[Arg<'_>],
        depth: usize,
        result_only: bool,
    ) -> Result<Option<Term>, Error> {
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        let Some(ItemKind::Fn(signature, _)) = self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        if !result_only && signature.modifiers.is_normal_form_head {
            return Ok(None);
        }
        let comptime = signature.modifiers.is_comptime;
        let clauses = signature.clause_params;
        self.ensure_signature(owner, head.span)?;
        let Some(signature) = self.signatures.get(&owner) else {
            return Ok(None);
        };
        let observed = if result_only {
            let Term::Function { result, .. } = signature.callable_signature() else {
                return Ok(None);
            };
            result.as_ref()
        } else {
            signature
        };
        if observed.check_scope(&[]).is_err() {
            return Ok(None);
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let bounded = nir::type_computations::charge_term(observed, &mut fuel);
        self.required_steps = fuel as u64;
        bounded.map_err(|e| error(head.span, e.to_string()))?;
        let result = if !result_only && comptime {
            Term::ComptimeFunction(Box::new(observed.clone()))
        } else {
            observed.clone()
        };
        let parameters = self
            .type_parameters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        if clauses.len() != parameters.len() {
            return Ok(None);
        }
        let mut supplied = vec![None; parameters.len()];
        let mut positional = 0;
        for argument in arguments {
            let (name, value) = match argument {
                Arg::Positional(value) => (None, *value),
                Arg::Named(name, value) => (Some(*name), *value),
                _ => return Ok(None),
            };
            let slot = Self::type_input_slot(clauses, name, &mut positional, head.span)?;
            let Some(cell) = supplied.get_mut(slot) else {
                return Ok(None);
            };
            if cell.is_some() {
                return Ok(None);
            }
            *cell = Some(value);
        }
        // Route first, then complete the declaration telescope. Later value
        // domains and defaults can depend on earlier Type or value clauses.
        let binders = self.input_binders(&parameters);
        let mut completed = Vec::with_capacity(supplied.len());
        for (index, supplied) in supplied.into_iter().enumerate() {
            let (value, bindings) = match supplied {
                Some(value) => (value, None),
                None => {
                    let hir::clause::ClauseParamKind::Optional(_, _, default) = clauses[index].kind
                    else {
                        return Ok(None);
                    };
                    (default, Some((parameters.as_slice(), completed.as_slice())))
                }
            };
            let hint = if binders[index].sort == Sort::Type {
                self.generator_static_type_hint(value, depth + 1, bindings)?
            } else {
                let Some(domain) = self.trait_input_domain(parameters[index]) else {
                    return Ok(None);
                };
                let domain = domain
                    .instantiate_parameters(&binders[..index], &completed)
                    .map_err(|e| error(value.span, e.to_string()))?;
                self.generator_static_value_hint(value, &domain, bindings)?
            };
            let Some(hint) = hint else { return Ok(None) };
            completed.push(hint);
        }
        result
            .instantiate_parameters(&self.input_binders(&parameters), &completed)
            .map(Some)
            .map_err(|e| error(head.span, e.to_string()))
    }

    /// Read stable value evidence without invoking a producer or guessing an
    /// unresolved clause. Call checking still validates the full input domain.
    fn generator_static_value_hint(
        &mut self,
        argument: &Expr<'_>,
        domain: &Term,
        bindings: Option<(&[DeclarationId], &[Term])>,
    ) -> Result<Option<Term>, Error> {
        if let H::ResolvedClauseParam(_, id) = argument.kind {
            let id = DeclarationId {
                package: semantic_package(&self.package_identities, id.pkg),
                local: id.index,
            };
            let supplied = bindings.and_then(|(parameters, values)| {
                parameters
                    .iter()
                    .position(|parameter| *parameter == id)
                    .and_then(|slot| values.get(slot))
            });
            let value = if let Some(value) = supplied {
                self.copy_alias_type(value, argument.span)?
            } else if bindings.is_some_and(|(parameters, _)| parameters.contains(&id)) {
                // A later default cannot borrow evidence from an unrelated
                // surrounding instantiation of the same declaration.
                return Ok(None);
            } else if let Some(value) = self.alias_substitution.get(&id).cloned() {
                self.copy_alias_type(&value, argument.span)?
            } else {
                if self.input_binders(&[id])[0].sort != Sort::ComptimeValue {
                    return Ok(None);
                }
                self.copy_alias_type(
                    &Term::Rigid {
                        declaration: id,
                        sort: Sort::ComptimeValue,
                    },
                    argument.span,
                )?
            };
            return Ok((value.sort() == Sort::ComptimeValue).then_some(value));
        }
        if !matches!(argument.kind, H::Lit(_)) {
            return Ok(None);
        }
        let checked =
            self.checking_attempt(false, false, |this| this.expr(argument, Some(domain)))?;
        let mut budget = comptime_runtime::Budget {
            steps: self.required_steps as usize,
            max_limbs: 4096,
        };
        let value = checked
            .as_ref()
            .and_then(|checked| self.known_normal_input(checked, 0, &mut budget));
        self.required_steps = budget.steps as u64;
        Ok(value)
    }

    pub(super) fn generator_static_type_hint(
        &mut self,
        value: &Expr<'_>,
        depth: usize,
        bindings: Option<(&[DeclarationId], &[Term])>,
    ) -> Result<Option<Term>, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(value.span, "generator type evidence budget exhausted"))?;
        if depth >= 64 {
            return Ok(None);
        }
        match &value.kind {
            H::Builtin(_) | H::TyType | H::TyVoid | H::TyNoReturn => {
                self.type_expr(value).map(Some)
            }
            H::NFApplication(head, arguments) => {
                let H::ResolvedDef { def_id, .. } = head.kind else {
                    return Ok(None);
                };
                let Some(owner) = self.package.definition_owner(def_id) else {
                    return Ok(None);
                };
                if self.package.item(owner).is_some_and(|item| {
                    matches!(item.kind, ItemKind::Struct(_) | ItemKind::Enum(_))
                }) {
                    if self.type_parameters[&owner].len() != arguments.len() {
                        return Ok(None);
                    }
                    let binders = self.input_binders(&self.type_parameters[&owner]);
                    let mut types = Vec::new();
                    for (argument, binder) in arguments.iter().zip(&binders) {
                        let Arg::Positional(argument) = argument else {
                            return Ok(None);
                        };
                        let value = if binder.sort == Sort::Type {
                            self.generator_static_type_hint(argument, depth + 1, bindings)?
                        } else {
                            let Some(domain) = self.trait_input_domain(binder.declaration) else {
                                return Ok(None);
                            };
                            let domain = domain
                                .instantiate_parameters(&binders[..types.len()], &types)
                                .map_err(|e| error(argument.span, e.to_string()))?;
                            self.generator_static_value_hint(argument, &domain, bindings)?
                        };
                        let Some(value) = value else { return Ok(None) };
                        types.push(value);
                    }
                    return Ok(Some(Term::Nominal {
                        declaration: declaration(self.package, &self.package_identities, owner),
                        arguments: types,
                    }));
                }
                let Some(ItemKind::Fn(signature, _)) =
                    self.package.item(owner).map(|item| &item.kind)
                else {
                    return Ok(None);
                };
                if !signature.modifiers.is_normal_form_head {
                    return Ok(None);
                }
                self.ensure_signature(owner, value.span)?;
                let id = declaration(self.package, &self.package_identities, owner);
                let tuple = Some(id) == self.protocols.tuple;
                if !tuple {
                    return self.generator_static_normal_form_hint(
                        owner, arguments, depth, bindings, value.span,
                    );
                }
                let mut types = Vec::with_capacity(arguments.len());
                for argument in *arguments {
                    match argument {
                        Arg::Positional(input) => {
                            let Some(ty) =
                                self.generator_static_type_hint(input, depth + 1, bindings)?
                            else {
                                return Ok(None);
                            };
                            types.push(ty);
                        }
                        Arg::Expand(input) if tuple => {
                            let Some(items) =
                                self.generator_static_type_list_hint(input, depth + 1, bindings)?
                            else {
                                return Ok(None);
                            };
                            types.extend(items);
                        }
                        Arg::Named(name, input)
                            if tuple
                                && arguments.len() == 1
                                && matches!(signature.clause_params, [parameter] if parameter.name.name == name.name) =>
                        {
                            let Some(items) =
                                self.generator_static_type_list_hint(input, depth + 1, bindings)?
                            else {
                                return Ok(None);
                            };
                            types.extend(items);
                        }
                        _ => return Ok(None),
                    }
                }
                // Formation preserves the opaque head. In particular Tuple
                // has ONE TypeList input, never a structural tuple identity.
                let arguments = if tuple {
                    vec![Term::TypeListValue(types)]
                } else {
                    types
                };
                Ok(Some(Term::NormalForm {
                    head: id,
                    sort: Sort::Type,
                    arguments,
                }))
            }
            H::ResolvedDef { def_id, .. } => {
                let Some(owner) = self.package.definition_owner(*def_id) else {
                    return Ok(None);
                };
                if self.package.item(owner).is_some_and(|item| {
                    matches!(item.kind, ItemKind::Struct(_) | ItemKind::Enum(_))
                }) && self.type_parameters[&owner].is_empty()
                {
                    return Ok(Some(self.nominal_self_type(owner)));
                }
                let Some(ItemKind::TypeAlias(target, clauses, constraints)) =
                    self.package.item(owner).map(|item| &item.kind)
                else {
                    return Ok(None);
                };
                // Only transparent, non-generic syntax aliases participate.
                // Do not invoke an alias RHS or discharge alias contracts here.
                if !clauses.is_empty() || !constraints.is_empty() {
                    return Ok(None);
                }
                self.generator_static_type_hint(target, depth + 1, bindings)
            }
            H::ResolvedClauseParam(_, id) => {
                if let Some((parameters, assigned)) = bindings {
                    let declaration = DeclarationId {
                        package: semantic_package(&self.package_identities, id.pkg),
                        local: id.index,
                    };
                    let Some(index) = parameters
                        .iter()
                        .position(|parameter| *parameter == declaration)
                    else {
                        return Ok(None);
                    };
                    return assigned
                        .get(index)
                        .map(|ty| self.copy_alias_type(ty, value.span))
                        .transpose();
                }
                self.type_expr(value).map(Some)
            }
            H::TyPtr(inner) => Ok(self
                .generator_static_type_hint(inner, depth + 1, bindings)?
                .map(|ty| Term::Pointer(Box::new(ty)))),
            H::TyFn(inputs) | H::TyComptimeFn(inputs) => {
                let mut parameters = Vec::with_capacity(inputs.len());
                for input in *inputs {
                    if input.flags != 0 {
                        return Ok(None);
                    }
                    let TyParamKind::Positional(ty) = input.kind else {
                        return Ok(None);
                    };
                    let Some(ty) = self.generator_static_type_hint(ty, depth + 1, bindings)? else {
                        return Ok(None);
                    };
                    parameters.push(Parameter {
                        sort: Sort::Runtime,
                        ty: ty
                            .abstract_parameters(&[])
                            .map_err(|e| error(input.span, e.to_string()))?,
                    });
                }
                Ok(Some(
                    Term::Function {
                        parameters,
                        result: Box::new(p(P::Void)),
                    }
                    .with_comptime_phase(matches!(value.kind, H::TyComptimeFn(_))),
                ))
            }
            H::TyFnArrow(input, output) => {
                let Some(input) = self.generator_static_type_hint(input, depth + 1, bindings)?
                else {
                    return Ok(None);
                };
                if !matches!(input.callable_signature(), Term::Function { .. }) {
                    return Ok(None);
                }
                let Some(result) = self.generator_static_type_hint(output, depth + 1, bindings)?
                else {
                    return Ok(None);
                };
                let result = result
                    .abstract_parameters(&[])
                    .map_err(|e| error(output.span, e.to_string()))?;
                Ok(input.with_callable_result(result))
            }
            H::TyClosure(context, body) => {
                let H::TyClosureEnvironment(inputs) = context.kind else {
                    return Ok(None);
                };
                let mut names = std::collections::HashSet::new();
                let mut environment = Vec::with_capacity(inputs.len());
                for input in inputs {
                    if input.value.is_some() || !names.insert(input.name.name) {
                        return Ok(None);
                    }
                    let Some(ty) =
                        self.generator_static_type_hint(input.ty, depth + 1, bindings)?
                    else {
                        return Ok(None);
                    };
                    environment.push(ty);
                }
                let Some(signature) = self.generator_static_type_hint(body, depth + 1, bindings)?
                else {
                    return Ok(None);
                };
                if environment.is_empty() {
                    return Ok(Some(signature));
                }
                if !matches!(signature, Term::Function { .. } | Term::ComptimeFunction(_)) {
                    return Ok(None);
                }
                Ok(Some(Term::Closure {
                    environment,
                    signature: Box::new(signature),
                }))
            }
            H::Tuple(fields) => {
                let mut types = Vec::with_capacity(fields.len());
                for field in *fields {
                    let Some(ty) = self.generator_static_type_hint(field, depth + 1, bindings)?
                    else {
                        return Ok(None);
                    };
                    types.push(ty);
                }
                self.tuple_value_type(types, value.span).map(Some)
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id)
                if self.immutable.contains(&local(*id)) =>
            {
                let Some(Some(n::Expr {
                    kind: K::TypeValue(ty),
                    ..
                })) = self.generator_data.get(&local(*id))
                else {
                    return Ok(None);
                };
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let bounded = nir::type_computations::charge_term(ty, &mut fuel);
                self.required_steps = fuel as u64;
                bounded.map_err(|e| error(value.span, e.to_string()))?;
                Ok(Some(ty.clone()))
            }
            _ => Ok(None),
        }
    }

    fn generator_static_normal_form_hint(
        &mut self,
        owner: OwnerId,
        arguments: &[Arg<'_>],
        depth: usize,
        outer_bindings: Option<(&[DeclarationId], &[Term])>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(
                arguments
                    .len()
                    .saturating_add(self.normal_parameters[&owner].len()) as u64,
            )
            .ok_or_else(|| error(span, "generator normal-form evidence budget exhausted"))?;
        let clauses = self.type_clause_params(owner);
        let parameters: Vec<_> = self.normal_parameters[&owner]
            .iter()
            .map(|(id, _, _)| *id)
            .collect();
        if clauses.len() != parameters.len() {
            return Ok(None);
        }
        let mut supplied = vec![None; parameters.len()];
        let mut positional = 0;
        for argument in arguments {
            let (name, value) = match argument {
                Arg::Positional(value) => (None, *value),
                Arg::Named(name, value) => (Some(*name), *value),
                _ => return Ok(None),
            };
            let slot = Self::type_input_slot(&clauses, name, &mut positional, value.span)?;
            let Some(cell) = supplied.get_mut(slot) else {
                return Ok(None);
            };
            if cell.replace(value).is_some() {
                return Ok(None);
            }
        }
        let binders = self.input_binders(&parameters);
        let mut completed = Vec::with_capacity(parameters.len());
        for (index, supplied) in supplied.into_iter().enumerate() {
            let (value, bindings) = match supplied {
                Some(value) => (value, outer_bindings),
                None => {
                    let hir::clause::ClauseParamKind::Optional(_, _, default) = clauses[index].kind
                    else {
                        return Ok(None);
                    };
                    (default, Some((parameters.as_slice(), completed.as_slice())))
                }
            };
            let declared = &self.normal_parameters[&owner][index].2;
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let bounded = nir::type_computations::charge_term(declared, &mut fuel);
            self.required_steps = fuel as u64;
            bounded.map_err(|e| error(value.span, e.to_string()))?;
            let domain = declared
                .instantiate_parameters(&binders[..index], &completed)
                .map_err(|e| error(value.span, e.to_string()))?;
            let hint = if domain == p(P::Type) {
                self.generator_static_type_hint(value, depth + 1, bindings)?
            } else {
                self.generator_static_value_hint(value, &domain, bindings)?
            };
            let Some(hint) = hint else { return Ok(None) };
            completed.push(hint);
        }
        // Form identity only. Never evaluate the head to obtain its body type.
        Ok(Some(Term::NormalForm {
            head: declaration(self.package, &self.package_identities, owner),
            sort: Sort::Type,
            arguments: completed,
        }))
    }

    fn generator_static_type_list_hint(
        &mut self,
        value: &Expr<'_>,
        depth: usize,
        bindings: Option<(&[DeclarationId], &[Term])>,
    ) -> Result<Option<Vec<Term>>, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(value.span, "generator type list evidence budget exhausted"))?;
        if depth >= 64 {
            return Ok(None);
        }
        match &value.kind {
            H::List(items) => {
                let mut types = Vec::with_capacity(items.len());
                for item in *items {
                    let Some(ty) = self.generator_static_type_hint(item, depth + 1, bindings)?
                    else {
                        return Ok(None);
                    };
                    types.push(ty);
                }
                Ok(Some(types))
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id)
                if self.immutable.contains(&local(*id)) =>
            {
                let Some(Some(n::Expr {
                    kind: K::TypeListValue(types),
                    ..
                })) = self.generator_data.get(&local(*id))
                else {
                    return Ok(None);
                };
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let bounded = types
                    .iter()
                    .try_for_each(|ty| nir::type_computations::charge_term(ty, &mut fuel));
                self.required_steps = fuel as u64;
                bounded.map_err(|e| error(value.span, e.to_string()))?;
                Ok(Some(types.clone()))
            }
            _ => Ok(None),
        }
    }

    // A common established type is only a hint. The ordinary checker still
    // validates selectors, exhaustiveness, patterns, guards and every branch.
    // Do not select a branch, coerce it, or invent types for unknown bodies.
    fn generator_branch_type_hint<'h>(
        &mut self,
        branches: impl IntoIterator<Item = &'h Expr<'h>>,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        let mut common = None;
        for branch in branches {
            let Some(ty) = self.generator_expression_type_hint(branch, depth)? else {
                return Ok(None);
            };
            if common.as_ref().is_some_and(|previous| previous != &ty) {
                return Ok(None);
            }
            common = Some(ty);
        }
        Ok(common)
    }
}
