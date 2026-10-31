//! Lexical dispatch and direct/direct-passing machine-call environments.
use super::*;
use nir::effects::Abi;
use std::sync::Arc;

pub(super) struct HandlerScope {
    pub(super) instance: iir::InstanceKey,
    pub(super) context: Vec<iir::cleanup::ContextKey>,
    pub(super) installation: nir::program::SourceRange,
    pub(super) handler: usize,
}

#[derive(Clone, Copy)]
pub(super) struct HandlerInvocation<'a> {
    pub(super) scope: Option<&'a HandlerScope>,
    pub(super) request: nir::program::SourceRange,
}

#[derive(Clone)]
pub(super) struct ReturnStorage {
    pub(super) token: String,
    pub(super) value: Slot,
}

pub(super) struct DirectHandler {
    return_storage: Option<ReturnStorage>,
    outer_loop_controls: BTreeMap<LocalId, (String, String)>,
    scope: Option<HandlerScope>,
    handler: nir::program::EffectHandler,
    provided: Option<Value>,
    machine: Option<(String, String)>,
    locals: BTreeMap<LocalId, Slot>,
    loops: BTreeMap<LocalId, frames::LoopTarget>,
    outer: Vec<Arc<DirectHandler>>,
}

impl Emitter<'_> {
    fn effect_environment(&mut self, row: &[nir::effects::Operation]) -> Result<(), String> {
        *self.fuel = self
            .fuel
            .checked_sub(row.len().saturating_add(self.effects.len()))
            .ok_or("host direct effect environment budget exhausted")?;
        if row.len() != self.effects.len()
            || row
                .iter()
                .zip(&self.effects)
                .any(|(op, entry)| *op != entry.handler.operation)
        {
            return Err("host direct effect environment does not match its declared row".into());
        }
        Ok(())
    }

    pub(super) fn effect(&mut self, expr: &Expr, depth: usize) -> Result<Value, String> {
        match &expr.kind {
            K::EffectResume(value) => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                let (slot, target, _) = self
                    .effect_resume
                    .clone()
                    .ok_or("host resume has no direct handler boundary")?;
                self.cleanup_boundary(expr.source, iir::cleanup::Boundary::EffectResume)?;
                self.store(&slot, &value)?;
                self.effect_resume.as_mut().unwrap().2 = true;
                self.branch(&target);
                Ok(Value::unit())
            }
            K::EffectComputation { effects, body } => {
                self.effect_environment(effects)?;
                self.expr(body, depth)
            }
            K::EffectHandle {
                computation,
                handlers,
                outer,
            } => {
                self.effect_environment(outer)?;
                *self.fuel = self
                    .fuel
                    .checked_sub(self.effects.len())
                    .ok_or("host direct effect installation budget exhausted")?;
                let previous = self.effects.clone();
                let mut installed = Vec::new();
                for (handler_index, handler) in handlers.iter().enumerate() {
                    if !matches!(handler.abi, Abi::Direct | Abi::DirectPassing) {
                        return Err(
                            "host lexical handlers require direct or direct-passing ABI".into()
                        );
                    }
                    if handler.abi == Abi::DirectPassing
                        && !iir::effects::data_passing_signature(
                            self.graph,
                            &handler.signature,
                            self.fuel,
                        )?
                    {
                        return Err("host direct-passing slots currently require a zero-argument pointer-free data signature".into());
                    }
                    if let Some(slot) = handler.fallback {
                        let target = previous
                            .get(slot)
                            .ok_or("host direct effect fallback slot missing")?;
                        if target.handler.operation != handler.operation
                            || target.handler.signature != handler.signature
                            || target.handler.abi != handler.abi
                        {
                            return Err(
                                "host direct effect fallback identity or ABI mismatch".into()
                            );
                        }
                    }
                    // Charge the copied branch graph. Return targets stay in
                    // this machine function; loop targets use lexical snapshots.
                    for part in handler
                        .arms
                        .iter()
                        .flat_map(|arm| arm.guard.iter().chain(std::iter::once(&arm.body)))
                    {
                        nir::visit::expression(part, self.fuel, |_, _| Ok(()))?;
                    }
                    *self.fuel = self
                        .fuel
                        .checked_sub(
                            self.locals
                                .len()
                                .saturating_add(self.loops.len())
                                .saturating_add(self.outlined_loops.len())
                                .saturating_add(previous.len())
                                .saturating_add(1),
                        )
                        .ok_or("host direct effect installation budget exhausted")?;
                    let mut entry = Arc::new(DirectHandler {
                        outer_loop_controls: self.outlined_loops.clone(),
                        return_storage: self
                            .outlined_return
                            .clone()
                            .or_else(|| self.native_return.clone()),
                        scope: self.cleanup.map(|scope| HandlerScope {
                            instance: scope.instance.clone(),
                            context: scope.context.clone(),
                            installation: expr.source,
                            handler: handler_index,
                        }),
                        handler: handler.clone(),
                        provided: None,
                        machine: None,
                        locals: self.locals.clone(),
                        loops: self.loops.clone(),
                        outer: previous.clone(),
                    });
                    if handler.abi == Abi::DirectPassing {
                        // Providers execute in the outer environment, before
                        // any part of the handled computation (even arguments).
                        let packet = Value {
                            ty: Ty::Record(Arc::new(vec![])),
                            text: "zeroinitializer".into(),
                        };
                        let value =
                            self.dispatch_direct(entry.clone(), packet, depth + 1, expr.source)?;
                        if self.terminated {
                            return Ok(Value::unit());
                        }
                        Arc::get_mut(&mut entry)
                            .ok_or("host provider activation escaped during installation")?
                            .provided = Some(value);
                    }
                    installed.push(entry);
                }
                installed.extend(previous.iter().cloned());
                self.expr_with_effects(computation, depth, Some(&installed))
            }
            K::EffectRun { computation, slots } => {
                let row = match &computation.ty {
                    Term::Computation { effects, .. } => effects.as_slice(),
                    _ if slots.is_empty() => &[],
                    _ => return Err("host effect routing requires a computation".into()),
                };
                if row.len() != slots.len() {
                    return Err("host effect routing arity mismatch".into());
                }
                *self.fuel = self
                    .fuel
                    .checked_sub(slots.len())
                    .ok_or("host direct effect routing budget exhausted")?;
                let mut selected = Vec::new();
                for (operation, slot) in row.iter().zip(slots) {
                    let entry = self.effects.get(*slot).ok_or("host effect route missing")?;
                    if entry.handler.operation != *operation {
                        return Err("host effect route identity mismatch".into());
                    }
                    selected.push(entry.clone());
                }
                if matches!(computation.ty, Term::Computation { .. }) {
                    self.expr_with_effects(computation, depth, Some(&selected))
                } else {
                    self.expr(computation, depth)
                }
            }
            K::EffectRequest {
                operation,
                abi,
                allowed_index,
                signature,
                arguments,
            } => {
                if !matches!(abi, Abi::Direct | Abi::DirectPassing) {
                    return Err("host effect request requires direct or direct-passing ABI".into());
                }
                let entry = self
                    .effects
                    .get(*allowed_index)
                    .cloned()
                    .ok_or("host effect request has no installed entry")?;
                if entry.handler.operation != *operation
                    || entry.handler.signature != **signature
                    || entry.handler.abi != *abi
                {
                    return Err("host effect request entry identity or signature mismatch".into());
                }
                let Term::Function { parameters, .. } = signature.as_ref() else {
                    return Err("host effect signature is not a function".into());
                };
                if parameters.len() != arguments.len() {
                    return Err("host effect argument arity mismatch".into());
                }
                let mut values = Vec::new();
                for (index, (argument, parameter)) in arguments.iter().zip(parameters).enumerate() {
                    let value = self.expr(argument, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    if parameter.sort != Sort::Runtime
                        || value.ty != repr(self.graph, &parameter.ty, self.checked)?
                    {
                        return Err("host effect argument representation mismatch".into());
                    }
                    self.retain_prepared_field(expr.source, index, argument.source, &value)?;
                    values.push(value);
                }
                let ty = Ty::Record(Arc::new(values.iter().map(|v| v.ty.clone()).collect()));
                let mut packet = Value {
                    ty: ty.clone(),
                    text: "zeroinitializer".into(),
                };
                for (index, value) in values.iter().enumerate() {
                    let (field, text) = if value.ty == Ty::Unit {
                        ("{}".into(), "zeroinitializer".into())
                    } else {
                        (value.ty.llvm(), value.text.clone())
                    };
                    packet = self.instruction(
                        ty.clone(),
                        format!(
                            "insertvalue {} {}, {field} {text}, {index}",
                            ty.llvm(),
                            packet.text
                        ),
                    );
                }
                self.finish_prepared_construction(expr.source)?;
                self.dispatch_direct(entry, packet, depth + 1, expr.source)
            }
            _ => unreachable!(),
        }
    }
}

