use super::*;
use std::collections::BTreeMap;

fn vtable_symbol(index: usize) -> String {
    format!("@flurry_dyn_vtable_{index}")
}

fn drop_symbol(index: usize) -> String {
    format!("@flurry_dyn_drop_{index}")
}

fn thunk_symbol(vt_idx: usize, slot: usize) -> String {
    format!("@flurry_dyn_thunk_{vt_idx}_{slot}")
}

fn dyn_allocator_helpers_needed(graph: &iir::Package) -> bool {
    graph
        .dyn_vtables
        .iter()
        .any(|v| !v.by_pointer || v.methods.iter().any(|m| m.returns_self))
}

fn dyn_env_record_ty(
    graph: &iir::Package,
    env_identities: &[Term],
    checked: bool,
) -> Result<Option<(Ty, Vec<Ty>)>, String> {
    if env_identities.is_empty() {
        return Ok(None);
    }
    let mut fields = Vec::with_capacity(env_identities.len());
    for env_term in env_identities {
        let Term::Runtime(id) = env_term else {
            return Err("host dyn trait object env identity is not a runtime id".into());
        };
        let raw_domain = graph
            .runtime_domains
            .get(id)
            .ok_or_else(|| format!("host dyn trait object missing runtime env domain for {id}"))?;
        let term = match raw_domain {
            Term::Lift(inner) => inner.as_ref(),
            other => other,
        };
        fields.push(repr(graph, term, checked)?);
    }
    let record_ty = Ty::Record(std::sync::Arc::new(fields.clone()));
    Ok(Some((record_ty, fields)))
}

fn emit_synthetic_function(
    functions: &mut String,
    symbol: &str,
    result: &Ty,
    checked: bool,
    inputs: Vec<String>,
    e: Emitter<'_>,
) {
    let result_llvm = if checked {
        control::packet(result).llvm()
    } else {
        result.llvm()
    };
    let mut all_inputs = inputs;
    if checked {
        all_inputs.push("ptr %task".into());
    }
    let name = symbol.strip_prefix('@').unwrap_or(symbol);
    functions.push_str(&format!(
        "define internal {result_llvm} @{name}({}) {{\nentry:\n{}{}}}\n\n",
        all_inputs.join(", "),
        e.prologue,
        e.body
    ));
}

