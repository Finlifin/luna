use super::*;
use nir::program::{MatchArm, Pattern};
type Receipt = BTreeMap<LocalId, (Value, Value)>;

impl Emitter<'_> {
    fn accept(&mut self, condition: Value, failure: &str) -> Result<(), String> {
        if self.terminated {
            return Ok(());
        }
        if condition.ty != Ty::Bool {
            return Err("host pattern condition is not bool".into());
        }
        let success = self.fresh();
        self.line(format!(
            "br i1 {}, label %{success}, label %{failure}",
            condition.text
        ));
        self.label(&success);
        Ok(())
    }

    fn pattern(
        &mut self,
        pattern: &Pattern,
        value: &Value,
        failure: &str,
        depth: usize,
        path: Option<Vec<usize>>,
        receipt: &mut Receipt,
    ) -> Result<(), String> {
        if self.terminated {
            return Ok(());
        }
        if depth >= 128 || *self.fuel == 0 {
            return Err("host pattern lowering budget exhausted".into());
        }
        *self.fuel -= 1;
        let depth = depth + 1;
        match pattern {
            Pattern::Tuple(fields) if fields.is_empty() && value.ty == Ty::Unit => {}
            Pattern::Tuple(fields)
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Record { fields, .. } => {
                let Ty::Record(layout) = &value.ty else {
                    return Err("host record pattern subject has no record layout".into());
                };
                if fields.len() != layout.len() {
                    return Err("host record pattern field count mismatch".into());
                }
                for (index, pattern) in fields.iter().enumerate() {
                    if self.terminated {
                        break;
                    }
                    let field = self.record_field(value, index)?;
                    self.pattern(
                        pattern,
                        &field,
                        failure,
                        depth,
                        child_path(&path, index),
                        receipt,
                    )?;
                }
            }
            Pattern::OptionalAbsent | Pattern::OptionalPresent(_) => {
                let tag = self.optional_tag(value)?;
                let condition = if matches!(pattern, Pattern::OptionalAbsent) {
                    self.instruction(Ty::Bool, format!("xor i1 {}, true", tag.text))
                } else {
                    tag
                };
                self.accept(condition, failure)?;
                if let Pattern::OptionalPresent(inner) = pattern {
                    let payload = self.optional_payload(value)?;
                    self.pattern(
                        inner,
                        &payload,
                        failure,
                        depth,
                        child_path(&path, 0),
                        receipt,
                    )?;
                }
            }
            Pattern::ErrorResult { label, inner } => {
                let index = self.error_index(&value.ty, label.as_ref())?;
                let tag = self.error_tag(value)?;
                let condition =
                    self.instruction(Ty::Bool, format!("icmp eq i32 {}, {index}", tag.text));
                self.accept(condition, failure)?;
                let payload = self.error_payload(value, index)?;
                self.pattern(
                    inner,
                    &payload,
                    failure,
                    depth,
                    child_path(&path, 0),
                    receipt,
                )?;
            }
            Pattern::Variant { index, fields, .. } => {
                let condition = self.variant_condition(value, *index)?;
                self.accept(condition, failure)?;
                for (field, pattern) in fields.iter().enumerate() {
                    if self.terminated {
                        break;
                    }
                    let payload = self.variant_field(value, *index, field)?;
                    self.pattern(
                        pattern,
                        &payload,
                        failure,
                        depth,
                        child_path(&path, field),
                        receipt,
                    )?;
                }
            }
            Pattern::Wild => {}
            Pattern::Unit if value.ty == Ty::Unit => {}
            Pattern::Literal(literal) => {
                let other = self.literal(literal, value.ty.clone())?;
                let equal = if matches!(
                    value.ty.clone(),
                    Ty::Float16 | Ty::BFloat16 | Ty::Float32 | Ty::Float64
                ) {
                    self.instruction(
                        Ty::Bool,
                        format!(
                            "fcmp oeq {} {}, {}",
                            value.ty.llvm(),
                            value.text,
                            other.text
                        ),
                    )
                } else {
                    self.compare("eq", value.clone(), other)?
                };
                self.accept(equal, failure)?;
            }
            Pattern::Binding { binding, inner } => {
                self.pattern(inner, value, failure, depth, path.clone(), receipt)?;
                if !self.terminated {
                    self.bind(*binding, value)?;
                    let code = match &path {
                        Some(path) => self.receipt_path(path)?.to_string(),
                        None => "-1".into(),
                    };
                    receipt.insert(
                        *binding,
                        (
                            Value {
                                ty: Ty::Int(32, false),
                                text: code,
                            },
                            Value {
                                ty: Ty::Int(32, false),
                                text: receipt.len().to_string(),
                            },
                        ),
                    );
                }
            }
            Pattern::Observe { binding, inner } => {
                let previous = self.locals.get(binding).cloned();
                self.bind(*binding, value)?;
                self.pattern(inner, value, failure, depth, path.clone(), receipt)?;
                self.locals.remove(binding);
                if let Some(previous) = previous {
                    self.locals.insert(*binding, previous);
                }
            }
            Pattern::Guard { pattern, condition } => {
                self.pattern(pattern, value, failure, depth, path.clone(), receipt)?;
                let condition = self.expr(condition, depth)?;
                self.accept(condition, failure)?;
            }
            Pattern::Then {
                first,
                value: expression,
                next,
            } => {
                self.pattern(first, value, failure, depth, path.clone(), receipt)?;
                let next_value = self.expr(expression, depth)?;
                self.pattern(next, &next_value, failure, depth, None, receipt)?;
            }
            Pattern::Remap { pattern, bindings } => {
                self.pattern(pattern, value, failure, depth, path.clone(), receipt)?;
                if self.terminated {
                    return Ok(());
                }
                let mapped = bindings
                    .iter()
                    .map(|(from, to)| {
                        self.locals
                            .get(from)
                            .cloned()
                            .map(|slot| (*to, slot))
                            .ok_or("host pattern remap source missing")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mapped_receipt = bindings
                    .iter()
                    .map(|(from, to)| {
                        receipt
                            .get(from)
                            .cloned()
                            .map(|value| (*to, value))
                            .ok_or("host remap receipt missing")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for (from, _) in bindings {
                    self.locals.remove(from);
                    receipt.remove(from);
                }
                let destinations = mapped.iter().map(|(id, _)| *id).collect::<Vec<_>>();
                self.locals.extend(mapped);
                for id in destinations {
                    self.set_live(id, &[], true)?;
                }
                receipt.extend(mapped_receipt);
            }
            Pattern::Or(alternatives) => {
                let outer = self.locals.clone();
                let original_receipt = receipt.clone();
                let mut receipt_slots: Option<BTreeMap<LocalId, (Slot, Slot)>> = None;
                let success = self.fresh();
                let mut merged: Option<BTreeMap<LocalId, Slot>> = None;
                for alternative in alternatives {
                    let next = self.fresh();
                    self.locals = outer.clone();
                    *receipt = original_receipt.clone();
                    self.pattern(alternative, value, &next, depth, path.clone(), receipt)?;
                    if !self.terminated {
                        let exports: BTreeMap<_, _> = self
                            .locals
                            .iter()
                            .filter(|(id, _)| !outer.contains_key(id))
                            .map(|(id, slot)| (*id, slot.clone()))
                            .collect();
                        if merged.is_none() {
                            merged = Some(
                                exports
                                    .iter()
                                    .map(|(id, slot)| (*id, self.slot(slot.ty.clone())))
                                    .collect(),
                            );
                        }
                        let destinations = merged.as_ref().unwrap();
                        if !exports.keys().eq(destinations.keys()) {
                            return Err("host alternative pattern exports differ".into());
                        }
                        for (id, source) in exports {
                            let current = self.load(&source);
                            self.store(&destinations[&id], &current)?;
                        }
                        if receipt_slots.is_none() {
                            receipt_slots = Some(
                                receipt
                                    .keys()
                                    .map(|id| {
                                        (
                                            *id,
                                            (
                                                self.slot(Ty::Int(32, false)),
                                                self.slot(Ty::Int(32, false)),
                                            ),
                                        )
                                    })
                                    .collect(),
                            );
                        }
                        let slots = receipt_slots.as_ref().unwrap();
                        if !slots.keys().eq(receipt.keys()) {
                            return Err("host alternative receipt bindings differ".into());
                        }
                        for (id, (path, rank)) in receipt.iter() {
                            self.store(&slots[id].0, path)?;
                            self.store(&slots[id].1, rank)?;
                        }
                        self.branch(&success);
                    }
                    self.label(&next);
                }
                self.branch(failure);
                self.locals = outer;
                if let Some(merged) = merged {
                    self.label(&success);
                    let destinations = merged.keys().copied().collect::<Vec<_>>();
                    self.locals.extend(merged);
                    for id in destinations {
                        self.set_live(id, &[], true)?;
                    }
                    receipt.clear();
                    for (id, (path, rank)) in receipt_slots.unwrap_or_default() {
                        receipt.insert(id, (self.load(&path), self.load(&rank)));
                    }
                }
            }
            _ => return Err("host aggregate/resource pattern is not implemented".into()),
        }
        Ok(())
    }

    pub(super) fn let_pattern(
        &mut self,
        source: nir::program::SourceRange,
        pattern: &Pattern,
        expression: &Expr,
        guard: Option<&Expr>,
        otherwise: Option<&Expr>,
        depth: usize,
    ) -> Result<Value, String> {
        let value = self.expr(expression, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let outer = self.locals.clone();
        let failure = self.fresh();
        let done = self.fresh();
        let mut receipt = Receipt::new();
        self.pattern(pattern, &value, &failure, depth, Some(vec![]), &mut receipt)?;
        if let Some(guard) = guard {
            let condition = self.expr(guard, depth)?;
            self.accept(condition, &failure)?;
        }
        let continues = !self.terminated;
        let matched = self.locals.clone();
        if continues {
            self.retain_pattern(source, &value, &receipt)?;
            self.branch(&done);
        }
        self.label(&failure);
        self.locals = outer;
        if let Some(otherwise) = otherwise {
            self.retain_pattern(expression.source, &value, &Receipt::new())?;
            self.expr(otherwise, depth)?;
            if !self.terminated {
                return Err("host let-else failure must diverge".into());
            }
        } else {
            self.line("call void @llvm.trap()");
            self.line("unreachable");
            self.terminated = true;
        }
        self.locals = matched;
        if continues {
            self.label(&done);
        }
        Ok(Value::unit())
    }

    pub(super) fn match_with_fallback(
        &mut self,
        value: Value,
        arms: &[MatchArm],
        ty: &Term,
        depth: usize,
        fallback: Option<std::sync::Arc<effects::DirectHandler>>,
        handler: Option<effects::HandlerInvocation<'_>>,
        selected: Option<&effects::Environment>,
    ) -> Result<Value, String> {
        let ty = if *ty == Term::Primitive(P::Never) {
            Ty::Unit
        } else {
            return_repr(self.graph, ty, self.checked)?
        };
        let result = self.slot(ty);
        let end = self.fresh();
        let previous_resume = if handler.is_some() {
            self.effect_resume
                .replace((result.clone(), end.clone(), false))
        } else {
            None
        };
        let outer = self.locals.clone();
        let lexical_loops = self.loops.clone();
        let mut continues = false;
        for (arm_index, arm) in arms.iter().enumerate() {
            let suspended = handler
                .and_then(|handler| handler.scope)
                .map(|scope| {
                    self.enter_local_scope_at(
                        &scope.instance,
                        &scope.context,
                        iir::cleanup::ContextKey::EffectHandler {
                            installation: scope.installation,
                            handler: scope.handler,
                            arm: arm_index,
                        },
                    )
                })
                .transpose()?;
            let next = self.fresh();
            self.locals = outer.clone();
            self.loops = lexical_loops.clone();
            let outcome = (|| {
                let mut receipt = Receipt::new();
                self.pattern(
                    &arm.pattern,
                    &value,
                    &next,
                    depth,
                    Some(vec![]),
                    &mut receipt,
                )?;
                if let Some(guard) = &arm.guard {
                    let condition = self.expr(guard, depth)?;
                    if !self.terminated {
                        self.accept(condition, &next)?;
                    }
                }
                if !self.terminated {
                    self.retain_pattern(arm.body.source, &value, &receipt)?;
                }
                let selected = self.expr_with_effects(&arm.body, depth, selected)?;
                if !self.terminated {
                    if handler.is_some() {
                        self.cleanup_boundary(arm.body.source, iir::cleanup::Boundary::HandlerEnd)?;
                    } else if !matches!(arm.body.kind, K::Block { .. } | K::CallPreparation { .. })
                    {
                        self.cleanup_boundary(arm.body.source, iir::cleanup::Boundary::ScopeEnd)?;
                    }
                    self.store(&result, &selected)?;
                    self.branch(&end);
                    continues = true;
                }
                Ok::<(), String>(())
            })();
            if let Some(suspended) = suspended {
                self.restore_local_frame(suspended);
            }
            outcome?;
            self.label(&next);
        }
        if let Some(fallback) = fallback {
            let selected = self.dispatch_direct(
                fallback,
                value,
                depth + 1,
                handler
                    .ok_or("host fallback has no handler invocation")?
                    .request,
            )?;
            if !self.terminated {
                self.store(&result, &selected)?;
                self.branch(&end);
                continues = true;
            }
        } else {
            // A total match must never deliver an uninitialized result.
            self.line("call void @llvm.trap()");
            self.line("unreachable");
            self.terminated = true;
        }
        if handler.is_some() {
            continues |= self.effect_resume.as_ref().is_some_and(|target| target.2);
            self.effect_resume = previous_resume;
        }
        self.locals = outer;
        if continues {
            self.label(&end);
            Ok(self.load(&result))
        } else {
            Ok(Value::unit())
        }
    }
}

impl Emitter<'_> {
    pub(super) fn retain_temporary(
        &mut self,
        source: nir::program::SourceRange,
        value: &Value,
    ) -> Result<Option<Slot>, String> {
        self.retain_pattern(source, value, &Receipt::new())?;
        Ok(self.temporaries.get(&(source.start, source.end)).cloned())
    }

    fn retain_pattern(
        &mut self,
        source: nir::program::SourceRange,
        value: &Value,
        receipt: &Receipt,
    ) -> Result<(), String> {
        let Some(schedule) = self.cleanup else {
            return Ok(());
        };
        let needed = schedule.sites.iter().flat_map(|site| &site.actions).any(|action|
            matches!(action.target, iir::cleanup::Target::Temporary {source: origin, ..} if origin == source)
            && super::cleanup::has_invoke(&action.program.steps));
        for order in &schedule.pattern_orders {
            if order.source != source {
                continue;
            }
            let mut permitted = Value {
                ty: Ty::Bool,
                text: "false".into(),
            };
            let mut selected = Value {
                ty: Ty::Int(32, false),
                text: "-1".into(),
            };
            for (index, alternative) in order.alternatives.iter().enumerate() {
                if alternative.len() != receipt.len() {
                    continue;
                }
                let mut valid = Value {
                    ty: Ty::Bool,
                    text: "true".into(),
                };
                for (rank, id) in alternative.iter().enumerate() {
                    let Some((_, actual)) = receipt.get(id) else {
                        valid.text = "false".into();
                        break;
                    };
                    let equal =
                        self.instruction(Ty::Bool, format!("icmp eq i32 {}, {rank}", actual.text));
                    valid = self
                        .instruction(Ty::Bool, format!("and i1 {}, {}", valid.text, equal.text));
                }
                permitted = self.instruction(
                    Ty::Bool,
                    format!("or i1 {}, {}", permitted.text, valid.text),
                );
                selected = self.instruction(
                    Ty::Int(32, false),
                    format!(
                        "select i1 {}, i32 {index}, i32 {}",
                        valid.text, selected.text
                    ),
                );
            }
            self.guard(&permitted)?;
            let slot = self.slot(Ty::Int(32, false));
            self.prologue
                .push_str(&format!("  store i32 -1, ptr {}\n", slot.pointer));
            self.store(&slot, &selected)?;
            self.pattern_orders.insert((source.start, source.end), slot);
        }
        if !needed {
            return Ok(());
        }
        let mut transfers = BTreeMap::<LocalId, Vec<Vec<usize>>>::new();
        for transfer in &schedule.pattern_transfers {
            if transfer.source == source {
                transfers
                    .entry(transfer.binding)
                    .or_default()
                    .push(transfer.fields.clone());
            }
        }
        for (binding, paths) in &transfers {
            let (actual, _) = receipt
                .get(&binding)
                .ok_or("host residual binding receipt missing")?;
            let mut valid = Value {
                ty: Ty::Bool,
                text: "false".into(),
            };
            for path in paths {
                let code = self.receipt_path(&path)?;
                let equal =
                    self.instruction(Ty::Bool, format!("icmp eq i32 {}, {code}", actual.text));
                valid = self.instruction(Ty::Bool, format!("or i1 {}, {}", valid.text, equal.text));
            }
            self.guard(&valid)?;
        }
        let key = (source.start, source.end);
        let mut fields = vec![];
        super::cleanup::product_paths(&value.ty, &mut vec![], &mut fields);
        for path in fields {
            let mut constructed = Value {
                ty: Ty::Bool,
                text: "true".into(),
            };
            for (binding, moved) in &transfers {
                let (actual, _) = receipt
                    .get(binding)
                    .ok_or("host residual binding receipt missing")?;
                for moved in moved {
                    if !path.starts_with(moved) {
                        continue;
                    }
                    let code = self.receipt_path(moved)?;
                    let remains =
                        self.instruction(Ty::Bool, format!("icmp ne i32 {}, {code}", actual.text));
                    constructed = self.instruction(
                        Ty::Bool,
                        format!("and i1 {}, {}", constructed.text, remains.text),
                    );
                }
            }
            let flag = self.slot(Ty::Bool);
            self.prologue
                .push_str(&format!("  store i1 false, ptr {}\n", flag.pointer));
            self.store(&flag, &constructed)?;
            self.temporary_fields.insert((key, path), flag);
        }
        let slot = self.slot(value.ty.clone());
        self.store(&slot, value)?;
        self.temporaries.insert(key, slot);
        let flag = self.slot(Ty::Bool);
        self.prologue
            .push_str(&format!("  store i1 false, ptr {}\n", flag.pointer));
        self.store(
            &flag,
            &Value {
                ty: Ty::Bool,
                text: "true".into(),
            },
        )?;
        self.temporary_live.insert(key, flag);
        Ok(())
    }
}
fn child_path(path: &Option<Vec<usize>>, index: usize) -> Option<Vec<usize>> {
    path.as_ref().map(|path| {
        let mut path = path.clone();
        path.push(index);
        path
    })
}
impl Emitter<'_> {
    fn receipt_path(&mut self, path: &[usize]) -> Result<u32, String> {
        if let Some(index) = self.receipt_paths.iter().position(|item| item == path) {
            return u32::try_from(index).map_err(|_| "host receipt path limit exceeded".into());
        }
        let index = u32::try_from(self.receipt_paths.len())
            .map_err(|_| "host receipt path limit exceeded")?;
        if index == u32::MAX {
            return Err("host receipt path limit exceeded".into());
        }
        self.receipt_paths.push(path.to_vec());
        Ok(index)
    }
}
