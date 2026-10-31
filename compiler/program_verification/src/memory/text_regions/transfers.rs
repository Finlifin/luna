//! Outlined transfer functions keep recursive traversal stack frames small.
//! Depth and fuel are charged by `Query::expr`, before dispatch.
use super::*;

impl Query<'_, '_> {
    #[inline(never)]
    pub(super) fn transfer_function(
        &mut self,
        expr: &Expr,
        _locals: &mut Locals,
        _returned: &mut Option<Regions>,
        _next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Function(declaration) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let mut empty = Some(Regions::new());
        empty.as_mut().unwrap().code.insert(
            vec![],
            Some(CallbackKey::Named(iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            })),
        );
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_trait_method(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::TraitMethod { .. } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let selected = self.methods.select(self.package, expr, self.fuel)?;
        self.expr(&selected, locals, returned, next)
    }

    #[inline(never)]
    pub(super) fn transfer_function_instance(
        &mut self,
        expr: &Expr,
        _locals: &mut Locals,
        _returned: &mut Option<Regions>,
        _next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::FunctionInstance {
            declaration,
            arguments,
        } = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let mut empty = Some(Regions::new());
        empty.as_mut().unwrap().code.insert(
            vec![],
            Some(CallbackKey::Named(iir::InstanceKey {
                declaration: *declaration,
                arguments: arguments.clone(),
            })),
        );
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_local(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        _returned: &mut Option<Regions>,
        _next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Local(id) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let regions = locals.get(id).ok_or("text region local is unavailable")?;
        charge_regions(regions, self.fuel)?;
        let mut regions = regions.clone();
        if !regions.code.contains_key(&vec![])
            && matches!(expr.ty.callable_signature(), Term::Function { .. })
        {
            if let Some(code) = self.known_local_callee(*id)? {
                let code = self.close_callable(code, 0)?;
                regions.code.insert(vec![], code.key());
            }
        }
        Ok(Some(regions))
    }

    #[inline(never)]
    pub(super) fn transfer_address_of(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        _returned: &mut Option<Regions>,
        _next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::AddressOf(place) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let mut empty = Some(Regions::new());
        let mut place = place.as_ref();
        let mut path = Vec::new();
        while let K::Field { value, index, .. } | K::AnonymousField { value, index, .. } =
            &place.kind
        {
            charge(self.fuel, 1)?;
            path.push(*index);
            place = value;
        }
        let K::Local(root) = place.kind else {
            return Err("text descriptor address requires local storage".into());
        };
        if !locals.contains_key(&root) {
            return Err("text descriptor address root is unavailable".into());
        }
        path.reverse();
        empty.as_mut().unwrap().places.insert(
            vec![],
            Some(BTreeSet::from([(DescriptorRoot::Local(root), path)])),
        );
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_pointer_field(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::PointerField { pointer, index, .. }
        | K::PointerAnonymousField { pointer, index, .. }) = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let Some(mut value) = self.expr(pointer, locals, returned, next)? else {
            return Ok(None);
        };
        let target = value
            .places
            .get_mut(&vec![])
            .and_then(Option::as_mut)
            .ok_or("text descriptor pointer target is unknown")?;
        for (root, mut path) in std::mem::take(target) {
            charge(self.fuel, path.len().saturating_add(1))?;
            path.push(*index);
            target.insert((root, path));
        }
        Ok(Some(value))
    }

    #[inline(never)]
    pub(super) fn transfer_read_pointer(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::ReadPointer(pointer) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let Some(value) = self.expr(pointer, locals, returned, next)? else {
            return Ok(None);
        };
        let targets = value
            .places
            .get(&vec![])
            .and_then(Option::as_ref)
            .filter(|targets| !targets.is_empty())
            .ok_or("text descriptor pointer target is unknown")?;
        let mut result = None;
        for (root, path) in targets {
            let value = locals
                .get_storage(root)
                .ok_or("text descriptor storage is no longer live")?;
            charge_regions(value, self.fuel)?;
            merge(
                &mut result,
                Some(project(value.clone(), path, self.fuel)?),
                self.fuel,
            )?;
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn transfer_write_pointer(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::WritePointer { pointer, value } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        if let Some(message) = self.readonly_storage {
            return Err(message.into());
        }
        let Some(pointer) = self.expr(pointer, locals, returned, next)? else {
            return Ok(None);
        };
        let targets = pointer
            .places
            .get(&vec![])
            .and_then(Option::as_ref)
            .filter(|targets| !targets.is_empty())
            .ok_or("text descriptor pointer target is unknown")?;
        let Some(value) = self.expr(value, locals, returned, next)? else {
            return Ok(None);
        };
        for (root, path) in targets {
            charge_regions(&value, self.fuel)?;
            let replacement = prefix(value.clone(), path, self.fuel)?;
            let target = locals
                .get_storage_mut(root)
                .ok_or("text descriptor storage is no longer live")?;
            charge_regions(target, self.fuel)?;
            if targets.len() == 1 {
                target.text.retain(|fields, _| !fields.starts_with(path));
                target.code.retain(|fields, _| !fields.starts_with(path));
                target.places.retain(|fields, _| !fields.starts_with(path));
                target.text.extend(replacement.text);
                target.code.extend(replacement.code);
                target.places.extend(replacement.places);
            } else {
                // Only one candidate is written at runtime. Keep each
                // candidate's old value for executions selecting another.
                let mut joined = Some(target.clone());
                merge(&mut joined, Some(replacement), self.fuel)?;
                *target = joined.unwrap();
            }
        }
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_symbol_name(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::SymbolName(value) | K::EnumName(value)) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        if self.expr(value, locals, returned, next)?.is_none() {
            return Ok(None);
        }
        Ok(Some(Regions::from(BTreeMap::from([(
            vec![],
            BTreeSet::from([TextRegionOrigin::Static]),
        )]))))
    }

    #[inline(never)]
    pub(super) fn transfer_tuple(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::Tuple(fields) | K::Record { fields, .. } | K::AnonymousRecord { fields }) =
            &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let mut empty = Some(Regions::new());
        for (index, field) in fields.iter().enumerate() {
            let Some(value) = self.expr(field, locals, returned, next)? else {
                return Ok(None);
            };
            merge(
                &mut empty,
                Some(prefix(value, &[index], self.fuel)?),
                self.fuel,
            )?;
        }
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_variant(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Variant { index, fields, .. } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let mut empty = Some(Regions::new());
        for (field, value) in fields.iter().enumerate() {
            let Some(value) = self.expr(value, locals, returned, next)? else {
                return Ok(None);
            };
            merge(
                &mut empty,
                Some(prefix(value, &[VARIANT_FIELD, *index, field], self.fuel)?),
                self.fuel,
            )?;
        }
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_let(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Let { binding, value } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        let Some(value) = self.expr(value, locals, returned, next)? else {
            return Ok(None);
        };
        locals.insert(*binding, value);
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_let_pattern(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::LetPattern {
            pattern,
            value,
            guard,
            otherwise,
        } = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        let Some(regions) = self.expr(value, locals, returned, next)? else {
            return Ok(None);
        };
        let bindings: Vec<_> = locals.keys().copied().collect();
        let mut candidate = self.locals_copy(locals)?;
        let matched = self.bind_pattern(
            pattern,
            &value.ty,
            &regions,
            Some(value),
            &mut candidate,
            next,
        )?;
        let mut success = matched != Some(false);
        let mut failure = if matched != Some(true) {
            Some(self.locals_copy(locals)?)
        } else {
            None
        };
        if success {
            if let Some(guard) = guard {
                if self.expr(guard, &mut candidate, returned, next)?.is_none() {
                    success = false;
                } else {
                    let condition = constant_bool(guard, 0);
                    if condition != Some(true) {
                        join_locals(&mut failure, &candidate, &bindings, self.fuel)?;
                    }
                    success = condition != Some(false);
                }
            }
        }
        if let (Some(mut failure), Some(otherwise)) = (failure, otherwise) {
            // Failure receives only the outer environment. Candidate
            // names must not shadow outer bindings in the else branch.
            if self
                .expr(otherwise, &mut failure, returned, next)?
                .is_some()
            {
                return Err("text let-else failure branch must not complete normally".into());
            }
        }
        if success {
            *locals = candidate;
            Ok(empty)
        } else {
            Ok(None)
        }
    }

    #[inline(never)]
    pub(super) fn transfer_assign(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Assign { place, value } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        let Some(value) = self.expr(value, locals, returned, next)? else {
            return Ok(None);
        };
        charge(self.fuel, place.fields.len())?;
        let path: Vec<_> = place.fields.iter().map(|(_, index)| *index).collect();
        let replacement = prefix(value, &path, self.fuel)?;
        let target = locals
            .get_mut(&place.root)
            .ok_or("text region assignment root is unavailable")?;
        charge_regions(target, self.fuel)?;
        // A write replaces exactly this subtree, including an absent
        // optional payload. Unrelated sibling dependencies survive.
        target.retain(|fields, _| !fields.starts_with(&path));
        target.code.retain(|fields, _| !fields.starts_with(&path));
        target.places.retain(|fields, _| !fields.starts_with(&path));
        target.places.extend(replacement.places);
        target.code.extend(replacement.code);
        target.text.extend(replacement.text);
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_block(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::Block { statements, result } | K::CallPreparation { statements, result }) =
            &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        let mut scope = self.locals_copy(locals)?;
        for statement in statements {
            if self.expr(statement, &mut scope, returned, next)?.is_none() {
                return Ok(None);
            }
        }
        let result = match result {
            Some(result) => self.expr(result, &mut scope, returned, next),
            None => Ok(empty),
        }?;
        if result.is_some() {
            // Only outer bindings escape the lexical scope. Writes
            // made before a return have no fallthrough successor.
            for value in result
                .iter()
                .chain(locals.keys().filter_map(|id| scope.get(id)))
                .chain(scope.frames.values())
            {
                for (root, _) in value.places.values().flatten().flatten() {
                    if !locals.contains_storage(root) {
                        return Err("text descriptor address escapes its local scope".into());
                    }
                }
            }
            locals.frames = std::mem::take(&mut scope.frames);
            for (id, value) in locals.iter_mut() {
                *value = scope.remove(id).ok_or("text region scope lost a local")?;
            }
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn transfer_if(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::If {
            condition,
            then_branch,
            else_branch,
        } = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        if self.expr(condition, locals, returned, next)?.is_none() {
            return Ok(None);
        }
        if let Some(condition) = constant_bool(condition, 0) {
            return match (condition, else_branch) {
                (true, _) => self.expr(then_branch, locals, returned, next),
                (false, Some(branch)) => self.expr(branch, locals, returned, next),
                (false, None) => Ok(empty),
            };
        }
        let mut left = self.locals_copy(locals)?;
        let mut right = self.locals_copy(locals)?;
        let mut result = self.expr(then_branch, &mut left, returned, next)?;
        let otherwise = match else_branch {
            Some(branch) => self.expr(branch, &mut right, returned, next)?,
            None => empty,
        };
        for (id, value) in &mut locals.frames {
            let mut joined = None;
            if result.is_some() {
                merge(&mut joined, left.frames.remove(id), self.fuel)?;
            }
            if otherwise.is_some() {
                merge(&mut joined, right.frames.remove(id), self.fuel)?;
            }
            if let Some(joined) = joined {
                *value = joined;
            }
        }
        for (id, value) in locals.iter_mut() {
            let mut joined = None;
            if result.is_some() {
                merge(&mut joined, left.remove(id), self.fuel)?;
            }
            if otherwise.is_some() {
                merge(&mut joined, right.remove(id), self.fuel)?;
            }
            if let Some(joined) = joined {
                *value = joined;
            }
        }
        merge(&mut result, otherwise, self.fuel)?;
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn transfer_match(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Match { value, arms } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let Some(regions) = self.expr(value, locals, returned, next)? else {
            return Ok(None);
        };
        let bindings: Vec<_> = locals.keys().copied().collect();
        let mut pending = Some(self.locals_copy(locals)?);
        let mut outgoing = None;
        let mut result = None;
        for arm in arms {
            let Some(input) = pending.take() else { break };
            let mut pattern = &arm.pattern;
            let mut guards = Vec::new();
            while let Pattern::Guard {
                pattern: inner,
                condition,
            } = pattern
            {
                charge(self.fuel, 1)?;
                guards.push(condition.as_ref());
                pattern = inner;
            }
            guards.reverse();
            if let Some(guard) = &arm.guard {
                guards.push(guard);
            }
            let mut scope = self.locals_copy(&input)?;
            let matches =
                self.bind_pattern(pattern, &value.ty, &regions, Some(value), &mut scope, next)?;
            if matches == Some(false) {
                pending = Some(input);
                continue;
            }
            if matches != Some(true) {
                pending = Some(input);
            }
            let mut accepted = true;
            for guard in guards {
                if self.expr(guard, &mut scope, returned, next)?.is_none() {
                    accepted = false;
                    break;
                }
                let condition = constant_bool(guard, 0);
                if condition != Some(true) {
                    // Guard effects survive candidate failure; pattern
                    // bindings do not escape into the next candidate.
                    join_locals(&mut pending, &scope, &bindings, self.fuel)?;
                }
                if condition == Some(false) {
                    accepted = false;
                    break;
                }
            }
            if !accepted {
                continue;
            }
            let body = self.expr(&arm.body, &mut scope, returned, next)?;
            if body.is_some() {
                join_locals(&mut outgoing, &scope, &bindings, self.fuel)?;
            }
            merge(&mut result, body, self.fuel)?;
        }
        if let Some(outgoing) = outgoing {
            *locals = outgoing;
        }
        Ok(result)
    }

    #[inline(never)]
    pub(super) fn transfer_loop(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Loop { identity, body } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        let mut head = self.locals_copy(locals)?;
        let bindings: Vec<_> = head.keys().copied().collect();
        let mut exits = None;
        loop {
            let mut iteration = self.locals_copy(&head)?;
            self.loops.push(LoopRegions {
                identity: *identity,
                bindings: bindings.clone(),
                exits: None,
                backedges: None,
            });
            let result = self.expr(body, &mut iteration, returned, next);
            let mut flow = self.loops.pop().unwrap();
            if result?.is_some() {
                join_locals(&mut flow.backedges, &iteration, &bindings, self.fuel)?;
            }
            if let Some(outgoing) = flow.exits {
                join_locals(&mut exits, &outgoing, &bindings, self.fuel)?;
            }
            let Some(backedges) = flow.backedges else {
                break;
            };
            let mut joined = Some(self.locals_copy(&head)?);
            join_locals(&mut joined, &backedges, &bindings, self.fuel)?;
            let joined = joined.unwrap();
            if joined == head {
                break;
            }
            head = joined;
        }
        if let Some(exits) = exits {
            *locals = exits;
            Ok(empty)
        } else {
            Ok(None)
        }
    }

    #[inline(never)]
    pub(super) fn transfer_break(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        _returned: &mut Option<Regions>,
        _next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::Break(identity) | K::Continue(identity)) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let flow = self
            .loops
            .iter_mut()
            .rev()
            .find(|flow| flow.identity == *identity)
            .ok_or("text region loop target is unavailable")?;
        let target = if matches!(&expr.kind, K::Break(_)) {
            &mut flow.exits
        } else {
            &mut flow.backedges
        };
        join_locals(target, locals, &flow.bindings, self.fuel)?;
        Ok(None)
    }

    #[inline(never)]
    pub(super) fn transfer_return(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Return(value) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        let value = match value {
            Some(value) => self.expr(value, locals, returned, next)?,
            None => empty,
        };
        if value.is_some() {
            self.record_exit(locals)?;
        }
        merge(returned, value, self.fuel)?;
        Ok(None)
    }

    #[inline(never)]
    pub(super) fn transfer_call(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let Some(callee_value) = self.expr(callee, locals, returned, next)? else {
            return Ok(None);
        };
        if let Some(Some(CallbackKey::Alternatives(candidates))) = callee_value.code.get(&vec![]) {
            return self.named_call(
                candidates,
                arguments,
                parameter_bindings,
                locals,
                returned,
                next,
            );
        }
        if let Some(Some(CallbackKey::ClosureAlternatives(candidates))) =
            callee_value.code.get(&vec![])
        {
            return self.closed_callback_choices(
                candidates,
                &callee_value,
                callee,
                arguments,
                parameter_bindings,
                locals,
                returned,
                next,
            );
        }
        let resolved = self
            .result_code(&callee_value, callee)?
            .ok_or("text region call requires a named concrete callee or stable local callable")?;
        let code = resolved.expression.as_ref();
        if matches!(code.kind, K::Closure { .. }) {
            let id = resolved
                .closure
                .ok_or("text closure has no closed identity")?;
            return self.closed_callback_choices(
                &[id],
                &callee_value,
                callee,
                arguments,
                parameter_bindings,
                locals,
                returned,
                next,
            );
        }
        let (declaration, types) = match &code.kind {
            K::Function(id) => (*id, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            _ => return Err("text region call requires a named concrete callee".into()),
        };
        self.named_call(
            &[iir::InstanceKey {
                declaration,
                arguments: types.to_vec(),
            }],
            arguments,
            parameter_bindings,
            locals,
            returned,
            next,
        )
    }

    #[inline(never)]
    pub(super) fn transfer_text_query(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::TextQuery { subject, query } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        let Some(value) = self.expr(subject, locals, returned, next)? else {
            return Ok(None);
        };
        if *query == nir::program::TextQueryKind::SymbolByte {
            // Symbol names have immutable storage independent of the
            // local scalar that carries the symbol identity.
            self.reads
                .last_mut()
                .unwrap()
                .insert(TextRegionOrigin::Static);
        } else if *query == nir::program::TextQueryKind::Byte {
            let origins = value
                .get(&vec![0])
                .filter(|origins| !origins.is_empty())
                .ok_or("text byte read has no backing-region origin")?;
            charge(self.fuel, origins.len())?;
            self.reads
                .last_mut()
                .unwrap()
                .extend(origins.iter().cloned());
        }
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_newtype_convert(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::NewtypeConvert(value) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let Some(regions) = self.expr(value, locals, returned, next)? else {
            return Ok(None);
        };
        let conversion =
            nir::newtypes::conversion(&self.package.newtypes, &value.ty, &expr.ty, &[], self.fuel)
                .map_err(|error| error.to_string())?
                .ok_or("invalid text view newtype conversion edge")?;
        let wraps = conversion.direction() == nir::newtypes::Direction::Wrap;
        Ok(Some(if wraps {
            prefix(regions, &[0], self.fuel)?
        } else {
            project(regions, &[0], self.fuel)?
        }))
    }

    #[inline(never)]
    pub(super) fn transfer_error_embedding(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::ErrorConvert {
            conversion:
                nir::error_results::Conversion::Embed | nir::error_results::Conversion::Product,
            value,
        } = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let mapping = error_tag_mapping(expr, Some(self.package), self.fuel)?;
        let Some(regions) = self.expr(value, locals, returned, next)? else {
            return Ok(None);
        };
        charge_regions(&regions, self.fuel)?;
        let mut transported = Regions::new();
        for (mut path, place) in regions.places {
            remap_error_path(&mut path, &mapping, self.fuel)?;
            merge_places(&mut transported.places, BTreeMap::from([(path, place)]));
        }
        for (mut path, code) in regions.code {
            remap_error_path(&mut path, &mapping, self.fuel)?;
            merge_known(
                &mut transported.code,
                BTreeMap::from([(path, code)]),
                self.fuel,
            )?;
        }
        for (mut path, origins) in regions.text {
            // Repack the destination slot, not its backing origin:
            // caller parameter paths still identify the original input.
            remap_error_path(&mut path, &mapping, self.fuel)?;
            transported.entry(path).or_default().extend(origins);
        }
        Ok(Some(transported))
    }

    #[inline(never)]
    pub(super) fn transfer_error_constructor(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::ErrorConvert { .. } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let (fields, tag) = owned_constructor(expr, self.fuel)?;
        let [value] = fields else {
            return Err("text error transport requires one payload".into());
        };
        self.expr(value, locals, returned, next)?
            .map(|value| prefix(value, &[VARIANT_FIELD, tag.unwrap(), 0], self.fuel))
            .transpose()
    }

    #[inline(never)]
    pub(super) fn transfer_closure(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Closure { captures, .. } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let mut empty = Some(Regions::new());
        let mut captured_code = Vec::new();
        for (index, capture) in captures.iter().enumerate() {
            let value = if let Some(value) = &capture.value {
                let Some(value) = self.expr(value, locals, returned, next)? else {
                    return Ok(None);
                };
                value
            } else {
                let local = Expr {
                    source: expr.source,
                    ty: capture.ty.clone(),
                    kind: K::Local(capture.binding),
                };
                self.expr(&local, locals, returned, next)?
                    .ok_or("text capture origin is unavailable")?
            };
            let code = self.result_code(
                &value,
                &Expr {
                    source: expr.source,
                    ty: capture.ty.clone(),
                    kind: K::Local(capture.binding),
                },
            )?;
            captured_code.push((capture.binding, code));
            merge(
                &mut empty,
                Some(prefix(value, &[index], self.fuel)?),
                self.fuel,
            )?;
        }
        for (binding, code) in &mut captured_code {
            *code = match code.take() {
                Some(code) => self.local_callables.insert(*binding, code),
                None => self.local_callables.remove(binding),
            };
        }
        let code = self.callable_input(expr);
        for (binding, previous) in captured_code {
            self.local_callables.remove(&binding);
            if let Some(previous) = previous {
                self.local_callables.insert(binding, previous);
            }
        }
        empty
            .as_mut()
            .unwrap()
            .code
            .insert(vec![], code?.and_then(|code| code.key()));
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_integer_binary(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::IntegerBinary { left, right, .. }
        | K::FloatBinary { left, right, .. }
        | K::BitEqual(left, right)
        | K::BitConcat(left, right)
        | K::BitIndex(left, right)
        | K::TemplateEqual(left, right)) = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        for operand in [left, right] {
            if self.expr(operand, locals, returned, next)?.is_none() {
                return Ok(None);
            }
        }
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_bit_window(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::BitWindow { value, start, end } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        for operand in [value, start, end] {
            if self.expr(operand, locals, returned, next)?.is_none() {
                return Ok(None);
            }
        }
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_intrinsic(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let K::Intrinsic { inputs, .. } = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        // Only admitted scalar operations have no hidden memory
        // behavior. Their source-ordered inputs may still read text.
        nir::intrinsics::checked(expr).map_err(|e| e.to_string())?;
        for input in inputs {
            if self.expr(input, locals, returned, next)?.is_none() {
                return Ok(None);
            }
        }
        Ok(empty)
    }

    #[inline(never)]
    pub(super) fn transfer_and(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::And(left, right) | K::Or(left, right)) = &expr.kind else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        if self.expr(left, locals, returned, next)?.is_none() {
            return Ok(None);
        }
        let runs_when = matches!(&expr.kind, K::And(..));
        match constant_bool(left, 0) {
            Some(value) if value != runs_when => Ok(empty),
            Some(_) => Ok(self
                .expr(right, locals, returned, next)?
                .map(|_| Regions::new())),
            None => {
                let mut taken = self.locals_copy(locals)?;
                if self.expr(right, &mut taken, returned, next)?.is_some() {
                    let bindings: Vec<_> = locals.keys().copied().collect();
                    let mut joined = Some(self.locals_copy(locals)?);
                    join_locals(&mut joined, &taken, &bindings, self.fuel)?;
                    *locals = joined.unwrap();
                }
                // The short-circuited path still continues even when
                // evaluating the right operand would return or diverge.
                Ok(empty)
            }
        }
    }

    #[inline(never)]
    pub(super) fn transfer_scalar_equal(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        next: usize,
    ) -> Result<Option<Regions>, String> {
        let (K::ScalarEqual(a, b)
        | K::ScalarLess(a, b)
        | K::ScalarLessEqual(a, b)
        | K::ScalarGreater(a, b)
        | K::ScalarGreaterEqual(a, b)) = &expr.kind
        else {
            unreachable!("checked text transfer variant")
        };
        let empty = Some(Regions::new());
        for operand in [a, b] {
            let Some(regions) = self.expr(operand, locals, returned, next)? else {
                return Ok(None);
            };
            // Text comparison observes characters, not just the view
            // descriptor. Non-text scalar operands have no regions.
            charge_regions(&regions, self.fuel)?;
            for origins in regions.text.into_values() {
                self.reads.last_mut().unwrap().extend(origins);
            }
        }
        Ok(empty)
    }
}
