//! Completed module and associated constants. Cached expressions reconstruct frozen value
//! graphs; active inference, caller premises and interpreter storage do not enter.
use super::*;
use comptime_interpretor::Value;

#[derive(Default)]
struct Materialization {
    // Keys identify interpreter closures only during this one snapshot. Their
    // addresses never enter NIR, diagnostics, persistent caches or dumps.
    closures: BTreeMap<usize, (n::LocalId, Term)>,
    active: BTreeSet<usize>,
    bindings: Vec<n::Expr>,
}

// Patterns have already been resolved and checked. Collect their private local
// identities, but check every embedded computation with the same availability
// rules as the surrounding staged argument. Matching itself stays in the VM.
fn staged_pattern_inputs<'a>(
    pattern: &'a n::Pattern,
    bindings: &mut BTreeSet<n::LocalId>,
    expressions: &mut Vec<&'a n::Expr>,
    fuel: &mut u64,
    span: Span,
) -> Result<(), Error> {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        *fuel = fuel.checked_sub(1).ok_or_else(|| error(span,"staged pattern structure budget exhausted"))?;
        match pattern {
            n::Pattern::Wild | n::Pattern::Unit | n::Pattern::Literal(_) => {},
            n::Pattern::Tuple(fields) | n::Pattern::TypeTuple(fields)
            | n::Pattern::Or(fields) | n::Pattern::Record {fields,..} | n::Pattern::AnonymousRecord { fields, .. }
            | n::Pattern::Variant {fields,..}
            | n::Pattern::NormalForm {arguments:fields,..} => pending.extend(fields),
            n::Pattern::Binding {binding,inner} => {
                bindings.insert(*binding);
                pending.push(inner);
            },
            n::Pattern::Remap {pattern,bindings:remapping} => {
                bindings.extend(remapping.iter().map(|(_,to)| *to));
                pending.push(pattern);
            },
            n::Pattern::Guard {pattern,condition} => {
                pending.push(pattern);
                expressions.push(condition);
            },
            n::Pattern::Then {first,value,next} => {
                pending.push(first);
                expressions.push(value);
                pending.push(next);
            },
        }
    }
    Ok(())
}

impl Checker<'_, '_> {
    pub(super) fn materialize_local_constant(&mut self,value:Value,ty:Term,span:Span)->Result<n::Expr,Error> {
        if matches!(value,Value::Type(_) | Value::TypeList(_)) {
            return self.materialize_generator_data(value,ty,span);
        }
        // Declaration identities are ordinary constant data, not quoted-code
        // evidence. Reuse module-constant reconstruction and phase validation.
        let mut snapshot=Materialization::default();
        let value=self.constant_expression(value,ty,span,0,&mut snapshot)?;
        if snapshot.bindings.is_empty() {return Ok(value);}
        Ok(n::Expr {source:super::source(span),ty:value.ty.clone(),kind:K::Block {
            statements:snapshot.bindings,result:Some(Box::new(value)),
        }})
    }