fn make_emitter<'a>(
    graph: &'a iir::Package,
    schedules: Option<&'a iir::cleanup::Schedule>,
    checked: bool,
    failure_result: Option<Ty>,
    lambdas: &'a BTreeMap<usize, usize>,
    template_identities: &'a callables::TemplateIdentities<'a>,
    declarations: &'a mut BTreeSet<String>,
    text_constants: &'a mut strings::TextConstants,
    attr_table: &'a mut attributes::AttributeTable,
    bitvec_cache: &'a mut bitvec_opt::SynthesisCache,
    fuel: &'a mut usize,
) -> Emitter<'a> {
    Emitter {
        graph,
        cleanup: None,
        schedules,
        resource_unwinds: vec![],
        checked,
        function: None,
        closure_body: None,
        failure_result,
        effects: vec![],
        effect_resume: None,
        native_return: None,
        loop_destinations: Default::default(),
        outlined_return: None,
        outlined_loops: BTreeMap::new(),
        lambdas,
        template_identities,
        locals: BTreeMap::new(),
        live: BTreeMap::new(),
        temporaries: BTreeMap::new(),
        temporary_live: BTreeMap::new(),
        prepared: BTreeMap::new(),
        receipt_paths: vec![],
        pattern_orders: BTreeMap::new(),
        temporary_fields: BTreeMap::new(),
        loops: BTreeMap::new(),
        prologue: String::new(),
        body: String::new(),
        declarations,
        text_constants,
        attr_table: Some(attr_table),
        bitvec_cache: Some(bitvec_cache),
        serial: 0,
        terminated: false,
        fuel,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_dyn_vtables(
    graph: &iir::Package,
    dyn_cleanups: &[crate::cleanup::DynCleanup],
    checked: bool,
    functions: &mut String,
    declarations: &mut BTreeSet<String>,
    lambdas: &BTreeMap<usize, usize>,
    template_identities: &callables::TemplateIdentities<'_>,
    text_constants: &mut strings::TextConstants,
    attr_table: &mut attributes::AttributeTable,
    bitvec_cache: &mut bitvec_opt::SynthesisCache,
    schedules: Option<&iir::cleanup::Schedule>,
    fuel: &mut usize,
) -> Result<(), String> {
    if graph.dyn_vtables.is_empty() {
        return Ok(());
    }
    let pointer_bits = graph.target.map_or(64, |t| t.pointer_bits());
    let ptr_bytes = u64::from(pointer_bits / 8).max(8);
    if dyn_allocator_helpers_needed(graph) {
        declarations.insert(include_str!("platform.ll").into());
    }

    // Emit one drop helper per unique owned subject type.
    let mut drop_subjects: Vec<Term> = Vec::new();
    for vtable in &graph.dyn_vtables {
        if !vtable.by_pointer && !drop_subjects.contains(&vtable.subject) {
            drop_subjects.push(vtable.subject.clone());
        }
    }
    for (drop_idx, subject) in drop_subjects.iter().enumerate() {
        let subject_ty = repr(graph, subject, checked)?;
        let layout = subject_ty.storage_layout_at(pointer_bits, 0, fuel)?;
        let align = layout.align.max(1);
        let cleanup_program = dyn_cleanups
            .iter()
            .find(|dc| dc.subject == *subject)
            .map(|dc| &dc.program);
        let mut e = make_emitter(
            graph,
            schedules,
            checked,
            checked.then_some(Ty::Unit),
            lambdas,
            template_identities,
            declarations,
            text_constants,
            attr_table,
            bitvec_cache,
            fuel,
        );
        if let Some(program) = cleanup_program
            && !program.steps.is_empty()
        {
            let receiver_slot = if layout.size == 0 {
                e.slot(subject_ty.clone())
            } else {
                Slot::new(subject_ty.clone(), "%data_ptr".into(), Some(align))
            };
            e.cleanup_subject_steps(&receiver_slot, &program.steps, Some(subject))?;
        }
        if !e.terminated && layout.size > 0 {
            let addr = e.instruction(Ty::Int(64, false), "ptrtoint ptr %data_ptr to i64".into());
            let eff_align = layout.align.max(ptr_bytes).next_power_of_two();
            e.line(format!(
                "call void @flurry_linux_dealloc(i64 {}, i64 {}, i64 {eff_align})",
                addr.text, layout.size
            ));
        }
        if !e.terminated {
            e.ret(&Value::unit(), None)?;
        }
        emit_synthetic_function(
            functions,
            &drop_symbol(drop_idx),
            &Ty::Unit,
            checked,
            vec!["ptr %data_ptr".into()],
            e,
        );
    }

    // Emit method thunks and vtable globals.
    for (vt_idx, vtable) in graph.dyn_vtables.iter().enumerate() {
        let mut vtable_entries = Vec::with_capacity(1 + vtable.methods.len());
        if vtable.by_pointer {
            vtable_entries.push("ptr null".to_string());
        } else {
            let drop_idx = drop_subjects
                .iter()
                .position(|s| s == &vtable.subject)
                .ok_or("host missing drop helper for owned dyn trait object")?;
            vtable_entries.push(format!("ptr {}", drop_symbol(drop_idx)));
        }

        let owned_vtable_idx = if vtable.methods.iter().any(|m| m.returns_self) {
            Some(
                graph
                    .dyn_vtables
                    .iter()
                    .position(|candidate| {
                        candidate.interface == vtable.interface
                            && candidate.subject == vtable.subject
                            && !candidate.by_pointer
                            && candidate.env_identities == vtable.env_identities
                    })
                    .ok_or("host missing owned vtable for Self-returning dyn trait method")?,
            )
        } else {
            None
        };

        let env_layout = dyn_env_record_ty(graph, &vtable.env_identities, checked)?;
        let subject_ty = repr(graph, &vtable.subject, checked)?;
        let subject_layout = subject_ty.storage_layout_at(pointer_bits, 0, fuel)?;

        for (slot, method) in vtable.methods.iter().enumerate() {
            let (callee_idx, callee_instance) = graph
                .functions
                .iter()
                .enumerate()
                .find(|(_, f)| {
                    f.key.declaration == method.body
                        && f.key.arguments.as_ref() == method.arguments.as_slice()
                })
                .ok_or_else(|| {
                    format!(
                        "host dyn trait method body is absent from graph: {}",
                        method.body
                    )
                })?;
            let f = &callee_instance.function;
            let Term::Function { parameters, result } = &f.signature else {
                return Err("host dyn trait method signature is not concrete".into());
            };
            let (effect_row, logical_result) = effects::row_result(result);
            if !effect_row.is_empty() {
                return Err("host dyn trait dispatch does not support effect rows".into());
            }
            let callee_ret_ty = return_repr(graph, result, checked)?;
            let thunk_ret_ty = if method.returns_self {
                Ty::TraitObject
            } else {
                callee_ret_ty.clone()
            };

            let mut thunk_inputs = vec!["ptr %data_ptr".to_string(), "ptr %env_ptr".to_string()];
            let mut explicit_arg_vals = Vec::new();
            for (idx, param) in parameters.iter().skip(1).enumerate() {
                let p_ty = repr(graph, &param.ty, checked)?;
                if p_ty != Ty::Unit {
                    let arg_name = format!("%arg{idx}");
                    thunk_inputs.push(format!("{} {arg_name}", p_ty.llvm()));
                    explicit_arg_vals.push((p_ty, arg_name));
                }
            }

            let mut e = make_emitter(
                graph,
                schedules,
                checked,
                checked.then(|| thunk_ret_ty.clone()),
                lambdas,
                template_identities,
                declarations,
                text_constants,
                attr_table,
                bitvec_cache,
                fuel,
            );

            let mut call_args = Vec::new();
            if method.by_reference_receiver {
                call_args.push("ptr %data_ptr".into());
            } else if subject_ty != Ty::Unit {
                let self_val = e.load(&Slot::new(subject_ty.clone(), "%data_ptr".into(), None));
                call_args.push(format!("{} {}", subject_ty.llvm(), self_val.text));
                if !vtable.by_pointer && subject_layout.size > 0 {
                    let addr =
                        e.instruction(Ty::Int(64, false), "ptrtoint ptr %data_ptr to i64".into());
                    let eff_align = subject_layout.align.max(ptr_bytes).next_power_of_two();
                    e.line(format!(
                        "call void @flurry_linux_dealloc(i64 {}, i64 {}, i64 {eff_align})",
                        addr.text, subject_layout.size
                    ));
                }
            }
            for (p_ty, arg_name) in explicit_arg_vals {
                call_args.push(format!("{} {arg_name}", p_ty.llvm()));
            }
            for env_param in &f.env_parameters {
                let env_ty = repr(graph, &env_param.ty, checked)?;
                if env_ty == Ty::Unit {
                    continue;
                }
                let Some(runtime_id) = env_param.runtime_id else {
                    return Err("host dyn trait thunk env parameter lacks runtime id".into());
                };
                if let Some(env_slot) = vtable
                    .env_identities
                    .iter()
                    .position(|id| matches!(id, Term::Runtime(rid) if *rid == runtime_id))
                {
                    let (record_ty, _) = env_layout
                        .as_ref()
                        .ok_or("host dyn thunk missing environment record layout")?;
                    let field_ptr = e.instruction(
                        Ty::Pointer,
                        format!(
                            "getelementptr {}, ptr %env_ptr, i32 0, i32 {env_slot}",
                            record_ty.llvm()
                        ),
                    );
                    let loaded = e.load(&Slot::new(env_ty.clone(), field_ptr.text, None));
                    call_args.push(format!("{} {}", env_ty.llvm(), loaded.text));
                } else if let Some(delivery) = f
                    .runtime_deliveries
                    .get(&runtime_id)
                    .or_else(|| graph.global_runtime_deliveries.get(&runtime_id))
                {
                    let val = e.expr(delivery, 0)?;
                    call_args.push(format!("{} {}", val.ty.llvm(), val.text));
                } else {
                    return Err(format!(
                        "host dyn trait thunk missing runtime environment delivery for {runtime_id}"
                    ));
                }
            }

            let target_sym = format!("@flurry_{callee_idx}");
            let (call_val, ctrl) = e.machine_call(&target_sym, &call_args, callee_ret_ty, "");
            if let Some(ctrl) = ctrl {
                e.accept_control(&ctrl, nir::program::SourceRange { start: 0, end: 0 }, false)?;
            }
            if !e.terminated {
                if method.returns_self {
                    let box_ptr = e.dyn_box_value(&call_val)?;
                    let owned_vt =
                        vtable_symbol(owned_vtable_idx.ok_or(
                            "host missing owned vtable for Self-returning dyn trait method",
                        )?);
                    let p0 = e.instruction(
                        Ty::TraitObject,
                        format!("insertvalue {{ ptr, ptr }} undef, ptr {box_ptr}, 0"),
                    );
                    let fat_ptr = e.instruction(
                        Ty::TraitObject,
                        format!("insertvalue {{ ptr, ptr }} {}, ptr {owned_vt}, 1", p0.text),
                    );
                    e.ret(&fat_ptr, None)?;
                } else if *logical_result == Term::Primitive(P::Never) {
                    e.line("unreachable");
                    e.terminated = true;
                } else {
                    e.ret(&call_val, None)?;
                }
            }

            let thunk_sym = thunk_symbol(vt_idx, slot);
            emit_synthetic_function(
                functions,
                &thunk_sym,
                &thunk_ret_ty,
                checked,
                thunk_inputs,
                e,
            );
            vtable_entries.push(format!("ptr {thunk_sym}"));
        }

        let entries_text = vtable_entries.join(", ");
        functions.push_str(&format!(
            "{} = internal constant [{} x ptr] [{}]\n\n",
            vtable_symbol(vt_idx),
            vtable_entries.len(),
            entries_text
        ));
    }
    Ok(())
}

impl Emitter<'_> {
    fn dyn_box_value(&mut self, value: &Value) -> Result<String, String> {
        if value.ty == Ty::Unit {
            return Ok("null".into());
        }
        let pointer_bits = self.graph.target.map_or(64, |t| t.pointer_bits());
        let ptr_bytes = u64::from(pointer_bits / 8).max(8);
        let layout = value.ty.storage_layout_at(pointer_bits, 0, self.fuel)?;
        if layout.size == 0 {
            return Ok("null".into());
        }
        let eff_align = layout.align.max(ptr_bytes).next_power_of_two();
        let addr = self.instruction(
            Ty::Int(64, false),
            format!(
                "call i64 @flurry_linux_alloc(i64 {}, i64 {eff_align})",
                layout.size
            ),
        );
        let ok = self.instruction(Ty::Bool, format!("icmp ne i64 {}, 0", addr.text));
        let ready = self.fresh();
        let oom = self.fresh();
        self.line(format!("br i1 {}, label %{ready}, label %{oom}", ok.text));
        self.label(&oom);
        self.line("call void @llvm.trap()");
        self.line("unreachable");
        self.label(&ready);
        let box_ptr = self.instruction(Ty::Pointer, format!("inttoptr i64 {} to ptr", addr.text));
        self.store(
            &Slot::new(
                value.ty.clone(),
                box_ptr.text.clone(),
                Some(layout.align.max(1)),
            ),
            value,
        )?;
        Ok(box_ptr.text)
    }

    fn wrap_trait_object_closure(
        &mut self,
        closure_ty: Ty,
        fat_ptr: &Value,
        env_record: &Value,
    ) -> Value {
        let Ty::TraitObjectClosure(ref fields) = closure_ty else {
            unreachable!();
        };
        let states = closures::state_count(fields);
        let v0 = self.instruction(
            closure_ty.clone(),
            format!(
                "insertvalue {} zeroinitializer, {{ ptr, ptr }} {}, 0",
                closure_ty.llvm(),
                fat_ptr.text
            ),
        );
        let v1 = self.instruction(
            closure_ty.clone(),
            format!(
                "insertvalue {} {}, {} {}, 1",
                closure_ty.llvm(),
                v0.text,
                env_record.ty.llvm(),
                env_record.text
            ),
        );
        self.instruction(
            closure_ty.clone(),
            format!(
                "insertvalue {} {}, [{states} x i1] [{}], 2",
                closure_ty.llvm(),
                v1.text,
                vec!["i1 true"; states].join(", ")
            ),
        )
    }

    pub(super) fn dyn_cast(
        &mut self,
        expr: &Expr,
        cast: &nir::program::DynCast,
        depth: usize,
    ) -> Result<Value, String> {
        let vt_idx = self
            .graph
            .dyn_vtables
            .iter()
            .position(|v| {
                v.interface == cast.interface
                    && v.subject == cast.subject
                    && v.by_pointer == cast.by_pointer
                    && v.env_identities == cast.env_identities
            })
            .ok_or("host dyn cast missing materialized vtable")?;
        let vt_sym = vtable_symbol(vt_idx);

        let value = self.expr(&cast.value, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let mut env_values = Vec::with_capacity(cast.env_arguments.len());
        for env_arg in &cast.env_arguments {
            let env_val = self.expr(env_arg, depth)?;
            if self.terminated {
                return Ok(Value::unit());
            }
            env_values.push(env_val);
        }

        let data_ptr = if cast.by_pointer {
            if value.ty != Ty::Pointer {
                return Err("host pointer dyn cast requires pointer operand".into());
            }
            value.text
        } else {
            self.dyn_box_value(&value)?
        };

        let p0 = self.instruction(
            Ty::TraitObject,
            format!("insertvalue {{ ptr, ptr }} undef, ptr {data_ptr}, 0"),
        );
        let fat_ptr = self.instruction(
            Ty::TraitObject,
            format!("insertvalue {{ ptr, ptr }} {}, ptr {vt_sym}, 1", p0.text),
        );

        let out_ty = repr(self.graph, &expr.ty, self.checked)?;
        if cast.env_arguments.is_empty() {
            if out_ty != Ty::TraitObject {
                return Err("host bare dyn cast representation mismatch".into());
            }
            Ok(fat_ptr)
        } else {
            let Ty::TraitObjectClosure(ref fields) = out_ty else {
                return Err("host env-bearing dyn cast requires closure representation".into());
            };
            let env_record_ty = Ty::Record(fields.clone());
            let mut record_val = Value {
                ty: env_record_ty.clone(),
                text: "zeroinitializer".into(),
            };
            for (idx, val) in env_values.iter().enumerate() {
                if val.ty == Ty::Unit {
                    continue;
                }
                record_val = self.instruction(
                    env_record_ty.clone(),
                    format!(
                        "insertvalue {} {}, {} {}, {idx}",
                        env_record_ty.llvm(),
                        record_val.text,
                        val.ty.llvm(),
                        val.text
                    ),
                );
            }
            Ok(self.wrap_trait_object_closure(out_ty, &fat_ptr, &record_val))
        }
    }

    pub(super) fn dyn_call(
        &mut self,
        expr: &Expr,
        call: &nir::program::DynCall,
        depth: usize,
    ) -> Result<Value, String> {
        let dyn_iface = self
            .graph
            .dyn_interfaces
            .iter()
            .find(|i| i.interface == call.interface)
            .ok_or("host dyn call missing dynamic interface metadata")?;
        let (slot_idx, dyn_method) = dyn_iface
            .methods
            .iter()
            .enumerate()
            .find(|(_, m)| m.member == call.member)
            .ok_or("host dyn call target method missing from dynamic interface")?;
        let parameters = &dyn_method.parameters;
        let result = &dyn_method.result;
        let (effect_row, logical_result) = effects::row_result(result);
        if !effect_row.is_empty() {
            return Err("host dyn call does not support effect rows".into());
        }

        let (fat_ptr, env_ptr, caller_env_record) = if call.by_value_receiver {
            let recv_val = self.expr(&call.receiver, depth)?;
            if self.terminated {
                return Ok(Value::unit());
            }
            match &recv_val.ty {
                Ty::TraitObject => (recv_val, "null".to_string(), None),
                Ty::TraitObjectClosure(fields) => {
                    let fat = self.instruction(
                        Ty::TraitObject,
                        format!("extractvalue {} {}, 0", recv_val.ty.llvm(), recv_val.text),
                    );
                    let env_record_ty = Ty::Record(fields.clone());
                    let env_val = self.instruction(
                        env_record_ty.clone(),
                        format!("extractvalue {} {}, 1", recv_val.ty.llvm(), recv_val.text),
                    );
                    let env_slot = self.slot(env_record_ty);
                    self.store(&env_slot, &env_val)?;
                    (fat, env_slot.pointer, Some(env_val))
                }
                _ => return Err("host dyn call receiver is not a trait object".into()),
            }
        } else {
            let recv_ptr = self.expr(&call.receiver, depth)?;
            if self.terminated {
                return Ok(Value::unit());
            }
            let Term::Pointer(pointee) = &call.receiver.ty else {
                return Err("host borrowed dyn call requires pointer receiver".into());
            };
            let pointee_ty = repr(self.graph, pointee, self.checked)?;
            match &pointee_ty {
                Ty::TraitObject => {
                    let fat = self.load(&Slot::new(Ty::TraitObject, recv_ptr.text, None));
                    (fat, "null".to_string(), None)
                }
                Ty::TraitObjectClosure(fields) => {
                    let fat_ptr_slot = self.instruction(
                        Ty::Pointer,
                        format!(
                            "getelementptr {}, ptr {}, i32 0, i32 0",
                            pointee_ty.llvm(),
                            recv_ptr.text
                        ),
                    );
                    let fat = self.load(&Slot::new(Ty::TraitObject, fat_ptr_slot.text, None));
                    let env_ptr_slot = self.instruction(
                        Ty::Pointer,
                        format!(
                            "getelementptr {}, ptr {}, i32 0, i32 1",
                            pointee_ty.llvm(),
                            recv_ptr.text
                        ),
                    );
                    let env_val = if dyn_method.returns_self {
                        let env_record_ty = Ty::Record(fields.clone());
                        Some(self.load(&Slot::new(env_record_ty, env_ptr_slot.text.clone(), None)))
                    } else {
                        None
                    };
                    (fat, env_ptr_slot.text, env_val)
                }
                _ => return Err("host borrowed dyn call pointee is not a trait object".into()),
            }
        };

        let explicit_count = parameters.len().saturating_sub(1);
        let mut arg_values: Vec<Option<Value>> = vec![None; explicit_count];
        for arg in &call.arguments {
            let val = self.expr(&arg.value, depth)?;
            if self.terminated {
                return Ok(Value::unit());
            }
            let slot = arg
                .parameter
                .checked_sub(1)
                .filter(|&idx| idx < explicit_count)
                .ok_or("host dyn call parameter index out of range")?;
            arg_values[slot] = Some(val);
        }

        let data_ptr = self.instruction(
            Ty::Pointer,
            format!("extractvalue {{ ptr, ptr }} {}, 0", fat_ptr.text),
        );
        let vtable_ptr = self.instruction(
            Ty::Pointer,
            format!("extractvalue {{ ptr, ptr }} {}, 1", fat_ptr.text),
        );
        let vtable_entry_ptr = self.instruction(
            Ty::Pointer,
            format!(
                "getelementptr ptr, ptr {}, i32 {}",
                vtable_ptr.text,
                1 + slot_idx
            ),
        );
        let fn_ptr = self.instruction(
            Ty::Pointer,
            format!("load ptr, ptr {}", vtable_entry_ptr.text),
        );

        let mut call_args = vec![format!("ptr {}", data_ptr.text), format!("ptr {env_ptr}")];
        for (val, param) in arg_values.into_iter().zip(parameters.iter().skip(1)) {
            let val = val.ok_or("host dyn call missing explicit argument")?;
            let expected_ty = repr(self.graph, &param.ty, self.checked)?;
            if val.ty != expected_ty {
                return Err("host dyn call argument representation mismatch".into());
            }
            if val.ty != Ty::Unit {
                call_args.push(format!("{} {}", val.ty.llvm(), val.text));
            }
        }

        let call_ret_ty = if dyn_method.returns_self {
            Ty::TraitObject
        } else {
            return_repr(self.graph, result, self.checked)?
        };
        let (call_val, ctrl) = self.machine_call(&fn_ptr.text, &call_args, call_ret_ty, "");
        if let Some(ctrl) = ctrl {
            self.accept_control(&ctrl, expr.source, false)?;
        }
        if *logical_result == Term::Primitive(P::Never) {
            if !self.terminated {
                self.line("unreachable");
                self.terminated = true;
            }
            return Ok(Value::unit());
        }
        if !dyn_method.returns_self {
            return Ok(call_val);
        }

        let expr_ty = return_repr(self.graph, &expr.ty, self.checked)?;
        match &expr_ty {
            Ty::TraitObject => Ok(call_val),
            Ty::TraitObjectClosure(_) => {
                let env_val = caller_env_record
                    .ok_or("host Self-returning dyn call missing runtime env record")?;
                Ok(self.wrap_trait_object_closure(expr_ty, &call_val, &env_val))
            }
            _ => Err("host Self-returning dyn call result is not a trait object".into()),
        }
    }
}