impl Emitter<'_> {
    pub(super) fn dispatch_direct(
        &mut self,
        entry: Arc<DirectHandler>,
        packet: Value,
        depth: usize,
        request: nir::program::SourceRange,
    ) -> Result<Value, String> {
        if depth >= 128 {
            return Err("host direct effect dispatch depth exhausted".into());
        }
        *self.fuel = self
            .fuel
            .checked_sub(
                entry
                    .locals
                    .len()
                    .saturating_add(entry.loops.len())
                    .saturating_add(entry.outer.len())
                    .saturating_add(entry.outer_loop_controls.len())
                    .saturating_add(entry.loops.len())
                    .saturating_add(self.loop_destinations.len())
                    .saturating_add(1),
            )
            .ok_or("host direct effect dispatch budget exhausted")?;
        if let Some(value) = &entry.provided {
            return Ok(value.clone());
        }
        if let Some((callee, environment)) = &entry.machine {
            let Term::Function { parameters, result } = &entry.handler.signature else {
                return Err("host direct entry signature missing".into());
            };
            let mut arguments = vec![format!("ptr {environment}")];
            for (index, parameter) in parameters.iter().enumerate() {
                let ty = repr(self.graph, &parameter.ty, self.checked)?;
                if parameter.sort != Sort::Runtime {
                    return Err("host direct entry retains staged parameters".into());
                }
                if ty != Ty::Unit {
                    let value = self.instruction(
                        ty.clone(),
                        format!("extractvalue {} {}, {index}", packet.ty.llvm(), packet.text),
                    );
                    arguments.push(format!("{} {}", ty.llvm(), value.text));
                }
            }
            let ty = return_repr(self.graph, result, self.checked)?;
            let call = format!("call {} {callee}({})", ty.llvm(), arguments.join(", "));
            let value = if ty == Ty::Unit {
                self.line(call);
                Value::unit()
            } else {
                self.instruction(ty, call)
            };
            if self.failure_result.is_some() {
                let failed = self.instruction(Ty::Bool, "load i1, ptr @flurry_failed".into());
                let succeeded = self.instruction(Ty::Bool, format!("xor i1 {}, true", failed.text));
                self.guard_at(&succeeded, Some(request))?;
            }
            self.check_effect_control(request)?;
            if **result == Term::Primitive(P::Never) {
                self.line("unreachable");
                self.terminated = true;
            }
            return Ok(value);
        }
        let Term::Function { result, .. } = &entry.handler.signature else {
            return Err("host direct handler signature missing".into());
        };
        let fallback = entry
            .handler
            .fallback
            .map(|slot| {
                entry
                    .outer
                    .get(slot)
                    .cloned()
                    .ok_or_else(|| "host direct effect fallback slot missing".to_string())
            })
            .transpose()?;
        // Suspend requester storage before installing lexical captures. The
        // handler's installer may be a different scope from this requester.
        let suspended = entry.scope.is_some();
        if suspended {
            let state = self.take_local_frame();
            self.resource_unwinds.push(frames::UnwindFrame {
                state,
                source: request,
                boundary: iir::cleanup::Boundary::PredicateFailure,
            });
        }
        let caller = std::mem::replace(&mut self.locals, entry.locals.clone());
        let environment = std::mem::replace(&mut self.effects, entry.outer.clone());
        let loops = std::mem::replace(&mut self.loops, entry.loops.clone());
        let outcome = self.match_with_fallback(
            packet,
            &entry.handler.arms,
            result,
            depth,
            fallback,
            Some(HandlerInvocation {
                scope: entry.scope.as_ref(),
                request,
            }),
            None,
        );
        self.effects = environment;
        self.loops = loops;
        self.locals = caller;
        if suspended {
            let frame = self
                .resource_unwinds
                .pop()
                .ok_or("host effect requester frame missing")?;
            self.restore_local_frame(frame.state);
        }
        outcome
    }
}

