//! Shared lexical input plans for declaration formation and caller supply.
use super::*;
mod dependencies;

/// The implicit prefix of a local declaration's clause telescope. This is
/// checker-local metadata over immutable HIR, not a persisted dependency summary.
/// Select from the complete lexical scope using declaration-rooted reads,
/// referenced declarations, inherited premises and input-domain prerequisites.
#[derive(Clone, Default)]
pub(super) struct LexicalInputPlan {
    inputs: Vec<LexicalInput>,
}

#[derive(Clone)]
struct LexicalInput {
    declaration: DeclarationId,
    sort: Sort,
    supply: LexicalSupply,
}

#[derive(Clone)]
enum LexicalSupply {
    Declaration,
    Staged(hir::HirId),
}

impl LexicalInputPlan {
    pub(super) fn declarations(&self) -> impl Iterator<Item = DeclarationId> + '_ {
        self.inputs.iter().map(|input| input.declaration)
    }
}

impl std::fmt::Display for LexicalInputPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(lexical-input-plan")?;
        for input in &self.inputs {
            write!(
                f,
                " (input {} (sort {}) (supply ",
                input.declaration, input.sort
            )?;
            match input.supply {
                LexicalSupply::Declaration => f.write_str("declaration")?,
                LexicalSupply::Staged(binding) => write!(f, "(staged {})", local(binding))?,
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_input_dump_distinguishes_supply_frames() {
        let plan = LexicalInputPlan {
            inputs: vec![
                LexicalInput {
                    declaration: DeclarationId {
                        package: 3,
                        local: 4,
                    },
                    sort: Sort::Type,
                    supply: LexicalSupply::Declaration,
                },
                LexicalInput {
                    declaration: DeclarationId {
                        package: 3,
                        local: 5,
                    },
                    sort: Sort::ComptimeValue,
                    supply: LexicalSupply::Staged(hir::HirId {
                        owner: OwnerId::new(hir::LocalDefId::new(8)),
                        local_id: hir::ItemLocalId::new(2),
                    }),
                },
            ],
        };
        assert_eq!(
            plan.to_string(),
            "(lexical-input-plan \
             (input (declaration (package 3) (local 4)) (sort type) (supply declaration)) \
             (input (declaration (package 3) (local 5)) (sort comptime-value) \
             (supply (staged (local 8 2)))))"
        );
        assert_eq!(
            LexicalInputPlan::default().to_string(),
            "(lexical-input-plan)"
        );
    }
}

impl Checker<'_, '_> {
    /// Pending domains may be excluded only after a complete source dependency
    /// observation. All outcomes debit the caller's shared preparation budget.
    pub(super) fn annotation_may_use_pending_inputs(
        &mut self,
        value: &Expr<'_>,
        inputs: &BTreeSet<n::LocalId>,
    ) -> Result<bool, Error> {
        if inputs.is_empty() {
            return Ok(false);
        }
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = dependencies::expression_may_use_inputs(
            self,
            self.signature_owner
                .ok_or_else(|| error(value.span, "annotation has no definition owner"))?,
            value,
            inputs,
            &mut fuel,
            value.span,
        );
        self.required_steps -= (before - fuel) as u64;
        result
    }

    /// Form the prefix once from a completed parent signature. Both signature
    /// construction and all ordinary local-function call paths consume it.
    /// A missing parent signature is an error, never an empty capture list.
    pub(super) fn lexical_input_plan(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<LexicalInputPlan, Error> {
        if let Some(plan) = self.lexical_input_plans.get(&owner) {
            return Ok(plan.clone());
        }
        let Some(parent) = self.package.lexical_parents.get(&owner).copied() else {
            return Ok(LexicalInputPlan::default());
        };
        self.ensure_signature(parent, span)?;
        // Scope availability and a declaration's captured prefix are distinct.
        // An unused nested helper may read an ancestor that its parent does not
        // itself capture; a caller of that helper acquires the dependency through
        // the declaration-reference closure.
        let mut ancestors = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = parent;
        loop {
            if !seen.insert(cursor) {
                return Err(error(span, "cyclic lexical declaration ownership"));
            }
            ancestors.push(cursor);
            let Some(next) = self.package.lexical_parents.get(&cursor).copied() else {
                break;
            };
            cursor = next;
        }
        let mut inputs = Vec::<LexicalInput>::new();
        let mut staged_domains = BTreeMap::new();
        for parent in ancestors.into_iter().rev() {
            let parameters = self.type_parameters[&parent]
                .iter()
                .copied()
                .chain(self.nominal_capture_ids(parent))
                .collect::<Vec<_>>();
            for binder in self.input_binders(&parameters) {
                if !inputs
                    .iter()
                    .any(|input| input.declaration == binder.declaration)
                {
                    inputs.push(LexicalInput {
                        declaration: binder.declaration,
                        sort: binder.sort,
                        supply: LexicalSupply::Declaration,
                    });
                }
            }
            for (binding, declaration, sort, domain) in self.lexical_staged_inputs(parent, span)? {
                if !inputs.iter().any(|input| input.declaration == declaration) {
                    inputs.push(LexicalInput {
                        declaration,
                        sort,
                        supply: LexicalSupply::Staged(binding),
                    });
                }
                staged_domains.insert(declaration, domain);
            }
        }
        let mut fuel = self.lexical_dependency_steps;
        let selected = (|| {
            let mut needed = dependencies::collect(self, owner, &mut fuel, span)?;
            let mut observed = BTreeSet::new();
            loop {
                let mut changed = false;
                for input in &inputs {
                    fuel = fuel.checked_sub(1).ok_or_else(|| {
                        error(span, "lexical input domain dependency budget exhausted")
                    })?;
                    if !needed.contains(&input.declaration) || !observed.insert(input.declaration) {
                        continue;
                    }
                    changed = true;
                    let domain = staged_domains
                        .get(&input.declaration)
                        .cloned()
                        .or_else(|| self.trait_input_domain(input.declaration));
                    if let Some(domain) = domain {
                        nir::type_computations::rigid_declarations_bounded(
                            &domain,
                            &mut fuel,
                            &mut needed,
                        )
                        .map_err(|e| error(span, e.to_string()))?;
                    }
                }
                if !changed {
                    break;
                }
            }
            Ok::<_, Error>(needed)
        })();
        self.lexical_dependency_steps = fuel;
        let needed = selected?;
        inputs.retain(|input| needed.contains(&input.declaration));
        // Domain metadata describes available lexical binders, including inputs
        // required only by descendants. Publish it after observation succeeds.
        self.lexical_input_domains.extend(staged_domains);
        let plan = LexicalInputPlan { inputs };
        self.lexical_input_plans.insert(owner, plan.clone());
        Ok(plan)
    }

    /// Lexical inputs are fixed by the enclosing instance; source arguments
    /// instantiate only the local function's explicit clause telescope.
    pub(super) fn lexical_function_arguments(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(..))
        ) {
            return Ok(Vec::new());
        }
        let plan = self.lexical_input_plan(owner, span)?;
        let mut arguments = Vec::with_capacity(plan.inputs.len());
        for input in plan.inputs {
            let value = match input.supply {
                LexicalSupply::Declaration => {
                    if let Some(value) = self.alias_substitution.get(&input.declaration).cloned() {
                        self.copy_alias_type(&value, span)?
                    } else {
                        Term::Rigid {
                            declaration: input.declaration,
                            sort: input.sort,
                        }
                    }
                }
                LexicalSupply::Staged(binding) => {
                    // Descendants use their fixed lexical input, not the
                    // parent's Bound index: a recursive callee's own parameter
                    // at that index can have a completely different sort.
                    if let Some((value, _)) = self.captured_staged_input(binding, span)? {
                        value
                    } else {
                        let value = if input.sort == Sort::Type {
                            self.type_input_bindings.get(&local(binding)).cloned()
                        } else {
                            self.value_input_bindings
                                .get(&local(binding))
                                .map(|(value, _)| value.clone())
                        };
                        value.ok_or_else(|| {
                            error(span, "lexical staged input is unavailable at this use")
                        })?
                    }
                }
            };
            arguments.push(value);
        }
        Ok(arguments)
    }

    /// Distinguish an unimplemented invocation environment from an unresolved
    /// name. Only diagnose after ordinary locals/closure captures were tried:
    /// lambdas already carry invocation values through their own environments.
    pub(super) fn unavailable_lexical_input(&self, binding: hir::HirId) -> &'static str {
        let fallback = "local binding is unavailable in this scope";
        let Some(mut owner) = self.signature_owner else {
            return fallback;
        };
        // Signature checking has no invocation values. Report the phase
        // boundary after staged inputs and ordinary locals have been tried,
        // including when a normal-form input uses the shared expression path.
        if owner == binding.owner
            && (self.active_signatures.contains(&owner) || self.checking_contract)
            && let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                self.package.item(owner).map(|item| &item.kind)
            && signature
                .params
                .iter()
                .any(|parameter| parameter.hir_id == binding && !parameter.ty.is_comptime())
        {
            return if signature.modifiers.is_comptime {
                "type annotation depending on an ordinary comptime-function input requires invocation-time type elaboration, which is not yet implemented; local binding is unavailable in this scope"
            } else {
                "normal-form annotation cannot read an ordinary runtime input; its value must be available at comptime; local binding is unavailable in this scope"
            };
        }
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.package.lexical_parents.get(&owner).copied() {
            if !seen.insert(parent) {
                return fallback;
            }
            if parent == binding.owner {
                if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                    self.package.item(parent).map(|item| &item.kind)
                {
                    if signature.modifiers.is_comptime
                        && signature.params.iter().any(|parameter| {
                            parameter.hir_id == binding && !parameter.ty.is_comptime()
                        })
                    {
                        return "capturing an ordinary comptime function parameter in a local declaration requires an invocation environment, which is not yet implemented";
                    }
                }
                return fallback;
            }
            owner = parent;
        }
        fallback
    }

    pub(super) fn lift_lexical_type(
        &self,
        parent: OwnerId,
        ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(parent).map(|item| &item.kind)
        else {
            return Ok(ty.clone());
        };
        let mut frame = vec![None; sig.params.len()];
        for (binding, id, sort, _) in self.lexical_staged_inputs(parent, span)? {
            let index = sig
                .params
                .iter()
                .position(|parameter| parameter.hir_id == binding)
                .unwrap();
            frame[index] = Some(Term::Rigid {
                declaration: id,
                sort,
            });
        }
        let lifted = ty
            .substitute_partial(&frame)
            .map_err(|e| error(span, e.to_string()))?;
        if !lifted
            .substitution_parameters()
            .map_err(|e| error(span, e.to_string()))?
            .is_empty()
        {
            return Err(error(
                span,
                "lexical type requirement depends on an unavailable runtime input",
            ));
        }
        Ok(lifted)
    }

    pub(super) fn lexical_staged_inputs(
        &self,
        parent: OwnerId,
        span: Span,
    ) -> Result<Vec<(hir::HirId, DeclarationId, Sort, Term)>, Error> {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(parent).map(|item| &item.kind)
        else {
            return Ok(Vec::new());
        };
        let domains = if let Some(signature) = self
            .signatures
            .get(&parent)
            .or_else(|| self.interface_signatures.get(&parent))
        {
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Ok(Vec::new());
            };
            parameters
                .iter()
                .map(|p| Some((p.sort, p.ty.clone())))
                .collect::<Vec<_>>()
        } else if let Some(definition) = self.open_functions.get(&parent) {
            definition
                .input_domains()
                .into_iter()
                .map(|(sort, domain)| domain.map(|domain| (sort, domain)))
                .collect()
        } else {
            return Err(error(span, "lexical parent signature is unavailable"));
        };
        let mut result = Vec::new();
        let mut frame = vec![None; sig.params.len()];
        for (index, parameter) in sig.params.iter().enumerate() {
            if !parameter.ty.is_comptime() {
                continue;
            }
            let (sort, domain) = domains
                .get(index)
                .and_then(Option::as_ref)
                .ok_or_else(|| error(span, "lexical staged input requires a formed domain"))?;
            let id = self
                .package
                .staged_parameter_definitions
                .get(&parameter.hir_id)
                .ok_or_else(|| error(span, "staged parameter has no source identity"))?;
            let id = DeclarationId {
                package: semantic_package(&self.package_identities, id.pkg),
                local: id.index,
            };
            let domain = domain
                .substitute_partial(&frame)
                .map_err(|e| error(span, e.to_string()))?;
            if !domain
                .substitution_parameters()
                .map_err(|e| error(span, e.to_string()))?
                .is_empty()
            {
                return Err(error(
                    span,
                    "lexical staged input domain depends on an unavailable runtime input",
                ));
            }
            frame[index] = Some(Term::Rigid {
                declaration: id,
                sort: *sort,
            });
            result.push((parameter.hir_id, id, *sort, domain));
        }
        Ok(result)
    }

    pub(super) fn captured_staged_input(
        &mut self,
        binding: hir::HirId,
        span: Span,
    ) -> Result<Option<(Term, Term)>, Error> {
        let Some(mut owner) = self.signature_owner else {
            return Ok(None);
        };
        if owner == binding.owner {
            return Ok(None);
        }
        let Some(id) = self.package.staged_parameter_definitions.get(&binding) else {
            return Ok(None);
        };
        let id = DeclarationId {
            package: semantic_package(&self.package_identities, id.pkg),
            local: id.index,
        };
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.package.lexical_parents.get(&owner).copied() {
            if !seen.insert(parent) {
                return Err(error(span, "cyclic lexical definition environment"));
            }
            if parent == binding.owner {
                let domain = self
                    .lexical_input_domains
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| error(span, "lexical staged input domain is unavailable"))?;
                let domain = self.substituted_trait_domain(&domain, span)?;
                let symbol = if let Some(value) = self.alias_substitution.get(&id).cloned() {
                    self.copy_alias_type(&value, span)?
                } else {
                    Term::Rigid {
                        declaration: id,
                        sort: if domain == p(P::Type) {
                            Sort::Type
                        } else {
                            Sort::ComptimeValue
                        },
                    }
                };
                return Ok(Some((symbol, domain)));
            }
            owner = parent;
        }
        Ok(None)
    }
}
