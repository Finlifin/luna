//! Generator data frames reuse invocation-owned cells while retaining checked
//! immutable snapshots for elaboration and quote construction. Updates publish
//! snapshots together; failed execution or freezing restores the original cells.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn read_generator_cell(
        &mut self,
        cell: &comptime_interpretor::Storage,
        span: Span,
    ) -> Result<comptime_interpretor::Value, Error> {
        let package = n::Package::default();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: self.required_steps,
                depth: 128,
            },
        )
        .map_err(|failure| {
            error(
                span,
                format!("generator storage preparation failed: {failure}"),
            )
        })?;
        let value = vm.read_owned_value(cell);
        self.required_steps = vm.remaining_steps();
        value.map_err(|failure| error(span, format!("generator storage read failed: {failure}")))
    }

    pub(super) fn checkpoint_generator_cells(
        &mut self,
        roots: &[comptime_interpretor::Value],
        span: Span,
    ) -> Result<comptime_interpretor::OwnedGraphCheckpoint, Error> {
        let package = n::Package::default();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: self.required_steps,
                depth: 128,
            },
        )
        .map_err(|failure| {
            error(
                span,
                format!("generator checkpoint preparation failed: {failure}"),
            )
        })?;
        let checkpoint = vm.checkpoint_owned_graph(roots);
        self.required_steps = vm.remaining_steps();
        checkpoint.map_err(|failure| error(span, format!("generator checkpoint failed: {failure}")))
    }

    pub(in crate::program::cmtt) fn generator_value_needs_live_storage(
        &mut self,
        value: &comptime_interpretor::Value,
        span: Span,
    ) -> Result<bool, Error> {
        use comptime_interpretor::Value;
        let mut pending = vec![(value, 0usize)];
        let mut live = false;
        let mut callbacks = Vec::new();
        let mut runtime_callback = false;
        while let Some((value, depth)) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "generator live data inspection budget exhausted"))?;
            if depth >= 128 {
                return Err(error(span, "generator live data nesting limit exceeded"));
            }
            match value {
                Value::Pointer(_) => live = true,
                Value::Tuple(fields)
                | Value::Record { fields, .. }
                | Value::Variant { fields, .. } => {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(fields.len() as u64)
                        .ok_or_else(|| {
                            error(span, "generator live data inspection budget exhausted")
                        })?;
                    pending.extend(fields.iter().map(|field| (field, depth + 1)));
                }
                Value::AnonymousRecord(fields) => {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(fields.len() as u64)
                        .ok_or_else(|| {
                            error(span, "generator live data inspection budget exhausted")
                        })?;
                    pending.extend(fields.iter().map(|(_, field)| (field, depth + 1)));
                }
                Value::Newtype { value, .. } | Value::ErrorResult { value, .. } => {
                    pending.push((value, depth + 1))
                }
                Value::Closure(closure) if closure.ty().is_comptime_callable() => {
                    callbacks.push(value)
                }
                Value::Closure(_)
                | Value::RuntimeFunction { .. }
                | Value::RuntimeTraitMethod(_) => runtime_callback = true,
                _ => {}
            }
        }
        if live {
            if runtime_callback {
                return Err(error(
                    span,
                    "live generator aggregate contains a runtime callback",
                ));
            }
            self.validate_generator_live_callbacks(&callbacks, span)?;
        }
        Ok(live)
    }

    pub(super) fn refresh_generator_cells(&mut self, span: Span) -> Result<(), Error> {
        let cells = self
            .generator_cells
            .iter()
            .map(|(id, (snapshot, cell))| (*id, snapshot.ty.clone(), cell.clone()))
            .collect::<Vec<_>>();
        let mut updated = BTreeMap::new();
        for (id, ty, cell) in cells {
            let value = self.read_generator_cell(&cell, span)?;
            let snapshot = if self.generator_value_needs_live_storage(&value, span)? {
                n::Expr {
                    source: source(span),
                    ty,
                    kind: K::Local(id),
                }
            } else {
                self.materialize_generator_data(value, ty, span)?
            };
            constants::charge_copy(&snapshot, &mut self.required_steps, span)?;
            updated.insert(id, (snapshot, cell));
        }
        for (id, (snapshot, cell)) in updated {
            self.generator_data.insert(id, Some(snapshot.clone()));
            self.generator_cells.insert(id, (snapshot, cell));
        }
        Ok(())
    }

    fn generator_pointer_snapshot_checks(
        &mut self,
        id: n::LocalId,
        ty: &Term,
        cell: &comptime_interpretor::Storage,
        statements: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<(), Error> {
        let Term::Pointer(pointee) = ty else {
            return Ok(());
        };
        let comptime_interpretor::Value::Pointer(pointer) = self.read_generator_cell(cell, span)?
        else {
            return Ok(());
        };
        let package = n::Package::default();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: self.required_steps,
                depth: 128,
            },
        )
        .map_err(|e| error(span, format!("generator pointer observation failed: {e}")))?;
        let value = vm.observe_pointer_scalar(&pointer);
        self.required_steps = vm.remaining_steps();
        let Some(value) =
            value.map_err(|e| error(span, format!("generator pointer observation failed: {e}")))?
        else {
            return Ok(());
        };
        let snapshot = self.materialize_generator_data(value, (**pointee).clone(), span)?;
        let place = n::Expr {
            source: source(span),
            ty: (**pointee).clone(),
            kind: K::ReadPointer(Box::new(n::Expr {
                source: source(span),
                ty: ty.clone(),
                kind: K::Local(id),
            })),
        };
        self.generator_snapshot_checks(place, &snapshot, statements, span, 0)
    }

    fn generator_snapshot_checks(
        &mut self,
        place: n::Expr,
        snapshot: &n::Expr,
        statements: &mut Vec<n::Expr>,
        span: Span,
        depth: usize,
    ) -> Result<(), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "generator snapshot check budget exhausted"))?;
        if depth >= 128 {
            return Err(error(
                span,
                "generator snapshot check nesting limit exceeded",
            ));
        }
        match &snapshot.kind {
            K::Literal(literal) => {
                constants::charge_copy(snapshot, &mut self.required_steps, span)?;
                let nan = match literal {
                    n::Literal::Float32(bits) => f32::from_bits(*bits).is_nan(),
                    n::Literal::Float64(bits) => f64::from_bits(*bits).is_nan(),
                    _ => false,
                };
                let predicate = if nan {
                    // NaN is not equal to its saved literal. Preserve the
                    // unordered fact without inventing a reflexive equality.
                    constants::charge_copy(&place, &mut self.required_steps, span)?;
                    K::Not(Box::new(n::Expr {
                        source: source(span),
                        ty: p(P::Bool),
                        kind: K::ScalarEqual(Box::new(place.clone()), Box::new(place)),
                    }))
                } else if matches!(literal, n::Literal::BitVec { .. }) {
                    K::BitEqual(Box::new(place), Box::new(snapshot.clone()))
                } else if place.ty == p(P::Integer) {
                    K::IntegerBinary {
                        op: n::IntegerOp::Equal,
                        left: Box::new(place),
                        right: Box::new(snapshot.clone()),
                    }
                } else {
                    K::ScalarEqual(Box::new(place), Box::new(snapshot.clone()))
                };
                statements.push(n::Expr {
                    source: source(span),
                    ty: p(P::Void),
                    kind: K::CheckPredicate(Box::new(n::Expr {
                        source: source(span),
                        ty: p(P::Bool),
                        kind: predicate,
                    })),
                });
            }
            K::Record {
                declaration,
                fields,
            } => {
                for (index, field) in fields.iter().enumerate() {
                    constants::charge_copy(&place, &mut self.required_steps, span)?;
                    let projection = n::Expr {
                        source: source(span),
                        ty: field.ty.clone(),
                        kind: K::Field {
                            value: Box::new(place.clone()),
                            declaration: *declaration,
                            index,
                        },
                    };
                    self.generator_snapshot_checks(projection, field, statements, span, depth + 1)?;
                }
            }
            K::AnonymousRecord { fields } => {
                if let Term::Record(types) = self.structural_type(&snapshot.ty, span)? {
                    for (index, ((name, _), field)) in types.iter().zip(fields).enumerate() {
                        constants::charge_copy(&place, &mut self.required_steps, span)?;
                        let projection = n::Expr {
                            source: source(span),
                            ty: field.ty.clone(),
                            kind: K::AnonymousField {
                                value: Box::new(place.clone()),
                                name: name.clone(),
                                index,
                            },
                        };
                        self.generator_snapshot_checks(
                            projection,
                            field,
                            statements,
                            span,
                            depth + 1,
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn assign_generator_place(
        &mut self,
        target: &Expr<'_>,
        value: &Expr<'_>,
        op: Option<BinOp>,
    ) -> Result<(), Error> {
        let mut root = target;
        loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(target.span, "generator place traversal budget exhausted"))?;
            match root.kind {
                H::Projection(inner, _) | H::Index(inner, _) | H::TakeBitVec(inner) => root = inner,
                H::ResolvedLocal(..) | H::ResolvedFnParam(..) => {
                    if !self.generator_data.contains_key(&binding(root)?) {
                        return Err(error(
                            target.span,
                            "generator place assignment cannot modify runtime context or code storage",
                        ));
                    }
                    break;
                }
                // The pointer expression is evaluated inside the private data
                // frame. Ordinary place checking validates its pointee; free
                // binding validation rejects runtime context and code storage.
                // No address is frozen across generator statements.
                H::Deref(_) | H::SelfValue => break,
                _ => {
                    return Err(error(
                        target.span,
                        "generator assignment requires an owned data place or a temporary pointer place",
                    ));
                }
            }
        }
        let assignment = Expr {
            kind: match op {
                Some(op) => H::AssignOp(op, target, value),
                None => H::Assign(target, value),
            },
            ..target.clone()
        };
        self.generator_data_frame(&assignment, Some(&p(P::Void)))?;
        Ok(())
    }

    pub(in crate::program::cmtt) fn generator_data_expression(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<(comptime_interpretor::Value, Term), Error> {
        self.generator_data_expression_in_context(value, expected, None)
    }

    /// Data inputs to either generator category are checked at comptime even
    /// when the invocation occurs while constructing a runtime quote.
    pub(in crate::program::cmtt) fn check_generator_data_input(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
        context: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let previous = std::mem::replace(&mut self.in_required_comptime, true);
        let checked = match context {
            Some(context) => self.input_context_expression(value, context),
            None => self.expr(value, expected),
        };
        self.in_required_comptime = previous;
        let checked = checked?;
        self.validate_generator_data(&checked, value.span)?;
        Ok(checked)
    }

    pub(in crate::program::cmtt) fn generator_data_expression_in_context(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
        context: Option<&crate::calls::InputContext>,
    ) -> Result<(comptime_interpretor::Value, Term), Error> {
        if self.generator_data.is_empty() {
            let Some(context) = context else {
                return self.required_typed_expression(value, expected);
            };
            let mode = std::mem::replace(&mut self.in_required_comptime, true);
            let checked = self.input_context_expression(value, context);
            self.in_required_comptime = mode;
            return self.required_normal_input(value, checked?, vec![]);
        }
        self.generator_data_frame_in_context(value, expected, context)?
            .ok_or_else(|| {
                error(
                    value.span,
                    "generator data invocation requires completed inputs",
                )
            })
    }

    fn generator_data_frame(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<(comptime_interpretor::Value, Term)>, Error> {
        self.generator_data_frame_in_context(value, expected, None)
    }

    fn generator_data_frame_in_context(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
        context: Option<&crate::calls::InputContext>,
    ) -> Result<Option<(comptime_interpretor::Value, Term)>, Error> {
        self.generator_data_frame_with(value, |this| match context {
            Some(context) => this.input_context_expression(value, context),
            None => this.expr(value, expected),
        })
    }

    pub(super) fn generator_data_frame_with(
        &mut self,
        value: &Expr<'_>,
        check: impl FnOnce(&mut Self) -> Result<n::Expr, Error>,
    ) -> Result<Option<(comptime_interpretor::Value, Term)>, Error> {
        // Keep every owned data binding as a local while checking the update.
        // Both selectors and RHS may modify siblings or other owned objects.
        // Only referenced roots enter the invocation as shared owned cells.
        self.required_steps = self
            .required_steps
            .checked_sub(self.generator_data.len() as u64)
            .ok_or_else(|| error(value.span, "generator update frame budget exhausted"))?;
        let previous = self
            .generator_data
            .iter_mut()
            .map(|(id, value)| (*id, value.take()))
            .collect::<BTreeMap<_, _>>();
        let mode = std::mem::replace(&mut self.in_required_comptime, true);
        let mut checkpoint = None;
        let result = (|| {
            // Immutable Type values are elaboration evidence as well as
            // data. Keep them available for annotations and type arguments.
            // Mutable bindings remain symbolic until this frame executes.
            for (id, stored) in &previous {
                if self.immutable.contains(id)
                    && let Some(stored) = stored
                    && matches!(stored.kind, K::TypeValue(_) | K::TypeListValue(_))
                {
                    constants::charge_copy(stored, &mut self.required_steps, value.span)?;
                    self.generator_data.insert(*id, Some(stored.clone()));
                }
            }
            let checked = check(self)?;
            self.validate_generator_data(&checked, value.span)?;
            if self.checking_code_generator {
                return Ok(None);
            }
            let result_ty = checked.ty.clone();
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let free = nir::free_bindings::collect(&checked, &mut fuel);
            self.required_steps = fuel as u64;
            let free = free.map_err(|message| error(value.span, message))?;
            let mut statements = Vec::new();
            let mut outputs = Vec::new();
            let mut identities = Vec::new();
            let mut live = Vec::new();
            let mut cells = BTreeMap::new();
            for id in free {
                let stored = previous
                    .get(&id)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| error(value.span, "generator data place is not completed"))?;
                constants::charge_copy(stored, &mut self.required_steps, value.span)?;
                let ty =
                    self.locals.get(&id).cloned().ok_or_else(|| {
                        error(value.span, "generator data place root has no type")
                    })?;
                let cell = match self.generator_cells.get(&id) {
                    Some((snapshot, cell)) if snapshot == stored => cell.clone(),
                    _ => {
                        let (initial, _) =
                            self.required_normal_input(value, stored.clone(), vec![])?;
                        comptime_interpretor::Storage::new(initial)
                    }
                };
                live.push((
                    id,
                    ty.clone(),
                    comptime_interpretor::Value::Storage(cell.clone()),
                ));
                self.generator_pointer_snapshot_checks(
                    id,
                    &ty,
                    &cell,
                    &mut statements,
                    value.span,
                )?;
                cells.insert(id, cell);
                self.generator_snapshot_checks(
                    n::Expr {
                        source: source(value.span),
                        ty: ty.clone(),
                        kind: K::Local(id),
                    },
                    stored,
                    &mut statements,
                    value.span,
                    0,
                )?;
                outputs.push(n::Expr {
                    ty: ty.clone(),
                    source: source(value.span),
                    kind: K::Local(id),
                });
                identities.push((id, ty));
            }
            let result_id = self.spread_binding(value.span)?;
            statements.push(n::Expr {
                ty: p(P::Void),
                source: source(value.span),
                kind: K::Let {
                    binding: result_id,
                    value: Box::new(checked),
                },
            });
            outputs.push(n::Expr {
                ty: result_ty.clone(),
                source: source(value.span),
                kind: K::Local(result_id),
            });
            let ty = self.tuple_value_type(
                outputs.iter().map(|output| output.ty.clone()).collect(),
                value.span,
            )?;
            let body = n::Expr {
                ty: ty.clone(),
                source: source(value.span),
                kind: K::Block {
                    statements,
                    result: Some(Box::new(n::Expr {
                        ty,
                        source: source(value.span),
                        kind: K::Tuple(outputs),
                    })),
                },
            };
            let roots = live
                .iter()
                .map(|(_, _, value)| value.clone())
                .chain(
                    self.generator_cells
                        .values()
                        .map(|(_, cell)| comptime_interpretor::Value::Storage(cell.clone())),
                )
                .collect::<Vec<_>>();
            checkpoint = Some(self.checkpoint_generator_cells(&roots, value.span)?);
            let (updated, _) = self.required_live_input(value, body, vec![], live)?;
            let comptime_interpretor::Value::Tuple(mut updated) = updated else {
                return Err(error(
                    value.span,
                    "generator update frame returned invalid state",
                ));
            };
            if updated.len() != identities.len() + 1 {
                return Err(error(
                    value.span,
                    "generator update frame returned invalid arity",
                ));
            }
            let output = updated.pop().unwrap();
            let mut completed = BTreeMap::new();
            for ((id, ty), stored) in identities.into_iter().zip(updated) {
                let snapshot = if self.generator_value_needs_live_storage(&stored, value.span)? {
                    n::Expr {
                        source: source(value.span),
                        ty,
                        kind: K::Local(id),
                    }
                } else {
                    self.materialize_generator_data(stored, ty, value.span)?
                };
                // The cache copy and equality check are bounded by this
                // same checked snapshot; publication happens as one batch.
                constants::charge_copy(&snapshot, &mut self.required_steps, value.span)?;
                completed.insert(id, Some(snapshot));
            }
            // A pointer write may change an owner absent from this
            // expression's syntactic free bindings. Refresh those owners
            // before publishing any of the new snapshots.
            let indirect = self
                .generator_cells
                .iter()
                .filter(|(id, _)| !cells.contains_key(id))
                .map(|(id, (snapshot, cell))| (*id, snapshot.ty.clone(), cell.clone()))
                .collect::<Vec<_>>();
            for (id, ty, cell) in indirect {
                let stored = self.read_generator_cell(&cell, value.span)?;
                let snapshot = if self.generator_value_needs_live_storage(&stored, value.span)? {
                    n::Expr {
                        source: source(value.span),
                        ty,
                        kind: K::Local(id),
                    }
                } else {
                    self.materialize_generator_data(stored, ty, value.span)?
                };
                constants::charge_copy(&snapshot, &mut self.required_steps, value.span)?;
                completed.insert(id, Some(snapshot));
                cells.insert(id, cell);
            }
            Ok(Some((completed, output, result_ty, cells)))
        })();
        self.in_required_comptime = mode;
        self.generator_data = previous;
        // Publish only after every output is frozen successfully. Failed
        // invocation or materialization leaves the original environment intact.
        if result.is_err() {
            if let Some(checkpoint) = checkpoint {
                checkpoint.restore().map_err(|failure| {
                    error(
                        value.span,
                        format!("generator frame restoration failed: {failure:?}"),
                    )
                })?;
            }
        }
        if let Some((updated, output, ty, cells)) = result? {
            for (id, cell) in cells {
                let snapshot = updated.get(&id).and_then(Option::as_ref).unwrap().clone();
                self.generator_cells.insert(id, (snapshot, cell));
            }
            self.generator_data.extend(updated);
            Ok(Some((output, ty)))
        } else {
            Ok(None)
        }
    }
}
