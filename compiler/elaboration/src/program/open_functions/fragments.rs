//! Definition-side validation of a checked region in its compacted input frame.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn check_open_fragment(
        &mut self,
        value: &n::Expr,
        span: Span,
        arithmetic: bool,
    ) -> Result<(), Error> {
        if !self.open_loop_transfers.is_empty() {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let checked = (|| {
                for (target, found) in &mut self.open_loop_transfers {
                    if !found.breaks {
                        found.breaks = control::loop_may_complete(value, *target, &mut fuel)?;
                    }
                    if !found.continues {
                        found.continues = control::loop_may_continue(value, *target, &mut fuel)?;
                    }
                }
                Ok::<_, String>(())
            })();
            self.required_steps -= (before - fuel) as u64;
            checked.map_err(|message| error(span, message))?;
        }
        let owner = self.signature_owner.unwrap();
        let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            &self.package.item(owner).unwrap().kind
        else {
            unreachable!()
        };
        let mut staged_locals = Vec::new();
        if self.in_staged_input {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let free = nir::free_bindings::collect(value, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            for binding in free.map_err(|message| error(span, message))? {
                // All inputs of a comptime function are available during its
                // invocation, including ordinary parameters used by defaults.
                // They already belong to the fragment's parameter telescope.
                if sig.modifiers.is_comptime
                    && sig
                        .params
                        .iter()
                        .any(|parameter| local(parameter.hir_id) == binding)
                {
                    continue;
                }
                if self.required_runtime_parameter(binding) {
                    return Err(error(
                        span,
                        "required comptime computation cannot read a runtime parameter",
                    ));
                }
                if !self.input_facts.types.contains_key(&binding)
                    && !self.input_facts.values.contains_key(&binding)
                {
                    if self.open_staged_locals.contains(&binding) {
                        let ty = self
                            .locals
                            .get(&binding)
                            .ok_or_else(|| error(span, "staged local has no established domain"))?;
                        staged_locals.push((binding, ty.clone()));
                    } else {
                        return Err(error(
                            span,
                            format!(
                                "required comptime computation cannot read an unstaged local {binding}"
                            ),
                        ));
                    }
                }
            }
        }
        let definition = &self.open_functions[&owner];
        let mut parameters = Vec::new();
        let mut bindings = Vec::new();
        let mut mapping = vec![None; definition.parameters.len()];
        for (slot, (parameter, (sort, annotation))) in
            sig.params.iter().zip(&definition.parameters).enumerate()
        {
            // A staged fragment of a runtime function cannot read runtime
            // inputs (checked above), nor store them as synthetic comptime
            // arguments merely because they occur in the enclosing signature.
            if self.in_staged_input && !sig.modifiers.is_comptime && *sort == Sort::Runtime {
                continue;
            }
            if let Annotation::Known(ty) = annotation {
                mapping[slot] = Some(Term::Bound {
                    depth: 0,
                    index: parameters.len(),
                    sort: *sort,
                });
                parameters.push(Parameter {
                    sort: *sort,
                    ty: ty.clone(),
                });
                bindings.push(local(parameter.hir_id));
            }
        }
        // Locally computed staged values are abstract inputs only to this
        // validation fragment. The suspended source, not this synthetic NIR,
        // is used to resume checking and execution with real bindings.
        for (binding, ty) in staged_locals {
            bindings.push(binding);
            parameters.push(Parameter {
                sort: Sort::ComptimeValue,
                ty,
            });
        }
        // Abstract inline selectors belong only to this definition-side
        // fragment. They are never runtime arguments of the source function.
        for (binding, ty) in &self.ordinary_behavior_inputs {
            if !bindings.contains(binding) {
                bindings.push(*binding);
                parameters.push(Parameter {
                    sort: Sort::ComptimeValue,
                    ty: ty.clone(),
                });
            }
        }
        // The fragment excludes unknown runtime domains. Every remaining type
        // reference must cross the same compaction map, including contracts and
        // operands nested in trait method values or callable signatures.
        for parameter in &mut parameters {
            parameter.ty = parameter
                .ty
                .rebind_scope_partial(&mapping)
                .map_err(|failure| error(span, failure.to_string()))?;
        }
        constants::charge_copy(value, &mut self.required_steps, span)?;
        let mut value = value.clone();
        if !self.in_staged_input {
            if let Some(effects) = &self.effect_environment {
                value = n::Expr {
                    source: value.source,
                    ty: Term::computation(effects.clone(), value.ty.clone()),
                    kind: K::EffectComputation {
                        effects: effects.clone(),
                        body: Box::new(value),
                    },
                };
            }
        }
        value
            .rewrite_types(&mut |ty| ty.rebind_scope_partial(&mapping))
            .map_err(|failure| error(span, failure.to_string()))?;
        let requirements = self
            .definition_requirements(owner)
            .iter()
            .map(|requirement| {
                required::map_requirement(
                    requirement,
                    &mut |ty| {
                        ty.rebind_scope_partial(&mapping)
                            .map_err(|failure| error(span, failure.to_string()))
                    },
                    span,
                    0,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let capabilities = self
            .capabilities
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|requirement| {
                Ok(n::TraitRequirement {
                    interface: requirement.interface,
                    subject: requirement
                        .subject
                        .rebind_scope_partial(&mapping)
                        .map_err(|failure| error(span, failure.to_string()))?,
                    interface_arguments: requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| {
                            ty.rebind_scope_partial(&mapping)
                                .map_err(|failure| error(span, failure.to_string()))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let (mut functions, runtime_function_identities) = self.fragment_callables(&value, span)?;
        functions.push(n::Function {
            interface: nir::program::FunctionDeclaration {
                declaration: DeclarationId {
                    package: u64::MAX,
                    local: u32::MAX,
                },
                name: "<ordinary-deferred-region>".into(),
                phase: if self.in_staged_input || sig.modifiers.is_comptime {
                    n::FunctionPhase::Comptime
                } else {
                    n::FunctionPhase::Runtime
                },
                inline: false,
                pure: false,
                is_unsafe: false,
                signature: Term::Function {
                    parameters,
                    result: Box::new(value.ty.clone()),
                },
                parameters: bindings,
                defaults: vec![],
                type_parameters: vec![],
                preconditions: vec![],
                postconditions: vec![],
                entry_snapshots: vec![],
                result_binding: None,
                requirements,
                capabilities,
            },
            body: value,
        });
        let package = n::Package {
            target: self.target,
            functions,
            runtime_function_identities,
            normal_forms: self.normal_form_metadata(),
            function_parameters: self.function_parameter_metadata(),
            structs: self.structs.values().cloned().collect(),
            newtypes: self.newtypes.values().cloned().collect(),
            alias_contracts: self.alias_contracts.values().cloned().collect(),
            enums: self.enums.values().cloned().collect(),
            traits: self.checked_traits.to_vec(),
            ..Default::default()
        };
        if arithmetic {
            crate::verification::flow::validate_machine_operations(&package).map_err(
                |mut failure| {
                    failure.source = failure
                        .source
                        .original(&self.expansion_sources)
                        .unwrap_or(failure.source);
                    error(span, failure.to_string())
                },
            )
        } else {
            nir::phases::validate(&package).map_err(|failure| error(span, failure.message))
        }
    }

    fn fragment_callables(
        &mut self,
        root: &n::Expr,
        span: Span,
    ) -> Result<(Vec<n::Function>, Vec<n::RuntimeFunctionIdentity>), Error> {
        let mut functions = Vec::new();
        let mut identities = Vec::new();
        // The phase pass inspects these method contracts as well as the fragment.
        // Their references are roots even if the fragment never calls that method.
        let contracts = self
            .checked_traits
            .iter()
            .flat_map(|interface| &interface.methods)
            .flat_map(|method| {
                method
                    .program
                    .preconditions
                    .iter()
                    .chain(&method.program.postconditions)
            });
        let mut pending = Vec::new();
        for expression in std::iter::once(root).chain(contracts) {
            pending.extend(
                required::references(expression, &mut self.required_steps)
                    .map_err(|failure| error(span, failure.to_string()))?
                    .into_iter()
                    .filter_map(required::Dependency::callable),
            );
        }
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let function = if let Some(function) = self.generated_functions.body(id) {
                // A completed body already carries its signature and program contracts.
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let charged =
                    nir::copy_budget::charge_function_substitution(function, &[], &[], &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                charged.map_err(|_| {
                    error(span, "ordinary region function snapshot budget exhausted")
                })?;
                function.clone()
            } else {
                let Some(owner) = self.source_declarations.owner(id) else {
                    continue;
                };
                match self.package.item(owner).map(|item| &item.kind) {
                    Some(ItemKind::Fn(sig, _))
                        if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head =>
                    {
                        self.required_function(owner, span)?
                    }
                    Some(ItemKind::Fn(sig, _)) if !sig.modifiers.is_normal_form_head => {
                        self.ensure_signature(owner, span)?;
                        if let Some(signature) = self.known_callable_signature(owner) {
                            identities.push(n::RuntimeFunctionIdentity {
                                declaration: id,
                                signature: self.callable_type(owner, signature),
                                type_parameters: self.type_parameters[&owner].clone(),
                            });
                        }
                        continue;
                    }
                    _ => {
                        self.ensure_signature(owner, span)?;
                        continue;
                    }
                }
            };
            pending.extend(
                required::function_references(&function, &mut self.required_steps)
                    .map_err(|failure| error(span, failure.to_string()))?
                    .into_iter()
                    .filter_map(required::Dependency::callable),
            );
            functions.push(function);
        }
        Ok((functions, identities))
    }
}
