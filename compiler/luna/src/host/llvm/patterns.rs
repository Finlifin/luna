use super::*;
use nir::program::{MatchArm, Pattern};

pub(super) type Receipt = BTreeMap<LocalId, (Value, Value)>;

struct PendingThen {
    source: nir::program::SourceRange,
    fail_slot: Slot,
    fail_flag: Slot,
    receipt_slots: BTreeMap<LocalId, (Slot, Slot)>,
    conditional: bool,
}

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
        pending_then: &mut Vec<PendingThen>,
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
                let field_count = match &value.ty {
                    Ty::Record(layout) | Ty::PackedRecord(layout) => layout.len(),
                    Ty::AlignedRecord(layout) => layout.fields.len(),
                    Ty::BitPackedRecord(layout) => layout.fields.len(),
                    _ => return Err("host record pattern subject has no record layout".into()),
                };
                if fields.len() != field_count {
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
                        pending_then,
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
                        pending_then,
                    )?;
                }
            }
            Pattern::ErrorResult { label, inner } => {
                let index = self.error_index(&value.ty, label.as_ref())?;
                let tag = self.error_tag(value)?;
                let Ty::Int(tag_bits, false) = tag.ty else {
                    return Err("host error tag must be an unsigned integer".into());
                };
                let condition = self.instruction(
                    Ty::Bool,
                    format!("icmp eq i{tag_bits} {}, {index}", tag.text),
                );
                self.accept(condition, failure)?;
                let payload = self.error_payload(value, index)?;
                self.pattern(
                    inner,
                    &payload,
                    failure,
                    depth,
                    child_path(&path, 0),
                    receipt,
                    pending_then,
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
                        pending_then,
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
                    self.float_compare("oeq", value, &other)
                } else {
                    self.compare("eq", value.clone(), other)?
                };
                self.accept(equal, failure)?;
            }
            Pattern::Binding { binding, inner } => {
                self.pattern(
                    inner,
                    value,
                    failure,
                    depth,
                    path.clone(),
                    receipt,
                    pending_then,
                )?;
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
                self.pattern(
                    inner,
                    value,
                    failure,
                    depth,
                    path.clone(),
                    receipt,
                    pending_then,
                )?;
                self.locals.remove(binding);
                if let Some(previous) = previous {
                    self.locals.insert(*binding, previous);
                }
            }
            Pattern::Guard { pattern, condition } => {
                self.pattern(
                    pattern,
                    value,
                    failure,
                    depth,
                    path.clone(),
                    receipt,
                    pending_then,
                )?;
                let condition = self.expr(condition, depth)?;
                self.accept(condition, failure)?;
            }
            Pattern::Then {
                first,
                value: expression,
                next,
            } => {
                self.pattern(
                    first,
                    value,
                    failure,
                    depth,
                    path.clone(),
                    receipt,
                    pending_then,
                )?;
                if self.terminated {
                    return Ok(());
                }
                let next_value = self.expr(expression, depth)?;
                if self.terminated {
                    return Ok(());
                }
                let fail_slot = self.slot(next_value.ty.clone());
                self.store(&fail_slot, &next_value)?;
                let fail_flag = self.slot(Ty::Bool);
                self.prologue
                    .push_str(&format!("  store i1 false, ptr {}\n", fail_flag.pointer));
                self.store(
                    &fail_flag,
                    &Value {
                        ty: Ty::Bool,
                        text: "true".into(),
                    },
                )?;
                let pt_index = pending_then.len();
                pending_then.push(PendingThen {
                    source: expression.source,
                    fail_slot,
                    fail_flag,
                    receipt_slots: BTreeMap::new(),
                    conditional: false,
                });
                let mut then_receipt = Receipt::new();
                self.pattern(
                    next,
                    &next_value,
                    failure,
                    depth,
                    Some(vec![]),
                    &mut then_receipt,
                    pending_then,
                )?;
                if !self.terminated {
                    let base_rank = receipt.len();
                    let mut receipt_slots = BTreeMap::new();
                    for (id, (path_val, rank_val)) in then_receipt {
                        let path_slot = self.slot(Ty::Int(32, false));
                        self.prologue
                            .push_str(&format!("  store i32 -1, ptr {}\n", path_slot.pointer));
                        self.store(&path_slot, &path_val)?;
                        let rank_slot = self.slot(Ty::Int(32, false));
                        self.prologue
                            .push_str(&format!("  store i32 -1, ptr {}\n", rank_slot.pointer));
                        self.store(&rank_slot, &rank_val)?;
                        receipt_slots.insert(id, (path_slot, rank_slot));
                        let combined_rank = self.instruction(
                            Ty::Int(32, false),
                            format!("add i32 {base_rank}, {}", rank_val.text),
                        );
                        receipt.insert(
                            id,
                            (
                                Value {
                                    ty: Ty::Int(32, false),
                                    text: "-1".into(),
                                },
                                combined_rank,
                            ),
                        );
                    }
                    pending_then[pt_index].receipt_slots = receipt_slots;
                }
            }
            Pattern::Remap { pattern, bindings } => {
                let then_start = pending_then.len();
                self.pattern(
                    pattern,
                    value,
                    failure,
                    depth,
                    path.clone(),
                    receipt,
                    pending_then,
                )?;
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
                for pt in &mut pending_then[then_start..] {
                    let mapped_slots = bindings
                        .iter()
                        .filter_map(|(from, to)| {
                            pt.receipt_slots.remove(from).map(|slots| (*to, slots))
                        })
                        .collect::<Vec<_>>();
                    pt.receipt_slots.extend(mapped_slots);
                }
            }
            Pattern::Or(alternatives) => {
                if alternatives.is_empty() {
                    self.branch(failure);
                    return Ok(());
                }
                if matches!(value.ty, Ty::Enum(_))
                    && alternatives.iter().all(|alt| {
                        matches!(
                            alt,
                            Pattern::Variant { fields, .. }
                                if fields.iter().all(|f| matches!(f, Pattern::Wild))
                        )
                    })
                {
                    let indices: Vec<usize> = alternatives
                        .iter()
                        .map(|alt| match alt {
                            Pattern::Variant { index, .. } => *index,
                            _ => unreachable!(),
                        })
                        .collect();
                    if let Some(cond) = self.variant_set_condition(value, &indices)? {
                        self.accept(cond, failure)?;
                        return Ok(());
                    }
                }
                let outer = self.locals.clone();
                let original_receipt = receipt.clone();
                let mut receipt_slots: Option<BTreeMap<LocalId, (Slot, Slot)>> = None;
                let success = self.fresh();
                let mut merged: Option<BTreeMap<LocalId, Slot>> = None;
                let mut or_pending = Vec::new();
                for alternative in alternatives {
                    let next = self.fresh();
                    self.locals = outer.clone();
                    *receipt = original_receipt.clone();
                    let mut alt_pending = Vec::new();
                    self.pattern(
                        alternative,
                        value,
                        &next,
                        depth,
                        path.clone(),
                        receipt,
                        &mut alt_pending,
                    )?;
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
                        for pt in &mut alt_pending {
                            pt.conditional = true;
                        }
                        self.branch(&success);
                    }
                    self.label(&next);
                    self.emit_pending_then_failure(&alt_pending)?;
                    or_pending.extend(alt_pending);
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
                    pending_then.extend(or_pending);
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
        let mut pending_then = Vec::new();
        self.pattern(
            pattern,
            &value,
            &failure,
            depth,
            Some(vec![]),
            &mut receipt,
            &mut pending_then,
        )?;
        if let Some(guard) = guard {
            let condition = self.expr(guard, depth)?;
            self.accept(condition, &failure)?;
        }
        let continues = !self.terminated;
        let matched = self.locals.clone();
        if continues {
            self.commit_pending_then(&pending_then)?;
            self.retain_pattern(source, &value, &receipt)?;
            self.branch(&done);
        }
        self.label(&failure);
        self.locals = outer;
        self.emit_pending_then_failure(&pending_then)?;
        if let Some(otherwise) = otherwise {
            self.retain_pattern(expression.source, &value, &Receipt::new())?;
            self.expr(otherwise, depth)?;
            if !self.terminated {
                return Err("host let-else failure must diverge".into());
            }
        } else {
            self.emit_trap();
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
        let never = *ty == Term::Primitive(P::Never);
        let ty = if never {
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
                let arm_fail = self.fresh();
                let mut pending_then = Vec::new();
                let mut receipt = Receipt::new();
                self.pattern(
                    &arm.pattern,
                    &value,
                    &arm_fail,
                    depth,
                    Some(vec![]),
                    &mut receipt,
                    &mut pending_then,
                )?;
                if let Some(guard) = &arm.guard {
                    let condition = self.expr(guard, depth)?;
                    if !self.terminated {
                        self.accept(condition, &arm_fail)?;
                    }
                }
                if !self.terminated {
                    self.commit_pending_then(&pending_then)?;
                    self.retain_pattern(arm.body.source, &value, &receipt)?;
                }
                let mut arm_unconsumed_k = false;
                if let Some(cap) = self.capturing_resume.as_mut() {
                    let k_binding = effects::capturing_continuation_binding(&arm.pattern);
                    cap.k_slot = k_binding.and_then(|id| self.locals.get(&id).cloned());
                    let mentions_k = k_binding.is_some_and(|id| {
                        effects::arm_mentions_local(&arm.body, arm.guard.as_ref(), id)
                    });
                    let has_drop_site = self.cleanup.is_some_and(|schedule| {
                        schedule.sites.iter().any(|site| {
                            site.source == arm.body.source
                                && site.boundary == iir::cleanup::Boundary::HandlerEnd
                                && site.actions.iter().any(|action| match &action.target {
                                    iir::cleanup::Target::Local { binding, fields } => k_binding
                                        .is_some_and(|id| *binding == id && fields.is_empty()),
                                    iir::cleanup::Target::Temporary { source, .. } => {
                                        k_binding.is_none() && *source == arm.body.source
                                    }
                                    _ => false,
                                })
                        })
                    });
                    arm_unconsumed_k = !mentions_k && !has_drop_site;
                    if arm_unconsumed_k {
                        let slot = cap.unconsumed_guard_slot.clone();
                        self.line(format!("store ptr %k_val, ptr {slot}, align 8"));
                    }
                }
                let selected = self.expr_with_effects(&arm.body, depth, selected)?;
                if !self.terminated {
                    if handler.is_some() {
                        self.cleanup_boundary(arm.body.source, iir::cleanup::Boundary::HandlerEnd)?;
                    } else if !arm.body.opens_block_scope() {
                        self.cleanup_boundary(arm.body.source, iir::cleanup::Boundary::ScopeEnd)?;
                    }
                    if arm_unconsumed_k
                        && let Some(guard_slot) = self
                            .capturing_resume
                            .as_ref()
                            .map(|cap| cap.unconsumed_guard_slot.clone())
                    {
                        let pending_k = self.instruction(
                            Ty::Pointer,
                            format!("load ptr, ptr {guard_slot}, align 8"),
                        );
                        self.line(format!("store ptr null, ptr {guard_slot}, align 8"));
                        self.line(format!(
                            "call void @flurry_cont_abort(ptr {}, ptr %task)",
                            pending_k.text
                        ));
                    }
                    // Checked ordinary matches retain even the impossible
                    // arms. A Never match has no normal edge or result slot
                    // to fill from one of those arms.
                    if never && handler.is_none() {
                        self.line("unreachable");
                        self.terminated = true;
                    } else {
                        self.store(&result, &selected)?;
                        self.branch(&end);
                        continues = true;
                    }
                }
                self.label(&arm_fail);
                self.emit_pending_then_failure(&pending_then)?;
                self.branch(&next);
                Ok::<(), String>(())
            })();
            if let Some(suspended) = suspended {
                self.restore_local_frame(suspended);
            }
            outcome?;
            self.label(&next);
        }
        if let Some(fallback) = fallback {
            if let Some(cap) = self.capturing_resume.clone() {
                if fallback.handler.abi != nir::effects::Abi::Capturing {
                    return Err("host capturing fallback requires a capturing outer handler".into());
                }
                let (callee, env) = fallback
                    .machine
                    .clone()
                    .ok_or("host capturing fallback target missing")?;
                let guard_slot = cap.unconsumed_guard_slot.clone();
                self.line(format!("store ptr %k_val, ptr {guard_slot}, align 8"));
                let (y_ptr, y_bytes) = if cap.y_ty == Ty::Unit {
                    ("null".to_owned(), "0".to_owned())
                } else {
                    let sz = self.instruction(
                        Ty::Int(64, false),
                        format!(
                            "ptrtoint ptr getelementptr ({}, ptr null, i32 1) to i64",
                            cap.y_ty.llvm()
                        ),
                    );
                    (cap.y_slot.clone(), sz.text)
                };
                self.emitted_effects.has_external_or_indirect_call = true;
                self.emitted_effects.has_trap_or_unwind = true;
                let aborting = self.instruction(
                    Ty::Int(8, false),
                    format!(
                        "call i8 @flurry_cont_suspend(ptr {callee}, ptr {env}, ptr %args_ptr, ptr {y_ptr}, i64 {y_bytes})"
                    ),
                );
                let ok = self.instruction(Ty::Bool, format!("icmp eq i8 {}, 0", aborting.text));
                let resumed_lbl = self.fresh();
                let abort_lbl = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{resumed_lbl}, label %{abort_lbl}, {}",
                    ok.text,
                    control::LIKELY
                ));
                self.label(&abort_lbl);
                let source = handler
                    .as_ref()
                    .map(|h| h.request)
                    .unwrap_or(nir::program::SourceRange { start: 0, end: 0 });
                self.cleanup_boundary(source, iir::cleanup::Boundary::PredicateFailure)?;
                self.unwind_resources()?;
                self.control_return(control::ABORT);
                self.label(&resumed_lbl);
                self.line(format!("store ptr null, ptr {guard_slot}, align 8"));
                self.line(format!("store i8 1, ptr {}, align 1", cap.resumed_slot));
                self.branch(&cap.exit_label);
            } else {
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
            }
        } else {
            // A total match must never deliver an uninitialized result.
            self.emit_trap();
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

    pub(super) fn retain_pattern(
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

    fn emit_pending_then_failure(&mut self, pending: &[PendingThen]) -> Result<(), String> {
        for pt in pending.iter().rev() {
            let needs_failure = self.cleanup.is_some_and(|schedule| {
                schedule.sites.iter().any(|site| {
                    site.source == pt.source
                        && site.boundary == iir::cleanup::Boundary::PatternFailure
                        && site
                            .actions
                            .iter()
                            .any(|action| super::cleanup::has_invoke(&action.program.steps))
                })
            });
            if !needs_failure {
                continue;
            }
            let active = self.load(&pt.fail_flag);
            let drop_bb = self.fresh();
            let skip_bb = self.fresh();
            self.line(format!(
                "br i1 {}, label %{drop_bb}, label %{skip_bb}",
                active.text
            ));
            self.label(&drop_bb);
            let key = (pt.source.start, pt.source.end);
            let previous_temp = self.temporaries.insert(key, pt.fail_slot.clone());
            let previous_live = self.temporary_live.insert(key, pt.fail_flag.clone());
            let field_keys: Vec<_> = self
                .temporary_fields
                .keys()
                .filter(|(k, _)| *k == key)
                .cloned()
                .collect();
            let mut previous_fields = Vec::with_capacity(field_keys.len());
            for field_key in field_keys {
                if let Some(slot) = self.temporary_fields.remove(&field_key) {
                    previous_fields.push((field_key, slot));
                }
            }
            let cleaned = self.cleanup_boundary(pt.source, iir::cleanup::Boundary::PatternFailure);
            match previous_temp {
                Some(slot) => {
                    self.temporaries.insert(key, slot);
                }
                None => {
                    self.temporaries.remove(&key);
                }
            }
            match previous_live {
                Some(slot) => {
                    self.temporary_live.insert(key, slot);
                }
                None => {
                    self.temporary_live.remove(&key);
                }
            }
            for (field_key, slot) in previous_fields {
                self.temporary_fields.insert(field_key, slot);
            }
            cleaned?;
            if !self.terminated {
                self.store(
                    &pt.fail_flag,
                    &Value {
                        ty: Ty::Bool,
                        text: "false".into(),
                    },
                )?;
                self.branch(&skip_bb);
            }
            self.label(&skip_bb);
        }
        Ok(())
    }

    fn commit_pending_then(&mut self, pending: &[PendingThen]) -> Result<(), String> {
        for pt in pending {
            if !pt.conditional {
                self.store(
                    &pt.fail_flag,
                    &Value {
                        ty: Ty::Bool,
                        text: "false".into(),
                    },
                )?;
                let val = self.load(&pt.fail_slot);
                let mut pt_receipt = Receipt::new();
                for (id, (path_slot, rank_slot)) in &pt.receipt_slots {
                    pt_receipt.insert(*id, (self.load(path_slot), self.load(rank_slot)));
                }
                self.retain_pattern(pt.source, &val, &pt_receipt)?;
                continue;
            }
            let needed = self.cleanup.is_some_and(|schedule| {
                schedule
                    .pattern_orders
                    .iter()
                    .any(|order| order.source == pt.source)
                    || schedule
                        .sites
                        .iter()
                        .flat_map(|site| &site.actions)
                        .any(|action| {
                            matches!(
                                action.target,
                                iir::cleanup::Target::Temporary { source: origin, .. }
                                if origin == pt.source
                            ) && super::cleanup::has_invoke(&action.program.steps)
                        })
            });
            if !needed {
                self.store(
                    &pt.fail_flag,
                    &Value {
                        ty: Ty::Bool,
                        text: "false".into(),
                    },
                )?;
                continue;
            }
            let active = self.load(&pt.fail_flag);
            let yes_bb = self.fresh();
            let done_bb = self.fresh();
            self.line(format!(
                "br i1 {}, label %{yes_bb}, label %{done_bb}",
                active.text
            ));
            self.label(&yes_bb);
            self.store(
                &pt.fail_flag,
                &Value {
                    ty: Ty::Bool,
                    text: "false".into(),
                },
            )?;
            let val = self.load(&pt.fail_slot);
            let mut pt_receipt = Receipt::new();
            for (id, (path_slot, rank_slot)) in &pt.receipt_slots {
                pt_receipt.insert(*id, (self.load(path_slot), self.load(rank_slot)));
            }
            self.retain_pattern(pt.source, &val, &pt_receipt)?;
            if !self.terminated {
                self.branch(&done_bb);
            }
            self.label(&done_bb);
        }
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
    pub(super) fn receipt_path(&mut self, path: &[usize]) -> Result<u32, String> {
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