    pub(super) fn materialize_generator_data(&mut self, value: Value, ty: Term, span: Span) -> Result<n::Expr, Error> {
        // Local construction data can retain the enclosing Type telescope;
        // immutable module exports use the stricter closed-value boundary.
        if matches!(value,Value::Type(_) | Value::TypeList(_)) {
            let symbol=self.required_normal_term(value,span)?;
            let kind=match symbol {
                Term::TypeListValue(elements) if self.is_type_list_type(&ty)=>K::TypeListValue(elements),
                symbol if ty==p(P::Type)=>K::TypeValue(symbol),
                _=>return Err(error(span,"local Type data does not match its declared domain")),
            };
            return Ok(n::Expr {source:super::source(span),ty,kind});
        }
        // A data argument is a completed value, not a live interpreter graph.
        // In particular, a nominal wrapper cannot smuggle mutable storage or
        // closure identities across the quote boundary.
        let copy_work=value.immutable_copy_work()
            .ok_or_else(||error(span,"code generator data cannot be frozen: unsupported or excessively deep immutable value"))?;
        self.required_steps=self.required_steps.checked_sub(copy_work as u64)
            .ok_or_else(||error(span,"code generator data freezing budget exhausted"))?;
        let mut budget = comptime_runtime::Budget {
            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX), max_limbs:4096,
        };
        let before = budget.steps;
        let frozen = value.clone().into_normal_term(&mut budget);
        self.required_steps -= (before-budget.steps) as u64;
        frozen.map_err(|failure| error(span, format!("code generator data cannot be frozen: {failure:?}")))?;
        let mut snapshot = Materialization::default();
        let value = self.constant_expression(value, ty, span, 0, &mut snapshot)?;
        if !snapshot.bindings.is_empty() {
            return Err(error(span, "generator data graph lifting is not yet implemented"));
        }
        Ok(value)
    }

    pub(super) fn freeze_dependency_input(&mut self, source: &Expr<'_>, input: n::Expr,
        bindings: Vec<(n::LocalId,Term,Term)>) -> Result<(n::Expr,Term),Error> {
        let (value,ty) = self.required_normal_input(source,input,bindings)?;
        // A call-site dependency may retain the caller's Type telescope. It is
        // not an exported, closed module constant; use the same scoped boundary
        // as normal-form inputs, after required evaluation restores those slots.
        if matches!(value,Value::Type(_) | Value::TypeList(_)) {
            let symbol=self.required_normal_term(value,source.span)?;
            let kind=match &symbol {
                Term::TypeListValue(elements) => {
                    if !self.is_type_list_type(&ty) {
                        return Err(error(source.span,"type-list dependency requires the bound meta.List<Type>"));
                    }
                    K::TypeListValue(elements.clone())
                }
                _ => {
                    if ty!=p(P::Type) {return Err(error(source.span,"Type dependency requires a Type domain"));}
                    K::TypeValue(symbol.clone())
                }
            };
            return Ok((n::Expr {source:super::source(source.span),ty,kind},symbol));
        }
        let symbol = self.required_normal_term(value.clone(),source.span)?;
        let mut snapshot = Materialization::default();
        let value = self.constant_expression(value,ty,source.span,0,&mut snapshot)?;
        if !snapshot.bindings.is_empty() { return Err(error(source.span,"dependent input graph materialization is not yet supported")); }
        Ok((value,symbol))
    }
    pub(super) fn constant_value(&mut self, owner: OwnerId, span: Span) -> Result<n::Expr, Error> {
        // Trait defaults are evaluated separately for each selected implementation.
        let selected_associated=self.selected_associated_implementation.is_some()
            && matches!(self.package.item(owner).map(|item| &item.kind),Some(ItemKind::Associated(_)));
        if let Some(value) = self.constants.get(&owner).filter(|_|!selected_associated) {
            charge_copy(value, &mut self.required_steps, span)?;
            return Ok(value.clone());
        }
        // Selected associated recursion is keyed by (implementation, member)
        // in selected_associated_value, rather than by the shared default owner.
        if !selected_associated && (self.active_constants.len() >= 64 || !self.active_constants.insert(owner)) {
            return Err(error(
                span,
                "cyclic or excessively deep constant dependency",
            ));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_checking = std::mem::replace(&mut self.checking_contract, false);
        self.context = self.initializer_implementation()
            .and_then(|implementation|self.associated_owner(implementation.declaration))
            .map(|owner|self.definition_context(owner))
            .unwrap_or_else(||self.nominal_context());
        let result = (|| {
            let item = self.package.item(owner).unwrap();
            let (annotation, initializer) = match &item.kind {
                ItemKind::Const(annotation, initializer) => (*annotation, *initializer),
                ItemKind::Associated(associated) => (associated.ty, associated.default.ok_or_else(|| error(span,"required associated value needs a selected implementation"))?),
                _ => return Err(error(span,"declaration is not a constant or associated value")),
            };
            let expected = if matches!(annotation.kind, H::TyPlaceholder) {
                None
            } else {
                Some(self.type_expr(annotation)?)
            };
            let (value, ty) = self.required_typed_expression(initializer, expected.as_ref())?;
            let mut snapshot = Materialization::default();
            let result = self.constant_expression(value, ty, initializer.span, 0, &mut snapshot)?;
            if snapshot.bindings.is_empty() {
                Ok(result)
            } else {
                let span = source(initializer.span);
                let ty = result.ty.clone();
                let body = n::Expr {
                    source: span,
                    ty: ty.clone(),
                    kind: K::Block {
                        statements: snapshot.bindings,
                        result: Some(Box::new(result)),
                    },
                };
                // Reconstruct the graph in a private invocation namespace. A
                // cached graph's binding IDs must never overwrite caller locals.
                let callee = n::Expr {
                    source: span,
                    ty: Term::ComptimeFunction(Box::new(Term::Function {
                        parameters: vec![],
                        result: Box::new(ty.clone()),
                    })),
                    kind: K::Closure {
                        parameters: vec![],
                        captures: vec![],
                        body: std::sync::Arc::new(body),
                    },
                };
                Ok(n::Expr {
                    source: span,
                    ty,
                    kind: K::Call {
                        callee: Box::new(callee),
                        arguments: vec![],
                        parameter_bindings: vec![],
                    },
                })
            }
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.checking_contract = saved_checking;
        if !selected_associated {self.active_constants.remove(&owner);}
        if let Ok(value) = &result {
            charge_copy(value, &mut self.required_steps, span)?;
            if !selected_associated {self.constants.insert(owner, value.clone());}
        }
        result
    }

    pub(super) fn record_staged_pattern(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let mut origin = value;
        while let K::AnonymousField { value, .. } | K::Field {value,..} = &origin.kind {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "staged projection provenance budget exhausted"))?;
            origin = value;
        }
        let dependencies = if let K::Local(input) = origin.kind {
            BTreeSet::from([input])
        } else {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let result = iir::staging::structural_boolean_dependencies(value, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            let Some(dependencies) = result.map_err(|e| error(span, e))? else {
                return Ok(());
            };
            dependencies
        };
        if dependencies.iter().any(|input| {
            !self.value_input_bindings.contains_key(input)
                && !self.type_input_bindings.contains_key(input)
                && !self.derived_staged_bindings.contains(input)
        }) {
            return Ok(());
        }
        let mut pending = vec![pattern];
        let mut bindings = Vec::new();
        while let Some(pattern) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "staged binding provenance budget exhausted"))?;
            match pattern {
                n::Pattern::Wild => {}
                n::Pattern::Binding { binding, inner } => {
                    bindings.push(*binding);
                    pending.push(inner);
                }
                n::Pattern::Tuple(fields) | n::Pattern::AnonymousRecord { fields, .. } | n::Pattern::Record {fields,..} => {
                    pending.extend(fields)
                }
                _ => return Ok(()),
            }
        }
        self.derived_staged_bindings.extend(bindings);
        Ok(())
    }

    pub(super) fn freeze_staged_call_inputs(&mut self, mut call:n::Expr, source:&Expr<'_>) -> Result<n::Expr,Error> {
        if self.in_required_comptime || self.preparing_spread == Some(n::SourceRange {start:source.span.lo().0,end:source.span.hi().0}) {return Ok(call);}
        let K::Call {callee,arguments,parameter_bindings}=&mut call.kind else {return Ok(call);};
        let Term::Function {parameters,..}=callee.ty.callable_signature() else {return Ok(call);};
        if !parameters.iter().any(|p|p.sort!=Sort::Runtime) {return Ok(call);}
        let staged_bindings:BTreeSet<_>=self.signature_owner.and_then(|owner|self.package.item(owner)).map(|item| {
            match &item.kind {
                ItemKind::Fn(sig,_) | ItemKind::FnDeclaration(sig)=>sig.params.iter().filter(|parameter|parameter.ty.is_comptime()).map(|parameter|local(parameter.hir_id)).collect(),
                _=>BTreeSet::new(),
            }
        }).unwrap_or_default();
        let type_bindings=self.signature_owner.and_then(|owner|self.type_parameters.get(&owner)).cloned().unwrap_or_default();
        let mut inputs=Vec::new();
        let mut prepared:BTreeMap<n::LocalId,n::Expr>=BTreeMap::new();
        let mut deferred_bindings=BTreeSet::new();
        for argument in arguments {
            if parameters[argument.parameter].sort==Sort::Runtime {continue;}
            if argument.is_default {
                let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before=fuel;
                let replaced=iir::staging::prepare_input_with_layouts(&self.normal_form_metadata(), &argument.value,&prepared,&mut fuel);
                self.required_steps-=(before-fuel) as u64;
                argument.value=replaced.map_err(|e|error(source.span,e))?;
            }
            let deferred=self.staged_input_can_defer(&argument.value,argument.is_default,&deferred_bindings,&staged_bindings,&type_bindings,source.span)?;
            if deferred {
                if let Some(binding)=parameter_bindings.get(argument.parameter) {deferred_bindings.insert(*binding);}
                continue;
            }
            let bindings=if argument.is_default {inputs.clone()} else {vec![]};
            let (value,ty)=self.required_normal_input(source,argument.value.clone(),bindings)?;
            let mut budget=comptime_runtime::Budget {steps:usize::try_from(self.required_steps).unwrap_or(usize::MAX),max_limbs:4096};
            let before=budget.steps;
            let symbolic=value.clone().into_normal_term(&mut budget);
            self.required_steps-=(before-budget.steps) as u64;
            let symbolic=symbolic.map_err(|e|error(source.span,format!("staged argument cannot be frozen: {e:?}")))?;
            let mut snapshot=Materialization::default();
            argument.value=self.constant_expression(value,ty.clone(),source.span,0,&mut snapshot)?;
            if !snapshot.bindings.is_empty() {return Err(error(source.span,"staged argument graph materialization is not yet supported"));}
            if let Some(binding)=parameter_bindings.get(argument.parameter) {
                inputs.push((*binding,ty,symbolic));
                prepared.insert(*binding,argument.value.clone());
            }
        }
        Ok(call)
    }

    fn staged_input_can_defer(
        &mut self, value:&n::Expr, is_default:bool,
        deferred_bindings:&BTreeSet<n::LocalId>, staged_bindings:&BTreeSet<n::LocalId>,
        type_bindings:&[nir::DeclarationId], span:Span,
    ) -> Result<bool,Error> {
            let mut deferred=false;
            let mut structural=true;
            let mut block_bindings=BTreeSet::new();
            let mut local_loops=BTreeSet::new();
            let mut pending=vec![value];
            while let Some(value)=pending.pop() {
                self.required_steps=self.required_steps.checked_sub(1).ok_or_else(||error(span,"staged input structure budget exhausted"))?;
                match &value.kind {
                    K::Local(binding) if is_default && deferred_bindings.contains(binding)=>deferred=true,
                    K::Local(binding) if staged_bindings.contains(binding) || self.derived_staged_bindings.contains(binding)=>deferred=true,
                    K::Local(binding) if block_bindings.contains(binding)=>{},
                    K::Local(binding) if self.required_runtime_parameter(*binding)=>{
                        return Err(error(span,"required comptime computation cannot read a runtime parameter; defer does not change the parameter's phase"));
                    },
                    K::Call {callee,arguments,..} if callee.ty.is_comptime_callable()=>{
                        pending.push(callee);
                        pending.extend(arguments.iter().map(|argument| &argument.value));
                    },
                    K::Call {..}=>{
                        return Err(error(span,"callable type is unavailable in this execution phase: runtime call in a comptime input"));
                    },
                    K::Function(_) if value.ty.is_comptime_callable()=>{},
                    K::Closure {captures,..} if value.ty.is_comptime_callable()=>{
                        // The body is separately phase-checked. Its parameters
                        // belong to invocation; only environment construction
                        // reads the surrounding stage here. Derived locals use
                        // the same provenance as direct argument references;
                        // specialization still checks writes/address escape
                        // before installing an owned capture snapshot.
                        deferred |= type_bindings.iter().any(|id|value.ty.mentions_rigid(*id));
                        for capture in captures {
                            self.required_steps=self.required_steps.checked_sub(1).ok_or_else(||error(span,"staged capture structure budget exhausted"))?;
                            if let Some(value)=&capture.value {pending.push(value);}
                            else if staged_bindings.contains(&capture.binding)
                                || self.derived_staged_bindings.contains(&capture.binding)
                                || (is_default && deferred_bindings.contains(&capture.binding)) {deferred=true;}
                            else if !block_bindings.contains(&capture.binding) {structural=false;}
                        }
                    },
                    K::FunctionInstance {arguments,..} if value.ty.is_comptime_callable()=>{
                        deferred |= arguments.iter().any(|ty|type_bindings.iter().any(|id|ty.mentions_rigid(*id)));
                    },
                    K::TraitMethod {subject,interface_arguments,method_arguments,..} if value.ty.is_comptime_callable()=>{
                        deferred |= std::iter::once(subject).chain(interface_arguments).chain(method_arguments)
                            .any(|ty|type_bindings.iter().any(|id|ty.mentions_rigid(*id)));
                    },
                    K::AssociatedValue {interface,subject,..}=>{
                        deferred |= std::iter::once(subject).chain(&interface.arguments)
                            .any(|ty|type_bindings.iter().any(|id|ty.mentions_rigid(*id)));
                    },
                    K::Tuple(values) | K::Record {fields:values,..}
                    | K::AnonymousRecord {fields:values} | K::Variant {fields:values,..}=>pending.extend(values),
                    K::Not(value) | K::Discard(value) | K::RequirePredicate(value) | K::EnsurePredicate(value) | K::CheckPredicate(value)
                    | K::Snapshot(value) | K::TakeBits(value) | K::SymbolName(value) | K::EnumName(value) | K::BitConvert(value) | K::BitSlice {value,..}
                    | K::Field {value,..} | K::AnonymousField {value,..}=>pending.push(value),
                    // Place fields are checked structural projections, never
                    // pointer dereferences or computations on caller storage.
                    K::Assign {place,value} | K::BitAssign {place,value,..}
                        if block_bindings.contains(&place.root)=>pending.push(value),
                    K::BitUpdate {place,selections,value,..} if block_bindings.contains(&place.root)=>{
                        pending.extend(selections.iter().flat_map(n::BitSelection::expressions));pending.push(value);
                    },
                    K::BitWindow {value,start,end}=>pending.extend([value.as_ref(),start.as_ref(),end.as_ref()]),
                    K::BitEqual(left,right) | K::BitConcat(left,right) | K::BitIndex(left,right)=>{pending.push(left);pending.push(right);},
                    K::And(left,right) | K::Or(left,right)
                        if matches!(value.ty,Term::Primitive(P::BitVec(_)))=>{pending.push(left);pending.push(right);},
                    K::Loop {identity,body}=>{local_loops.insert(*identity);pending.push(body);},
                    K::Break(identity) | K::Continue(identity) if local_loops.contains(identity)=>{},
                    K::And(left,right) | K::Or(left,right) | K::ScalarEqual(left,right)
                    | K::ScalarLess(left,right) | K::ScalarLessEqual(left,right)
                    | K::ScalarGreater(left,right) | K::ScalarGreaterEqual(left,right)
                        if value.ty==p(P::Bool)=>{pending.push(left);pending.push(right);},
                    K::FloatNegate(inner) if matches!(value.ty,Term::Primitive(P::F32 | P::F64))=>pending.push(inner),
                    K::FloatBinary {left,right,..} if matches!(value.ty,Term::Primitive(P::F32 | P::F64))=>{pending.push(left);pending.push(right);},
                    K::IntegerNegate(inner) if n::machine_integer_type(value).is_some() || value.ty==p(P::Integer)=>pending.push(inner),
                    K::IntegerBinary {op,left,right} if value.ty==p(P::Integer) || (op.is_comparison() && value.ty==p(P::Bool)) || (n::machine_integer_type(value).is_some()
                        && matches!(op,n::IntegerOp::Add | n::IntegerOp::Subtract | n::IntegerOp::Multiply | n::IntegerOp::DivideTrunc | n::IntegerOp::RemainderTrunc))=>{pending.push(left);pending.push(right);},
                    K::If {condition,then_branch,else_branch}=>{
                        pending.push(condition);pending.push(then_branch);pending.extend(else_branch.as_deref());
                    }
                    K::Match {value,arms}=>{
                        pending.push(value);
                        for arm in arms {
                            staged_pattern_inputs(&arm.pattern,&mut block_bindings,&mut pending,&mut self.required_steps,span)?;
                            pending.extend(arm.guard.as_ref());
                            pending.push(&arm.body);
                        }
                    }
                    K::Block {statements,result} | K::CallPreparation {statements,result}=>{
                        // These are already resolved, uniquely identified locals.
                        // Only these block-owned locals may be updated. Captured
                        // caller storage remains outside staged input evaluation.
                        pending.extend(result.as_deref());
                        for statement in statements.iter().rev() {
                            if let K::Let {binding,value}= &statement.kind {
                                block_bindings.insert(*binding);
                                pending.push(value);
                            } else { pending.push(statement); }
                        }
                    },
                    K::LetPattern {pattern,value,guard,otherwise}=>{
                        staged_pattern_inputs(pattern,&mut block_bindings,&mut pending,&mut self.required_steps,span)?;
                        pending.push(value);
                        pending.extend(guard.as_deref());
                        pending.extend(otherwise.as_deref());
                    },
                    K::TypeValue(ty)=>{
                        deferred |= type_bindings.iter().any(|id|ty.mentions_rigid(*id));
                        deferred |= !ty.substitution_parameters().map_err(|e|error(span,e.to_string()))?.is_empty();
                    },
                    K::TypeListValue(types)=>{
                        for ty in types {
                            deferred |= type_bindings.iter().any(|id|ty.mentions_rigid(*id));
                            deferred |= !ty.substitution_parameters().map_err(|e|error(span,e.to_string()))?.is_empty();
                        }
                    },
                    K::NormalFormType {arguments,plan,..}=>{
                        if let Some(plan)=plan {
                            if !plan.valid(arguments.len()) {structural=false;break;}
                            block_bindings.extend(plan.local_bindings().unwrap());
                            pending.extend(plan.preparations.iter().map(|p| &p.expression));
                        }
                        for argument in arguments {
                            for pattern in argument.patterns() {
                                staged_pattern_inputs(pattern,&mut block_bindings,&mut pending,&mut self.required_steps,span)?;
                            }
                            pending.extend(argument.expressions());
                        }
                    },
                    K::Literal(_) | K::Unit=>{},
                    _=>{structural=false;break;},
                }
            }
            Ok(deferred && structural)
    }

    pub(super) fn freeze_staged_preparation(&mut self, mut expression:n::Expr, source:&Expr<'_>) -> Result<n::Expr,Error> {
        if self.in_required_comptime {return Ok(expression);}
        let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before=fuel;
        let plan=iir::staging::preparation(&expression,&mut fuel);
        self.required_steps-=(before-fuel) as u64;
        let Some(plan)=plan.map_err(|e|error(source.span,e))? else {return Ok(expression)};
        let staged_bindings:BTreeSet<_>=self.signature_owner.and_then(|owner|self.package.item(owner)).map(|item| {
            match &item.kind {
                ItemKind::Fn(sig,_) | ItemKind::FnDeclaration(sig)=>sig.params.iter().filter(|parameter|parameter.ty.is_comptime()).map(|parameter|local(parameter.hir_id)).collect(),
                _=>BTreeSet::new(),
            }
        }).unwrap_or_default();
        let type_bindings=self.signature_owner.and_then(|owner|self.type_parameters.get(&owner)).cloned().unwrap_or_default();
        if self.staged_input_can_defer(&plan.expression,false,&BTreeSet::new(),&staged_bindings,&type_bindings,source.span)? {return Ok(expression)};
        charge_copy(&plan.expression,&mut self.required_steps,source.span)?;
        let (value,ty)=self.required_normal_input(source,plan.expression.clone(),vec![])?;
        let mut snapshot=Materialization::default();
        let completed=self.constant_expression(value,ty,source.span,0,&mut snapshot)?;
        if !snapshot.bindings.is_empty() {return Err(error(source.span,"staged preparation graph materialization is not yet supported"));}
        let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before=fuel;
        let result=plan.apply_with_layouts(&self.normal_form_metadata(), &mut expression,completed,&mut fuel);
        self.required_steps-=(before-fuel) as u64;
        result.map_err(|e|error(source.span,e))?;
        Ok(expression)
    }

    fn constant_expression(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
        depth: usize,
        snapshot: &mut Materialization,
    ) -> Result<n::Expr, Error> {
        if depth >= 128 || self.required_steps == 0 {
            return Err(error(span, "constant materialization budget exhausted"));
        }
        self.required_steps -= 1;
        let kind = match value {
            Value::TypeParameter(_) => {
                return Err(error(span,"abstract type input cannot be exported as a completed constant"));
            }
            Value::AnonymousRecord(values) => {
                let Term::Record(types) = &ty else {
                    return Err(error(span, "anonymous record constant requires a record type"));
                };
                if values.len() != types.len() || values.iter().zip(types).any(|((name, _), (expected, _))| name != expected) {
                    return Err(error(span, "anonymous record constant fields disagree with its type"));
                }
                let fields = values.into_iter().zip(types).map(|((_, value), (_, ty))| {
                    self.constant_expression(value, ty.clone(), span, depth + 1, snapshot)
                }).collect::<Result<_, _>>()?;
                K::AnonymousRecord { fields }
            }
            Value::Pointer(_) | Value::Storage(_) => {
                return Err(error(
                    span,
                    "runtime storage cannot be materialized as a constant",
                ));
            }
            Value::Type(value) => K::TypeValue(value),
            Value::TypeList(types) => {
                if !self.is_type_list_type(&ty) {
                    return Err(error(span, "type-list constant requires the bound meta.List<Type>"));
                }
                for element in &types {
                    let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before=fuel;
                    let checked=iir::completed_type(element,&BTreeSet::new(),&mut fuel);
                    self.required_steps-=(before-fuel) as u64;
                    checked.map_err(|e|error(span,e))?;
                    if element.sort()!=Sort::Type {return Err(error(span,"type-list element is not Type"));}
                }
                K::TypeListValue(types)
            }
            Value::Unit => K::Unit,
            Value::Bool(value) => K::Literal(n::Literal::Bool(value)),
            Value::Char(value) => K::Literal(n::Literal::Char(value)),
            Value::Symbol(value) => {
                self.required_steps = self
                    .required_steps
                    .checked_sub(value.len() as u64)
                    .ok_or_else(|| error(span, "constant materialization budget exhausted"))?;
                K::Literal(n::Literal::Symbol(value.to_string()))
            }
            Value::String(value) => {
                self.required_steps = self
                    .required_steps
                    .checked_sub(value.len() as u64)
                    .ok_or_else(|| error(span, "constant materialization budget exhausted"))?;
                K::Literal(n::Literal::String(value.to_string()))
            }
            Value::SignedInteger(value) => K::Literal(n::Literal::SignedInteger(value)),
            Value::UnsignedInteger(value) => K::Literal(n::Literal::UnsignedInteger(value)),
            Value::Float32(value) => K::Literal(n::Literal::Float32(value)),
            Value::Float64(value) => K::Literal(n::Literal::Float64(value)),
            Value::Integer(value) => {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let start = budget.steps;
                let spelling = value.to_hex_literal(&mut budget);
                self.required_steps -= (start - budget.steps) as u64;
                K::Literal(n::Literal::Integer(
                    spelling.map_err(|e| error(span, e.to_string()))?,
                ))
            }
            Value::BitVec(value) => K::Literal(n::Literal::BitVec {
                width: value.width(),
                words: value.words().to_vec(),
            }),
            Value::Tuple(values) => {
                let shape = self.structural_type(&ty, span)?;
                let Term::Tuple(types) = &shape else {
                    return Err(error(span, "constant tuple type mismatch"));
                };
                if values.len() != types.len() {
                    return Err(error(span, "constant tuple arity mismatch"));
                }
                K::Tuple(
                    values
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_expression(value, ty.clone(), span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Record {
                arguments,
                declaration,
                fields,
            } => {
                if ty
                    != (Term::Nominal {
                        declaration,
                        arguments: arguments.clone(),
                    })
                {
                    return Err(error(span, "constant record identity mismatch"));
                }
                let definition = self
                    .structs
                    .get(&declaration)
                    .ok_or_else(|| error(span, "constant record descriptor is missing"))?;
                let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before=fuel;
                let types=nir::normal_forms::instantiate_value_fields(&definition.type_parameters,&arguments,
                    definition.fields.iter().map(|(_,ty)|ty),&mut fuel);
                self.required_steps-=(before-fuel) as u64;
                let types=types.ok_or_else(||error(span,"constant record layout is invalid or exceeds the materialization budget"))?;
                if fields.len() != types.len() {
                    return Err(error(span, "constant record field count mismatch"));
                }
                K::Record {
                    declaration,
                    fields: fields
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_expression(value, ty, span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                }
            }
            Value::Variant {
                arguments,
                declaration,
                index,
                fields,
            } => {
                if ty
                    != (Term::Nominal {
                        declaration,
                        arguments: arguments.clone(),
                    })
                {
                    return Err(error(span, "constant enum identity mismatch"));
                }
                let definition = self
                    .enums
                    .get(&declaration)
                    .ok_or_else(|| error(span, "constant enum descriptor is missing"))?;
                let variant = definition.variants
                    .get(index)
                    .ok_or_else(|| error(span, "constant enum variant is missing"))?;
                if fields.len() != variant.fields.len() {
                    return Err(error(span, "constant enum field count mismatch"));
                }
                let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before=fuel;
                let types=nir::normal_forms::instantiate_value_fields(&definition.type_parameters,&arguments,
                    variant.fields.iter(),&mut fuel);
                self.required_steps-=(before-fuel) as u64;
                let types=types.ok_or_else(||error(span,"constant enum layout is invalid or exceeds the materialization budget"))?;
                K::Variant {
                    declaration,
                    index,
                    fields: fields
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_expression(value, ty, span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                }
            }
            Value::Function(declaration) => K::Function(declaration),
            Value::RuntimeFunction { declaration, arguments } => K::FunctionInstance { declaration, arguments },
            Value::RuntimeTraitMethod(identity) => {
                let comptime_interpretor::TraitMethodIdentity { interface, subject, member, method_arguments } = *identity;
                K::TraitMethod { interface: interface.declaration, interface_arguments: interface.arguments, subject, member, method_arguments }
            },
            Value::Closure(closure) => {
                if let Some((declaration, arguments)) = closure.declaration_instance() {
                    K::FunctionInstance {
                        declaration,
                        arguments: arguments.to_vec(),
                    }
                } else {
                    let identity = std::sync::Arc::as_ptr(&closure) as usize;
                    if let Some((binding, previous_type)) = snapshot.closures.get(&identity) {
                        if previous_type != &ty {
                            return Err(error(span, "shared constant closure type mismatch"));
                        }
                        return Ok(n::Expr {
                            source: source(span),
                            ty,
                            kind: K::Local(*binding),
                        });
                    }
                    if !snapshot.active.insert(identity) {
                        return Err(error(
                            span,
                            "cyclic closure environment cannot be materialized as a constant",
                        ));
                    }
                    let types: &[Term] = match &ty {
                        Term::Closure { environment, .. } => environment,
                        Term::ComptimeFunction(_) => &[],
                        _ => {
                            return Err(error(
                                span,
                                "constant closure has no comptime callable type",
                            ));
                        }
                    };
                    if types.len() != closure.captured_values().len() {
                        return Err(error(span, "constant closure environment shape mismatch"));
                    }
                    let mut captures = Vec::new();
                    for ((binding, value), capture_type) in closure.captured_values().zip(types) {
                        // Only owned capture cells can be snapshotted. Pointers
                        // and free runtime storage remain invalid constants.
                        let value = if let Value::Storage(cell) = value {
                            cell.read().map_err(|e| {
                                error(span, format!("constant capture snapshot failed: {e:?}"))
                            })?
                        } else {
                            value.clone()
                        };
                        captures.push(n::Capture {
                            binding,
                            ty: capture_type.clone(),
                            value: Some(Box::new(self.constant_expression(
                                value,
                                capture_type.clone(),
                                span,
                                depth + 1,
                                snapshot,
                            )?)),
                        });
                    }
                    let index = self.next_temporary_binding;
                    self.next_temporary_binding = index
                        .checked_add(1)
                        .ok_or_else(|| error(span, "constant closure binding space exhausted"))?;
                    let binding = n::LocalId {
                        owner: u32::MAX,
                        index,
                    };
                    let value = n::Expr {
                        source: source(span),
                        ty: ty.clone(),
                        kind: K::Closure {
                            parameters: closure.parameters().to_vec(),
                            captures,
                            // The body is immutable, already elaborated and specialized.
                            body: closure.body().clone(),
                        },
                    };
                    snapshot.bindings.push(n::Expr {
                        source: source(span),
                        ty: p(P::Void),
                        kind: K::Let {
                            binding,
                            value: Box::new(value),
                        },
                    });
                    snapshot.active.remove(&identity);
                    snapshot.closures.insert(identity, (binding, ty.clone()));
                    K::Local(binding)
                }
            }
        };
        Ok(n::Expr {
            source: source(span),
            ty,
            kind,
        })
    }
}

pub(super) fn charge_copy(value: &n::Expr, steps: &mut u64, span: Span) -> Result<(), Error> {
    enum Node<'a> {
        Expr(&'a n::Expr),
        Type(&'a Term),
    }
    let mut work = vec![(Node::Expr(value), 0usize)];
    while let Some((node, depth)) = work.pop() {
        if depth >= 128 || *steps == 0 {
            return Err(error(span, "constant copy budget exhausted"));
        }
        *steps -= 1;
        let size = match node {
            Node::Expr(expr) => {
                work.push((Node::Type(&expr.ty), depth + 1));
                match &expr.kind {
                    K::Call {
                        callee,
                        arguments,
                        parameter_bindings,
                    } => {
                        work.push((Node::Expr(callee), depth + 1));
                        work.extend(
                            arguments
                                .iter()
                                .map(|argument| (Node::Expr(&argument.value), depth + 1)),
                        );
                        parameter_bindings.len()
                    }
                    K::Block { statements, result } | K::CallPreparation { statements, result } => {
                        work.extend(statements.iter().map(|expr| (Node::Expr(expr), depth + 1)));
                        if let Some(result) = result {
                            work.push((Node::Expr(result), depth + 1));
                        }
                        0
                    }
                    K::Let { value, .. } => {
                        work.push((Node::Expr(value), depth + 1));
                        0
                    }
                    K::Closure {
                        parameters,
                        captures,
                        ..
                    } => {
                        for capture in captures {
                            work.push((Node::Type(&capture.ty), depth + 1));
                            if let Some(value) = &capture.value {
                                work.push((Node::Expr(value), depth + 1));
                            }
                        }
                        // Expr::clone shares the immutable Arc body. Captured
                        // initializers and parameter bindings are actual copies.
                        parameters.len()
                    }
                    K::FunctionInstance { arguments, .. } => {
                        work.extend(arguments.iter().map(|ty| (Node::Type(ty), depth + 1)));
                        0
                    }
                    K::TraitMethod { subject, interface_arguments, method_arguments, .. } => {
                        work.push((Node::Type(subject), depth + 1));
                        work.extend(interface_arguments.iter().chain(method_arguments).map(|ty| (Node::Type(ty), depth + 1)));
                        0
                    }
                    K::TypeValue(ty) => {
                        work.push((Node::Type(ty), depth + 1));
                        0
                    }
                    K::TypeListValue(types) => {
                        work.extend(types.iter().map(|ty| (Node::Type(ty), depth + 1)));
                        0
                    }
                    K::Tuple(items)
                    | K::AnonymousRecord { fields: items } | K::Record { fields: items, .. }
                    | K::Variant { fields: items, .. } => {
                        work.extend(items.iter().map(|e| (Node::Expr(e), depth + 1)));
                        0
                    }
                    K::Literal(n::Literal::Symbol(s) | n::Literal::String(s) | n::Literal::Integer(s)) => s.len(),
                    K::Literal(n::Literal::BitVec { words, .. }) => words.len(),
                    _ => 0,
                }
            }
            Node::Type(ty) => match ty {
                Term::AssociatedType { arguments, subject, .. } => {
                    work.push((Node::Type(subject), depth + 1));
                    work.extend(arguments.iter().map(|t| (Node::Type(t), depth + 1)));
                    0
                }
                Term::NominalValue {ty,fields,..} => {work.push((Node::Type(ty),depth+1));work.extend(fields.iter().map(|field|(Node::Type(field),depth+1)));0},
                Term::Record(fields) | Term::RecordValue(fields) => {
                    work.extend(fields.iter().map(|(_, ty)| (Node::Type(ty), depth + 1)));
                    fields.iter().map(|(name, _)| name.len()).sum()
                }
                Term::Tuple(items)
                | Term::TypeListValue(items) | Term::TypeListConcat(items) | Term::TupleValue(items)
                | Term::Nominal {
                    arguments: items, ..
                }
                | Term::NormalForm {
                    arguments: items, ..
                } => {
                    work.extend(items.iter().map(|t| (Node::Type(t), depth + 1)));
                    0
                }
                Term::Pointer(t) | Term::ComptimeFunction(t) => {
                    work.push((Node::Type(t), depth + 1));
                    0
                }
                Term::Function { parameters, result } => {
                    work.push((Node::Type(result), depth + 1));
                    work.extend(parameters.iter().map(|p| (Node::Type(&p.ty), depth + 1)));
                    0
                }
                Term::Closure {
                    environment,
                    signature,
                } => {
                    work.push((Node::Type(signature), depth + 1));
                    work.extend(environment.iter().map(|t| (Node::Type(t), depth + 1)));
                    0
                }
                Term::String(s) | Term::Symbol(s) => s.len(),
                Term::Integer(n) => n.bit_len().div_ceil(32),
                Term::BitVec(v) => v.words().len(),
                _ => 0,
            },
        };
        *steps = steps
            .checked_sub(size as u64)
            .ok_or_else(|| error(span, "constant copy budget exhausted"))?;
        if work.len() as u64 > *steps {
            return Err(error(span, "constant copy budget exhausted"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_value_copies_are_bounded() {
        let value = n::Expr {
            source: source(Span::default()),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("λ".into())),
        };
        assert!(charge_copy(&value, &mut 3, Span::default()).is_err());
        let mut steps = 4;
        charge_copy(&value, &mut steps, Span::default()).unwrap();
        assert_eq!(steps, 0);
    }
    #[test]
    fn graph_snapshot_copy_budget_visits_bound_values() {
        let binding = n::LocalId {
            owner: u32::MAX,
            index: 0,
        };
        let span = Span::default();
        let value = n::Expr {
            source: source(span),
            ty: p(P::Str),
            kind: K::Block {
                statements: vec![n::Expr {
                    source: source(span),
                    ty: p(P::Void),
                    kind: K::Let {
                        binding,
                        value: Box::new(n::Expr {
                            source: source(span),
                            ty: p(P::Str),
                            kind: K::Literal(n::Literal::String("x".repeat(128))),
                        }),
                    },
                }],
                result: Some(Box::new(n::Expr {
                    source: source(span),
                    ty: p(P::Str),
                    kind: K::Local(binding),
                })),
            },
        };
        assert!(charge_copy(&value, &mut 64, span).is_err());
        charge_copy(&value, &mut 256, span).unwrap();
    }

    #[test]
    fn callable_copy_budget_includes_identity_arguments_absent_from_signature() {
        let span = Span::default();
        let member = DeclarationId { package: 1, local: 1 };
        for large_slot in 0..3 {
            let mut types = vec![p(P::Bool); 3];
            types[large_slot] = Term::Tuple(vec![p(P::Bool); 128]);
            let value = n::Expr {
                source: source(span),
                ty: Term::Function { parameters: vec![], result: Box::new(p(P::Bool)) },
                kind: K::TraitMethod {
                    interface: member, member,
                    subject: types.remove(0),
                    interface_arguments: vec![types.remove(0)],
                    method_arguments: types,
                },
            };
            assert!(charge_copy(&value, &mut 64, span).is_err());
            charge_copy(&value, &mut 512, span).unwrap();
        }
    }

    #[test]
    fn associated_copy_budget_visits_subject_and_interface_inputs() {
        for subject_large in [false, true] {
            let large = Term::Tuple(vec![p(P::Bool); 128]);
            let ty = Term::AssociatedType {
                origin: Box::new(nir::AssociatedTypeOrigin { interface: DeclarationId { package: 1, local: 1 },
                member: DeclarationId { package: 1, local: 2 } }),
                subject: Box::new(if subject_large { large.clone() } else { p(P::Bool) }),
                arguments: if subject_large { vec![] } else { vec![large] },
            };
            let value = n::Expr { source: source(Span::default()), ty: p(P::Type), kind: K::TypeValue(ty.clone()) };
            assert!(charge_copy(&value, &mut 64, Span::default()).is_err());
            charge_copy(&value, &mut 512, Span::default()).unwrap();
            let mut budget = comptime_runtime::Budget { steps: 64, ..Default::default() };
            assert!(super::super::normal::clone_known_term(&ty, &mut budget).is_none());
        }
    }
}