#[cfg(test)]
#[path = "handler_frame_tests.rs"]
mod frame_tests;

pub(super) type Environment = [Arc<DirectHandler>];
pub(super) fn row_result(ty: &Term) -> (&[nir::effects::Operation], &Term) {
    match ty {
        Term::Computation { effects, result } => (effects, result),
        _ => (&[], ty),
    }
}
impl Emitter<'_> {
    pub(super) fn bind_effect_parameters(
        &mut self,
        contracts: &iir::effects::Contracts<'_>,
        row: &[nir::effects::Operation],
        inputs: &mut Vec<String>,
    ) -> Result<(), String> {
        if !contracts.belongs_to(self.graph) {
            return Err("host effect contracts belong to a different graph".into());
        }
        for (index, contract) in contracts.row(row, self.fuel)?.into_iter().enumerate() {
            let Term::Function { parameters, result } = &contract.signature else {
                return Err("host effect entry signature missing".into());
            };
            let (provided, machine) = match contract.abi {
                Abi::Direct => {
                    for parameter in parameters {
                        if parameter.sort != Sort::Runtime { return Err("host direct entry retains staged parameters".into()); }
                        repr(self.graph, &parameter.ty, self.checked)?;
                    }
                    return_repr(self.graph, result, self.checked)?;
                    let callee = format!("%effect{index}_entry");
                    let environment = format!("%effect{index}_env");
                    inputs.push(format!("ptr {callee}"));
                    inputs.push(format!("ptr {environment}"));
                    (None, Some((callee, environment)))
                }
                Abi::DirectPassing if iir::effects::data_passing_signature(self.graph, &contract.signature, self.fuel)? => {
                    let ty = repr(self.graph, result, self.checked)?;
                    let value = if ty == Ty::Unit { Value::unit() } else {
                        inputs.push(format!("{} %effect{index}", ty.llvm()));
                        Value { ty, text: format!("%effect{index}") }
                    };
                    (Some(value), None)
                }
                _ => return Err("host cross-function effect ABI requires direct entries or pointer-free direct passing".into()),
            };
            self.effects.push(Arc::new(DirectHandler {
                return_storage: None,
                outer_loop_controls: BTreeMap::new(),
                scope: None,
                handler: nir::program::EffectHandler {
                    operation: contract.operation.clone(),
                    abi: contract.abi,
                    signature: contract.signature.clone(),
                    arms: vec![],
                    fallback: None,
                },
                provided,
                machine,
                locals: BTreeMap::new(),
                loops: BTreeMap::new(),
                outer: vec![],
            }));
        }
        Ok(())
    }
    pub(super) fn effect_arguments(
        &mut self,
        row: &[nir::effects::Operation],
        selected: Option<&Environment>,
    ) -> Result<Vec<String>, String> {
        if row.is_empty() {
            return Ok(vec![]);
        }
        let entries = selected.unwrap_or(&self.effects).to_vec();
        *self.fuel = self
            .fuel
            .checked_sub(row.len())
            .ok_or("host effect call budget exhausted")?;
        if entries.len() != row.len() {
            return Err("host effect call row arity mismatch".into());
        }
        let mut inputs = Vec::new();
        for (operation, entry) in row.iter().zip(entries) {
            if operation != &entry.handler.operation {
                return Err("host effect call entry identity mismatch".into());
            }
            if entry.handler.abi == Abi::Direct {
                let (callee, environment) = self.outline_direct(&entry)?;
                inputs.push(format!("ptr {callee}"));
                inputs.push(format!("ptr {environment}"));
                continue;
            }
            if entry.handler.abi != Abi::DirectPassing
                || !iir::effects::data_passing_signature(
                    self.graph,
                    &entry.handler.signature,
                    self.fuel,
                )?
            {
                return Err(
                    "host cross-function effect ABI currently requires pointer-free direct passing"
                        .into(),
                );
            }
            let value = entry
                .provided
                .as_ref()
                .ok_or("host effect call provider value missing")?;
            let Term::Function { result, .. } = &entry.handler.signature else {
                unreachable!()
            };
            if value.ty != repr(self.graph, result, self.checked)? {
                return Err("host effect call provider representation mismatch".into());
            }
            if value.ty != Ty::Unit {
                inputs.push(format!("{} {}", value.ty.llvm(), value.text));
            }
        }
        Ok(inputs)
    }
}

