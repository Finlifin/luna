//! Contextual inputs consumed by a comptime construction program whose result
//! is ordinary data. Calls execute only at required construction boundaries.
use super::*;
use comptime_interpretor::Value;
use statements::{Flow, ResultInterface};

impl Checker<'_, '_> {
    pub(in crate::program) fn is_data_consumer(&self, owner: OwnerId) -> bool {
        self.package.item(owner).is_some_and(|item| {
            let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
                return false;
            };
            !sig.return_ty.is_some_and(|ty| {
                matches!(
                    ty.kind,
                    H::ContextualExprType(..) | H::ContextualPatternType(..)
                )
            }) && sig.params.iter().any(|param| match param.ty.kind {
                TyParamKind::Positional(ty)
                | TyParamKind::Optional(_, ty, _)
                | TyParamKind::Varadic(_, ty) => matches!(
                    ty.kind,
                    H::ContextualExprType(..) | H::ContextualPatternType(..)
                ),
                _ => false,
            })
        })
    }

    pub(in crate::program) fn ensure_data_consumer(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        self.validate_comptime_receiver(owner)?;
        if self.data_consumers.contains_key(&owner) {
            return Ok(());
        }
        let definition = self.form_contextual_generator(owner, span, |this, output| {
            output
                .map(|output| this.type_expr(output))
                .unwrap_or_else(|| Ok(p(P::Void)))
        })?;
        self.data_consumers.insert(owner, definition);
        Ok(())
    }

    pub(in crate::program) fn data_method_contract(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(CodeFunctionSignature<Term>, nir::contracts::ProgramContract), Error> {
        self.ensure_data_consumer(owner, span)?;
        let definition = self.copy_data_consumer(owner, span)?;
        let program =
            self.contextual_method_contract(owner, &definition, Some(&definition.output), span)?;
        Ok((definition.signature, program))
    }

    fn copy_data_consumer(&mut self, owner: OwnerId, span: Span) -> Result<Generator<Term>, Error> {
        let definition = &self.data_consumers[&owner];
        Self::charge_code_generator(definition, &mut self.required_steps, span)?;
        let definition = definition.clone();
        self.apply_generator_captures(owner, definition, span)
    }

    pub(in crate::program) fn check_data_consumer(&mut self, owner: OwnerId) -> Result<(), Error> {
        let span = self.package.item(owner).unwrap().span;
        let definition = &self.data_consumers[&owner];
        if definition.checked {
            return Ok(());
        }
        Self::charge_code_generator(definition, &mut self.required_steps, span)?;
        let definition = definition.clone();
        let metadata = self.code_storage_metadata();
        for (_, input) in &definition.inputs {
            if let Input::Code(interface) = input {
                for ty in interface
                    .context
                    .values()
                    .chain(std::iter::once(&interface.result))
                {
                    nir::phases::validate_runtime_storage(ty, &metadata, &[]).map_err(
                        |message| error(span, format!("quoted runtime interface: {message}")),
                    )?;
                }
            }
        }
        let arguments = definition
            .inputs
            .iter()
            .map(|(_, input)| {
                Ok(Some(match input {
                    Input::Data(_) => Argument::Data(None),
                    Input::Code(interface) => {
                        Argument::Code(CodeValue::Expression(Arc::new(ExpressionCode {
                            body: None,
                            result: interface.result.clone(),
                            context: interface.context.clone(),
                            ports: interface.ports.clone(),
                            defaults: BTreeMap::new(),
                        })))
                    }
                    Input::Pattern(interface) => Argument::Code(CodeValue::Pattern(
                        self.abstract_pattern_input(interface, span)?,
                    )),
                }))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let checking = std::mem::replace(&mut self.checking_code_generator, true);
        let result = self.generator_frame(owner, |this| {
            this.install_generator_capture_values(&definition.captures, span)?;
            this.check_generator_clause_value_defaults(owner, &definition.types)?;
            this.run_data_consumer(&definition, arguments)
        });
        self.checking_code_generator = checking;
        result?;
        self.data_consumers.get_mut(&owner).unwrap().checked = true;
        Ok(())
    }

    pub(in crate::program) fn data_consumer_call(&mut self, value: &Expr<'_>) -> Option<OwnerId> {
        let (H::Application(callee, _) | H::ExtendedApplication(callee, _)) = value.kind else {
            return None;
        };
        let callee = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        let definition = match callee.kind {
            H::ResolvedDef { def_id, .. } => Some(def_id),
            H::Projection(..) => self.package.member_projections.get(&callee.hir_id).copied(),
            _ => None,
        };
        definition
            .and_then(|id| self.package.definition_owner(id))
            .filter(|owner| self.is_data_consumer(*owner))
            .or_else(|| {
                self.generic_code_method_hint(callee)
                    .filter(|owner| self.is_data_consumer(*owner))
            })
            .or_else(|| {
                self.static_code_method_hint(callee)
                    .filter(|owner| self.is_data_consumer(*owner))
            })
    }

    #[inline(never)]
    pub(in crate::program) fn required_data_consumer(
        &mut self,
        site: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<(Value, Term), Error> {
        let (value, ty) = self.invoke_data_consumer(site, expected)?;
        value
            .map(|value| (value, ty))
            .ok_or_else(|| error(site.span, "contextual data call requires completed inputs"))
    }

    // This is deliberately not called from ordinary expression elaboration:
    // required construction evaluates the call when its statement is reached.
    pub(in crate::program) fn invoke_data_consumer(
        &mut self,
        site: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<(Option<Value>, Term), Error> {
        // Include input preparation: deeply nested calls in arguments also
        // consume native stack before their callee frames have been installed.
        if self.data_consumer_depth >= 8 {
            return Err(error(
                site.span,
                "contextual data invocation depth exhausted",
            ));
        }
        self.data_consumer_depth += 1;
        let result = (|| {
            let (H::Application(callee, arguments) | H::ExtendedApplication(callee, arguments)) =
                site.kind
            else {
                return Err(error(site.span, "expected contextual data call"));
            };
            self.with_code_method_callee(callee, |this, callee, receiver| {
                if receiver.is_some() {
                    return Err(error(
                        site.span,
                        "contextual data receiver invocation requires value-reference dispatch not yet implemented",
                    ));
                }
                let selected = Expr {
                    kind: if matches!(site.kind, H::ExtendedApplication(..)) {
                        H::ExtendedApplication(callee, arguments)
                    } else {
                        H::Application(callee, arguments)
                    },
                    ..site.clone()
                };
                this.prepare_and_invoke_data_consumer(&selected, expected)
            })
        })();
        self.data_consumer_depth -= 1;
        result
    }

    fn prepare_and_invoke_data_consumer(
        &mut self,
        site: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<(Option<Value>, Term), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(site.span, "contextual data invocation budget exhausted"))?;
        let owner = self
            .data_consumer_call(site)
            .ok_or_else(|| error(site.span, "expected contextual data call"))?;
        self.ensure_data_consumer(owner, site.span)?;
        let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            &self.package.item(owner).unwrap().kind
        else {
            unreachable!()
        };
        if sig
            .params
            .iter()
            .any(|param| matches!(param.ty.kind, TyParamKind::Itself { .. }))
        {
            return Err(error(
                site.span,
                "contextual data receiver invocation requires value-reference dispatch not yet implemented",
            ));
        }
        let template = self.copy_data_consumer(owner, site.span)?;
        let (H::Application(callee, arguments) | H::ExtendedApplication(callee, arguments)) =
            site.kind
        else {
            unreachable!()
        };
        let (definition, values) = if matches!(site.kind, H::ExtendedApplication(..))
            && arguments
                .iter()
                .any(|argument| matches!(argument, Arg::Named(..)))
        {
            let (collector, plan) = self.contextual_property_plan(
                owner,
                &template,
                site,
                callee,
                expected,
                Term::clone,
            )?;
            self.with_generator_properties(site, arguments, collector, |this, inputs| {
                this.prepare_data_consumer_inputs(
                    owner,
                    template,
                    site,
                    callee,
                    inputs,
                    expected,
                    Some(plan),
                )
            })?
        } else {
            self.prepare_data_consumer_inputs(
                owner, template, site, callee, arguments, expected, None,
            )?
        };
        if let Some(expected) = expected {
            self.require(&definition.output, expected, site.span)?;
        }
        if self.checking_code_generator {
            return Ok((None, definition.output.clone()));
        }
        let result =
            self.generator_frame(owner, |this| this.run_data_consumer(&definition, values))?;
        Ok((result, definition.signature.output))
    }

    fn prepare_data_consumer_inputs(
        &mut self,
        owner: OwnerId,
        template: Generator<Term>,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Term>,
        type_plan: Option<GeneratorTypePlan>,
    ) -> Result<(Generator<Term>, Vec<Option<Argument>>), Error> {
        let result_context = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == site.hir_id)
            .map(|(_, context)| context.clone());
        let prepared = if arguments
            .iter()
            .any(|argument| matches!(argument, Arg::Expand(_)))
        {
            self.with_generator_spreads(owner, site.span, arguments, |this, inputs, packets| {
                this.prepare_contextual_inputs(
                    owner,
                    template,
                    site,
                    callee,
                    inputs,
                    expected.cloned(),
                    Term::clone,
                    result_context.as_ref(),
                    packets,
                    type_plan,
                )
            })?
        } else {
            self.prepare_contextual_inputs(
                owner,
                template,
                site,
                callee,
                arguments,
                expected.cloned(),
                Term::clone,
                result_context.as_ref(),
                &mut spreads::SpreadPlan::default(),
                type_plan,
            )?
        };
        Ok(prepared)
    }

    pub(in crate::program) fn check_data_consumer_expression(
        &mut self,
        site: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if !self.checking_code_generator {
            return Err(error(
                site.span,
                "contextual data call requires a direct comptime binding or construction statement; nested ordinary expression scheduling is not yet implemented",
            ));
        }
        let (_, ty) = self.invoke_data_consumer(site, expected)?;
        let id = self.spread_binding(site.span)?;
        self.locals.insert(id, ty.clone());
        self.generator_data.insert(id, None);
        Ok(n::Expr {
            ty,
            kind: K::Local(id),
            source: source(site.span),
        })
    }

    fn run_data_consumer(
        &mut self,
        definition: &Generator<Term>,
        arguments: Vec<Option<Argument>>,
    ) -> Result<Option<Value>, Error> {
        let owner = self.signature_owner.expect("contextual consumer frame");
        let item = self.package.item(owner).unwrap();
        let (sig, body) = match &item.kind {
            ItemKind::Fn(sig, body) => (sig, Some(*body)),
            ItemKind::FnDeclaration(sig) => (sig, None),
            _ => unreachable!(),
        };
        self.install_generator_capture_values(&definition.captures, item.span)?;
        self.alias_substitution.extend(
            definition
                .types
                .iter()
                .map(|(id, value)| (*id, Arc::new(value.clone()))),
        );
        self.install_generator_clause_values(owner, &definition.types, &definition.clause_domains)?;
        for (slot, ((id, input), argument)) in definition.inputs.iter().zip(arguments).enumerate() {
            let default = match sig.params[slot].ty.kind {
                TyParamKind::Optional(_, _, value) => Some(value),
                _ => None,
            };
            let supplied = if self.checking_code_generator || argument.is_none() {
                default
                    .map(|value| self.generator_default(value, input))
                    .transpose()?
            } else {
                None
            };
            let value = argument
                .or(supplied)
                .ok_or_else(|| error(sig.params[slot].span, "missing contextual data argument"))?;
            self.immutable.insert(*id);
            self.install_generator_type_input(*id, definition.input_value(slot));
            match (input, value) {
                (Input::Data(ty), Argument::Data(value)) => {
                    self.locals.insert(*id, ty.clone());
                    self.generator_data.insert(*id, value);
                }
                (Input::Data(ty), Argument::LiveData(value)) => {
                    self.install_live_generator_input(*id, ty, value, sig.params[slot].span)
                }
                (Input::Code(interface), Argument::Code(CodeValue::Expression(code))) => {
                    let code = self.ascribe_code(code, Some(interface), sig.params[slot].span)?;
                    self.code_values.insert(*id, code);
                }
                (Input::Pattern(interface), Argument::Code(CodeValue::Pattern(code))) => {
                    self.ascribe_pattern_input(&code, interface, sig.params[slot].span)?;
                    self.pattern_codes.insert(*id, code);
                }
                _ => {
                    return Err(error(
                        sig.params[slot].span,
                        "contextual data argument category mismatch",
                    ));
                }
            }
        }
        self.generator_contracts(owner, false, sig.span, None)?;
        let entry = self.capture_generator_entry_data(owner, sig.span)?;
        let result_binding = sig.return_binding.map(local);
        if self.checking_code_generator {
            self.generator_contracts_with_result(
                owner,
                true,
                sig.span,
                Some(&entry),
                result_binding.map(|binding| (binding, &definition.output)),
            )?;
        }
        let control = if !self.checking_code_generator
            && let Some(target) = self.code_method_targets.get(&owner).copied()
        {
            self.ensure_data_consumer(target, sig.span)?;
            let template = self.copy_data_consumer(target, sig.span)?;
            let (selected, arguments) =
                self.prepare_selected_contextual_method(target, definition, &template, sig.span)?;
            self.require(&selected.output, &definition.output, sig.span)?;
            let result =
                self.generator_frame(target, |this| this.run_data_consumer(&selected, arguments))?;
            Control::DataReturn(result)
        } else {
            let Some(body) = body else {
                return if self.checking_code_generator {
                    Ok(None)
                } else {
                    Err(error(
                        item.span,
                        "contextual data consumer has no selected body",
                    ))
                };
            };
            let value = self.package.body(body).unwrap().value;
            self.consumer_result(value, &definition.output, 0)?
        };
        if self.checking_code_generator {
            let flow = control.flow();
            if flow.next || flow.has_transfer() {
                return Err(error(
                    sig.span,
                    "contextual data function can finish without returning its result",
                ));
            }
            Ok(None)
        } else {
            let Control::DataReturn(Some(value)) = control else {
                return Err(error(
                    sig.span,
                    "contextual data function did not return a value",
                ));
            };
            if let Some(binding) =
                result_binding.filter(|_| !self.program_postconditions[&owner].is_empty())
            {
                // Expose the completed result only to the postcondition frame.
                // Owned storage retains nested identities; reading it back uses
                // the same metered representation path as other generator cells.
                self.install_live_generator_input(binding, &definition.output, value, sig.span);
                self.generator_contracts_with_result(
                    owner,
                    true,
                    sig.span,
                    Some(&entry),
                    Some((binding, &definition.output)),
                )?;
                let cell = self.generator_cells[&binding].1.clone();
                Ok(Some(self.read_generator_cell(&cell, sig.span)?))
            } else {
                self.generator_contracts(owner, true, sig.span, Some(&entry))?;
                Ok(Some(value))
            }
        }
    }

    pub(super) fn consumer_result(
        &mut self,
        value: &Expr<'_>,
        ty: &Term,
        depth: usize,
    ) -> Result<Control, Error> {
        if depth >= 64 {
            return Err(error(
                value.span,
                "contextual data result nesting exhausted",
            ));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(value.span, "contextual data construction budget exhausted"))?;
        match value.kind {
            H::Block(block) => self.consumer_block(block, ty, depth + 1),
            H::InlineControl(inner) => self.consumer_result(inner, ty, depth + 1),
            H::If(condition, yes, no) => {
                let selected = self.generator_when_condition(Some(condition))?;
                if self.checking_code_generator {
                    let yes = self.consumer_block(yes, ty, depth + 1)?;
                    let no = match no {
                        Some(no) => self.consumer_result(no, ty, depth + 1)?,
                        None => Control::Next,
                    };
                    Ok(Control::Abstract(match selected {
                        Some(true) => yes.flow(),
                        Some(false) => no.flow(),
                        None => self.join_generator_flows(yes.flow(), no.flow(), value.span)?,
                    }))
                } else if selected == Some(true) {
                    self.consumer_block(yes, ty, depth + 1)
                } else if let Some(no) = no {
                    self.consumer_result(no, ty, depth + 1)
                } else {
                    Ok(Control::Next)
                }
            }
            H::When(arms) => {
                let mut flow = Flow::default();
                let mut pending = true;
                for arm in arms {
                    let selected = self.generator_when_condition(arm.cond)?;
                    if self.checking_code_generator {
                        let control = self.generator_scope(arm.span, |this| {
                            this.consumer_result(arm.body, ty, depth + 1)
                        })?;
                        if pending && selected != Some(false) {
                            flow = self.join_generator_flows(flow, control.flow(), arm.span)?;
                        }
                        if selected == Some(true) {
                            pending = false;
                        }
                    } else if selected == Some(true) {
                        return self.generator_scope(arm.span, |this| {
                            this.consumer_result(arm.body, ty, depth + 1)
                        });
                    }
                }
                flow.next |= pending;
                Ok(if self.checking_code_generator {
                    Control::Abstract(flow)
                } else {
                    Control::Next
                })
            }
            H::Match(subject, arms) => {
                let controls =
                    self.generator_match(value, subject, arms, None, |this, _, body| {
                        this.consumer_result(body, ty, depth + 1)
                    })?;
                if self.checking_code_generator {
                    let mut flow = Flow::default();
                    for control in controls {
                        flow = self.join_generator_flows(flow, control.flow(), value.span)?;
                    }
                    Ok(Control::Abstract(flow))
                } else {
                    controls
                        .into_iter()
                        .next()
                        .ok_or_else(|| error(value.span, "contextual data match selected no arm"))
                }
            }
            H::Return(..)
            | H::Loop(_)
            | H::For { .. }
            | H::Semi(_)
            | H::Let(_)
            | H::Assign(..)
            | H::AssignOp(..) => self.generator_statement_with_result(
                value,
                ResultInterface::Data(ty),
                depth + 1,
                &mut Vec::new(),
            ),
            _ if self.checking_code_generator => {
                let checked = self.check_generator_data_input(value, Some(ty), None)?;
                self.require(&checked.ty, ty, value.span)?;
                Ok(Control::DataReturn(None))
            }
            _ => {
                let (result, actual) = self.generator_data_expression(value, Some(ty))?;
                self.require(&actual, ty, value.span)?;
                Ok(Control::DataReturn(Some(result)))
            }
        }
    }

    fn consumer_block(
        &mut self,
        block: &hir::Block<'_>,
        ty: &Term,
        depth: usize,
    ) -> Result<Control, Error> {
        self.generator_scope(block.span, |this| {
            let mut control = Control::Next;
            let mut loops = Vec::new();
            for statement in block.stmts {
                let next = this.generator_statement_with_result(
                    statement,
                    ResultInterface::Data(ty),
                    depth + 1,
                    &mut loops,
                )?;
                if this.checking_code_generator {
                    control = this.sequence_generator_control(control, next, statement.span)?;
                } else if !matches!(next, Control::Next) {
                    return Ok(next);
                }
            }
            if let Some(tail) = block.expr {
                let next = this.consumer_result(tail, ty, depth + 1)?;
                if this.checking_code_generator {
                    control = this.sequence_generator_control(control, next, tail.span)?;
                } else {
                    control = next;
                }
            }
            if *ty == p(P::Void) {
                let unit =
                    Control::DataReturn((!this.checking_code_generator).then_some(Value::Unit));
                if this.checking_code_generator {
                    control = this.sequence_generator_control(control, unit, block.span)?;
                } else if matches!(control, Control::Next) {
                    control = unit;
                }
            }
            Ok(control)
        })
    }
}