impl Emitter<'_> {
    /// Outline a resumptive handler. Captured places are borrowed from its
    /// installer, never copied into new owned storage in the callback.
    fn outline_direct(&mut self, entry: &Arc<DirectHandler>) -> Result<(String, String), String> {
        self.outline_direct_at(entry, 0)
    }
    fn outline_direct_at(
        &mut self,
        entry: &Arc<DirectHandler>,
        depth: usize,
    ) -> Result<(String, String), String> {
        if depth >= 128 {
            return Err("host direct outlining depth exhausted".into());
        }
        if let Some(machine) = &entry.machine {
            return Ok(machine.clone());
        }
        *self.fuel = self
            .fuel
            .checked_sub(
                entry
                    .locals
                    .len()
                    .saturating_add(entry.outer.len())
                    .saturating_add(entry.outer_loop_controls.len())
                    .saturating_add(entry.loops.len())
                    .saturating_add(1),
            )
            .ok_or("host direct outlining budget exhausted")?;
        // Checked source acquisition proves every lexical capture Copy and
        // records it as persistent installer storage in each arm's schedule.
        // Pointer-bearing captures additionally have stable provenance checks.
        // This permits borrowing aggregate places, not moving affine owners.
        let checked_captures = self.checked && entry.scope.is_some();
        let mut captures = BTreeSet::new();
        let mut parts = Vec::new();
        for arm in &entry.handler.arms {
            parts.extend(nir::visit::pattern_observations(&arm.pattern, self.fuel)?);
            parts.extend(arm.guard.iter());
            parts.push(&arm.body);
        }
        // Loop identities are lexical, not LLVM labels. Loops declared inside
        // the outlined tree receive fresh labels in the child emitter; exits
        // targeting the installer's loops still need nonlocal propagation.
        let mut local_loops = BTreeSet::new();
        for part in &parts {
            nir::visit::expression_in_closure_environment(part, self.fuel, |node, _| {
                if let K::Loop { identity, .. } = node.kind {
                    local_loops.insert(identity);
                }
                Ok(())
            })?;
        }
        let mut remote_loops = BTreeMap::new();
        for part in parts {
            nir::visit::expression_in_closure_environment(part, self.fuel, |node, _| {
                match node.kind {
                    K::Return(_) if entry.return_storage.is_none() => {
                        return Err("host outlined direct return requires checked cleanup".into());
                    }
                    K::Break(id) | K::Continue(id) if !local_loops.contains(&id) => {
                        let controls = entry
                            .loops
                            .get(&id)
                            .and_then(|t| t.control.clone())
                            .or_else(|| entry.outer_loop_controls.get(&id).cloned())
                            .ok_or(
                                "host outlined direct loop exit requires checked target evidence",
                            )?;
                        remote_loops.insert(id, controls);
                    }
                    _ => (),
                }
                if let K::Closure {
                    captures: closure_captures,
                    ..
                } = &node.kind
                {
                    for capture in closure_captures {
                        if capture.value.is_none() && entry.locals.contains_key(&capture.binding) {
                            if !checked_captures
                                && !matches!(capture.ty, Term::Primitive(_) | Term::Pointer(_))
                            {
                                return Err("host outlined direct aggregate captures require checked resource evidence".into());
                            }
                            captures.insert(capture.binding);
                        }
                    }
                }
                let written = match &node.kind {
                    K::Assign { place, value } => Some((place, &value.ty)),
                    K::BitAssign {
                        place,
                        storage_type,
                        ..
                    }
                    | K::BitUpdate {
                        place,
                        storage_type,
                        ..
                    } => Some((place, storage_type)),
                    _ => None,
                };
                if let Some((place, ty)) = written {
                    if entry.locals.contains_key(&place.root) {
                        if !checked_captures
                            && (!place.fields.is_empty()
                                || !matches!(ty, Term::Primitive(_) | Term::Pointer(_)))
                        {
                            return Err("host outlined direct aggregate captures require checked resource evidence".into());
                        }
                        captures.insert(place.root);
                    }
                }
                if let K::Local(id) = node.kind {
                    if entry.locals.contains_key(&id) {
                        // A nominal wrapper can have scalar machine layout
                        // while retaining distinct ownership. Layout is not
                        // permission to copy or consume a captured owner.
                        if !checked_captures
                            && !matches!(node.ty, Term::Primitive(_) | Term::Pointer(_))
                        {
                            return Err("host outlined direct aggregate captures require checked resource evidence".into());
                        }
                        captures.insert(id);
                    }
                }
                Ok(())
            })?;
        }
        let captures = captures
            .into_iter()
            .map(|id| (id, entry.locals[&id].clone()))
            .collect::<Vec<_>>();
        for (_, slot) in &captures {
            if !checked_captures
                && !matches!(
                    slot.ty,
                    Ty::Unit | Ty::Bool | Ty::Int(..) | Ty::Float32 | Ty::Float64 | Ty::Pointer
                )
            {
                return Err(
                    "host outlined direct aggregate captures require checked resource evidence"
                        .into(),
                );
            }
        }
        let Term::Function { parameters, result } = &entry.handler.signature else {
            return Err("host direct entry signature missing".into());
        };
        let result_ty = return_repr(self.graph, result, self.checked)?;
        let pointer_ty = Ty::Pointer;
        let mut fields = captures
            .iter()
            .map(|(_, slot)| Value {
                ty: Ty::Pointer,
                text: slot.pointer.clone(),
            })
            .collect::<Vec<_>>();
        let mut outer_layout = Vec::new();
        for outer in &entry.outer {
            let offset = fields.len();
            if let Some(value) = &outer.provided {
                fields.push(value.clone());
                outer_layout.push((outer, offset, false));
            } else {
                let (callee, environment) = self.outline_direct_at(outer, depth + 1)?;
                fields.push(Value {
                    ty: pointer_ty.clone(),
                    text: callee,
                });
                fields.push(Value {
                    ty: pointer_ty.clone(),
                    text: environment,
                });
                outer_layout.push((outer, offset, true));
            }
        }
        let return_offset = entry.return_storage.as_ref().map(|storage| {
            let offset = fields.len();
            fields.push(Value {
                ty: pointer_ty.clone(),
                text: storage.token.clone(),
            });
            fields.push(Value {
                ty: pointer_ty.clone(),
                text: storage.value.pointer.clone(),
            });
            offset
        });
        let loop_offset = fields.len();
        for (exit, again) in remote_loops.values() {
            fields.push(Value {
                ty: pointer_ty.clone(),
                text: exit.clone(),
            });
            fields.push(Value {
                ty: pointer_ty.clone(),
                text: again.clone(),
            });
        }
        let env_ty = Ty::Record(Arc::new(fields.iter().map(|v| v.ty.clone()).collect()));
        let env = self.slot(env_ty.clone());
        for (index, value) in fields.iter().enumerate() {
            if value.ty == Ty::Unit {
                continue;
            }
            let address = self.instruction(
                pointer_ty.clone(),
                format!(
                    "getelementptr {}, ptr {}, i32 0, i32 {index}",
                    env_ty.llvm(),
                    env.pointer
                ),
            );
            self.store(
                &Slot {
                    ty: value.ty.clone(),
                    pointer: address.text,
                },
                value,
            )?;
        }
        // Reserve a module-unique symbol before recursively emitting calls.
        let symbol = format!("flurry_handler_{}", self.declarations.len());
        self.declarations.insert(format!("; reserved {symbol}"));
        let mut child = Emitter {
            graph: self.graph,
            cleanup: None,
            schedules: self.schedules,
            resource_unwinds: vec![],
            checked: self.checked,
            function: None,
            closure_body: None,
            failure_result: self.checked.then(|| result_ty.clone()),
            effects: vec![],
            effect_resume: None,
            native_return: None,
            // Remote targets travel as tokens in outlined_loops. Only loops
            // created by this outlined tree need fresh local destinations.
            loop_destinations: local_loops
                .intersection(&self.loop_destinations)
                .copied()
                .collect(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas: self.lambdas,
            template_identities: self.template_identities,
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
            declarations: self.declarations,
            text_constants: self.text_constants,
            serial: 0,
            terminated: false,
            fuel: self.fuel,
        };
        for (index, (id, slot)) in captures.iter().enumerate() {
            let address = child.instruction(
                Ty::Pointer,
                format!(
                    "getelementptr {}, ptr %environment, i32 0, i32 {index}",
                    env_ty.llvm()
                ),
            );
            let pointer = child.instruction(Ty::Pointer, format!("load ptr, ptr {}", address.text));
            child.locals.insert(
                *id,
                Slot {
                    ty: slot.ty.clone(),
                    pointer: pointer.text,
                },
            );
        }
        for (outer, offset, machine) in outer_layout {
            let mut load = |index: usize| -> Value {
                let ty = fields[index].ty.clone();
                let address = child.instruction(
                    pointer_ty.clone(),
                    format!(
                        "getelementptr {}, ptr %environment, i32 0, i32 {index}",
                        env_ty.llvm()
                    ),
                );
                child.load(&Slot {
                    ty,
                    pointer: address.text,
                })
            };
            let first = load(offset);
            let (provided, machine) = if machine {
                (None, Some((first.text, load(offset + 1).text)))
            } else {
                (Some(first), None)
            };
            child.effects.push(Arc::new(DirectHandler {
                return_storage: None,
                outer_loop_controls: BTreeMap::new(),
                scope: None,
                handler: outer.handler.clone(),
                provided,
                machine,
                locals: BTreeMap::new(),
                loops: BTreeMap::new(),
                outer: vec![],
            }));
        }
        if let (Some(offset), Some(storage)) = (return_offset, &entry.return_storage) {
            let mut load_pointer = |index: usize| {
                let address = child.instruction(
                    pointer_ty.clone(),
                    format!(
                        "getelementptr {}, ptr %environment, i32 0, i32 {index}",
                        env_ty.llvm()
                    ),
                );
                child
                    .instruction(
                        pointer_ty.clone(),
                        format!("load ptr, ptr {}", address.text),
                    )
                    .text
            };
            let token = load_pointer(offset);
            let pointer = load_pointer(offset + 1);
            child.outlined_return = Some(ReturnStorage {
                token,
                value: Slot {
                    ty: storage.value.ty.clone(),
                    pointer,
                },
            });
        }
        for (position, id) in remote_loops.keys().enumerate() {
            let mut load_pointer = |index: usize| {
                let address = child.instruction(
                    pointer_ty.clone(),
                    format!(
                        "getelementptr {}, ptr %environment, i32 0, i32 {index}",
                        env_ty.llvm()
                    ),
                );
                child
                    .instruction(
                        pointer_ty.clone(),
                        format!("load ptr, ptr {}", address.text),
                    )
                    .text
            };
            let exit = load_pointer(loop_offset + position * 2);
            let again = load_pointer(loop_offset + position * 2 + 1);
            child.outlined_loops.insert(*id, (exit, again));
        }
        let fallback = entry
            .handler
            .fallback
            .map(|index| {
                child
                    .effects
                    .get(index)
                    .cloned()
                    .ok_or("host outlined direct fallback missing")
            })
            .transpose()?;
        let mut inputs = vec!["ptr %environment".to_string()];
        let types = parameters
            .iter()
            .map(|p| {
                if p.sort != Sort::Runtime {
                    return Err("host direct entry retains staged parameters".to_string());
                }
                repr(child.graph, &p.ty, child.checked)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let packet_ty = Ty::Record(Arc::new(types.clone()));
        let mut packet = Value {
            ty: packet_ty.clone(),
            text: "zeroinitializer".into(),
        };
        for (index, ty) in types.iter().enumerate() {
            if *ty == Ty::Unit {
                continue;
            }
            inputs.push(format!("{} %arg{index}", ty.llvm()));
            packet = child.instruction(
                packet_ty.clone(),
                format!(
                    "insertvalue {} {}, {} %arg{index}, {index}",
                    packet_ty.llvm(),
                    packet.text,
                    ty.llvm()
                ),
            );
        }
        let value = child.match_with_fallback(
            packet,
            &entry.handler.arms,
            result,
            0,
            fallback,
            Some(HandlerInvocation {
                scope: entry.scope.as_ref(),
                request: entry
                    .scope
                    .as_ref()
                    .map(|s| s.installation)
                    .unwrap_or(nir::program::SourceRange { start: 0, end: 0 }),
            }),
            None,
        )?;
        if !child.terminated {
            if **result == Term::Primitive(P::Never) {
                return Err("host Never handler has a normal completion path".into());
            }
            if value.ty != result_ty {
                return Err("host direct handler return representation mismatch".into());
            }
            child.line(if value.ty == Ty::Unit {
                "ret void".into()
            } else {
                format!("ret {} {}", value.ty.llvm(), value.text)
            });
        }
        let definition = format!(
            "define internal {} @{symbol}({}) {{\nentry:\n{}{}}}\n",
            result_ty.llvm(),
            inputs.join(", "),
            child.prologue,
            child.body
        );
        child.declarations.insert(definition);
        Ok((format!("@{symbol}"), env.pointer))
    }
}

impl Emitter<'_> {
    /// Propagate control without running the current callable's postconditions:
    /// this is not a normal result (and not a user-visible error result).
    pub(super) fn propagate_effect_control(&mut self) -> Result<(), String> {
        let result = self
            .failure_result
            .clone()
            .ok_or("host effect control needs a checked machine result")?;
        self.line(if result == Ty::Unit {
            "ret void".into()
        } else {
            format!("ret {} zeroinitializer", result.llvm())
        });
        self.terminated = true;
        Ok(())
    }

    pub(super) fn check_effect_control(
        &mut self,
        source: nir::program::SourceRange,
    ) -> Result<(), String> {
        if !self.checked {
            return Ok(());
        }
        let pending = self.instruction(Ty::Pointer, "load ptr, ptr @flurry_effect_control".into());
        let active = self.instruction(Ty::Bool, format!("icmp ne ptr {}, null", pending.text));
        let escape = self.fresh();
        let good = self.fresh();
        self.line(format!(
            "br i1 {}, label %{escape}, label %{good}",
            active.text
        ));
        self.label(&escape);
        if let Some(storage) = self.native_return.clone() {
            let ours = self.instruction(
                Ty::Bool,
                format!("icmp eq ptr {}, {}", pending.text, storage.token),
            );
            let receive = self.fresh();
            let forward = self.fresh();
            self.line(format!(
                "br i1 {}, label %{receive}, label %{forward}",
                ours.text
            ));
            self.label(&receive);
            self.line("store ptr null, ptr @flurry_effect_control");
            let value = self.load(&storage.value);
            self.receive_effect_return(&value, source)?;
            self.label(&forward);
        }
        *self.fuel = self
            .fuel
            .checked_sub(self.loops.len().saturating_mul(2))
            .ok_or("host effect loop dispatch budget exhausted")?;
        let loops = self.loops.clone();
        for (id, target) in loops {
            if let Some((exit, again)) = &target.control {
                for (token, is_break) in [(exit, true), (again, false)] {
                    let ours = self
                        .instruction(Ty::Bool, format!("icmp eq ptr {}, {token}", pending.text));
                    let receive = self.fresh();
                    let forward = self.fresh();
                    self.line(format!(
                        "br i1 {}, label %{receive}, label %{forward}",
                        ours.text
                    ));
                    self.label(&receive);
                    self.line("store ptr null, ptr @flurry_effect_control");
                    self.receive_effect_loop(id, &target, source, is_break)?;
                    self.label(&forward);
                }
            }
        }
        self.cleanup_boundary(source, iir::cleanup::Boundary::EffectEscape)?;
        self.unwind_effect_frames()?;
        self.propagate_effect_control()?;
        self.label(&good);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct ControlDestinations {
    pub returns: bool,
    pub loops: BTreeSet<LocalId>,
}

/// Only handler exits need dynamic installation targets in this machine function.
/// Ordinary returns and returns in separately compiled closure bodies must not
/// allocate a control destination or duplicate the function's postconditions.
/// Loop exits within their declaring handler scope remain ordinary branches;
/// only an exit crossing a handler boundary needs a dynamic target.
/// Each deferred handler scope is queued once; nested handlers remain linear.
pub(super) fn control_destinations(
    body: &Expr,
    fuel: &mut usize,
) -> Result<ControlDestinations, String> {
    let mut scopes = vec![(body, false)];
    let mut needed = ControlDestinations::default();
    while let Some((scope, in_handler)) = scopes.pop() {
        let mut local_loops = BTreeSet::new();
        let mut exits = BTreeSet::new();
        nir::visit::expression_in_scope(scope, fuel, |node, fuel| {
            if in_handler {
                match node.kind {
                    K::Return(_) => needed.returns = true,
                    K::Loop { identity, .. } => {
                        local_loops.insert(identity);
                    }
                    K::Break(target) | K::Continue(target) => {
                        exits.insert(target);
                    }
                    _ => {}
                }
            }
            if let K::EffectHandle { handlers, .. } = &node.kind {
                *fuel = fuel
                    .checked_sub(handlers.len())
                    .ok_or("host control destination analysis budget exhausted")?;
                for handler in handlers {
                    *fuel = fuel
                        .checked_sub(handler.arms.len())
                        .ok_or("host control destination analysis budget exhausted")?;
                    for arm in &handler.arms {
                        scopes.extend(
                            nir::visit::pattern_observations(&arm.pattern, fuel)?
                                .into_iter()
                                .map(|expr| (expr, true)),
                        );
                        scopes.extend(arm.guard.iter().map(|expr| (expr, true)));
                        scopes.push((&arm.body, true));
                    }
                }
            }
            Ok(())
        })?;
        needed.loops.extend(exits.difference(&local_loops).copied());
    }
    Ok(needed)
}

#[cfg(test)]
mod destination_tests {
    use super::*;
    use nir::program::{Capture, EffectHandler, MatchArm, Pattern, SourceRange};

    fn expr(kind: K) -> Expr {
        Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::Primitive(P::Void),
            kind,
        }
    }
    fn id(index: u32) -> LocalId {
        LocalId { owner: 0, index }
    }
    fn handler(body: Expr) -> Expr {
        expr(K::EffectHandle {
            computation: Box::new(expr(K::Unit)),
            handlers: vec![EffectHandler {
                operation: nir::effects::Operation {
                    declaration: nir::DeclarationId {
                        package: 0,
                        local: 0,
                    },
                    arguments: vec![],
                },
                abi: nir::effects::Abi::Direct,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(P::Void)),
                },
                arms: vec![MatchArm {
                    pattern: Pattern::Wild,
                    guard: None,
                    body,
                }],
                fallback: None,
            }],
            outer: vec![],
        })
    }
    fn loop_body(identity: LocalId, body: Expr) -> Expr {
        expr(K::Loop {
            identity,
            body: Box::new(body),
        })
    }

    #[test]
    fn local_and_cross_handler_loop_exits_have_different_destinations() {
        let local = handler(loop_body(id(1), expr(K::Break(id(1)))));
        assert!(
            control_destinations(&local, &mut 1000)
                .unwrap()
                .loops
                .is_empty()
        );
        let crossing = handler(loop_body(id(1), handler(expr(K::Continue(id(1))))));
        let targets = control_destinations(&crossing, &mut 1000).unwrap();
        assert_eq!(targets.loops, BTreeSet::from([id(1)]));
        assert!(!targets.returns);
        let ordinary = loop_body(id(2), expr(K::Break(id(2))));
        assert!(
            control_destinations(&ordinary, &mut 1000)
                .unwrap()
                .loops
                .is_empty()
        );
    }

    #[test]
    fn closure_body_is_separate_but_capture_initializer_is_in_current_scope() {
        let closure = expr(K::Closure {
            parameters: vec![],
            captures: vec![Capture {
                binding: id(3),
                ty: Term::Primitive(P::Void),
                value: Some(Box::new(handler(expr(K::Break(id(2)))))),
            }],
            body: std::sync::Arc::new(handler(expr(K::Return(None)))),
        });
        let targets = control_destinations(&closure, &mut 1000).unwrap();
        assert_eq!(targets.loops, BTreeSet::from([id(2)]));
        assert!(!targets.returns);
        let K::Closure { body, .. } = &closure.kind else {
            unreachable!()
        };
        assert!(control_destinations(body, &mut 1000).unwrap().returns);
    }

    #[test]
    fn observations_and_guards_are_part_of_handler_control() {
        let mut root = handler(expr(K::Unit));
        let K::EffectHandle { handlers, .. } = &mut root.kind else {
            unreachable!()
        };
        handlers[0].arms[0].pattern = Pattern::Then {
            first: Box::new(Pattern::Wild),
            value: Box::new(expr(K::Continue(id(4)))),
            next: Box::new(Pattern::Wild),
        };
        handlers[0].arms[0].guard = Some(expr(K::Return(None)));
        let targets = control_destinations(&root, &mut 1000).unwrap();
        assert_eq!(targets.loops, BTreeSet::from([id(4)]));
        assert!(targets.returns);
    }

    #[test]
    fn nested_analysis_is_linear_and_budget_failure_can_be_retried() {
        let mut root = expr(K::Break(id(7)));
        for _ in 0..64 {
            root = handler(root);
        }
        let before = root.clone();
        assert!(control_destinations(&root, &mut 0).is_err());
        assert!(control_destinations(&root, &mut 32).is_err());
        // A quadratic rescan of all nested arms cannot fit this linear budget.
        let targets = control_destinations(&root, &mut (64 * 12)).unwrap();
        assert_eq!(targets.loops, BTreeSet::from([id(7)]));
        assert_eq!(root, before);
    }
}
