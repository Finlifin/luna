//! Program-domain refinement for contextual-code methods, before their erasure.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn check_code_method_preconditions(
        &mut self,
        public: &n::MethodContract,
        actual: &n::MethodContract,
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        // An omitted clause inherits the public domain at invocation.
        if actual.program.preconditions.is_empty() {
            return Ok(());
        }
        let parameter_ids = parameters
            .iter()
            .map(|parameter| parameter.declaration)
            .collect::<Vec<_>>();
        let domains = parameter_ids
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let specialized = public.program.instantiate_declaration_parameters(
            &parameter_ids,
            &domains,
            arguments,
            &[],
            &mut fuel,
        );
        self.required_steps -= (before - fuel) as u64;
        let mut preconditions = specialized
            .map_err(|e| error(span, e.to_string()))?
            .preconditions;
        let mut bindings: BTreeMap<_, _> = public
            .parameter_bindings
            .iter()
            .copied()
            .zip(actual.parameter_bindings.iter().copied())
            .collect();
        let mut clauses = BTreeMap::new();
        if parameters
            .iter()
            .any(|parameter| parameter.sort == Sort::ComptimeValue)
        {
            for (owner, _) in self.package.owners() {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "code contract binding budget exhausted"))?;
                for clause in self.type_clause_params(owner) {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(1)
                        .ok_or_else(|| error(span, "code contract binding budget exhausted"))?;
                    if let Some(id) = clause.definition {
                        clauses.insert(
                            DeclarationId {
                                package: semantic_package(&self.package_identities, id.pkg),
                                local: id.index,
                            },
                            local(clause.hir_id),
                        );
                    }
                }
            }
        }
        let mut values = BTreeMap::new();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            if parameter.sort != Sort::ComptimeValue {
                continue;
            }
            let binding = *clauses
                .get(&parameter.declaration)
                .ok_or_else(|| error(span, "code contract value binding is unavailable"))?;
            if let Term::Rigid {
                declaration,
                sort: Sort::ComptimeValue,
            } = argument
            {
                bindings.insert(
                    binding,
                    *clauses.get(declaration).ok_or_else(|| {
                        error(span, "code contract replacement binding is unavailable")
                    })?,
                );
            } else {
                values.insert(binding, argument);
            }
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let result = (|| -> Result<(), String> {
            for predicate in &mut preconditions {
                // Simultaneous rewriting: an inserted destination belongs to
                // the implementation frame and must not be substituted again.
                nir::visit::expression_mut_pruned(predicate, &mut fuel, |expr, fuel| {
                    let K::Local(binding) = expr.kind else {
                        return Ok(true);
                    };
                    if let Some(value) = values.get(&binding) {
                        expr.kind = match value {
                            Term::Bool(value) => K::Literal(n::Literal::Bool(*value)),
                            Term::Char(value) => K::Literal(n::Literal::Char(*value)),
                            Term::Integer(value) => {
                                let mut budget = comptime_runtime::Budget { steps: *fuel, max_limbs: 4096 };
                                let spelling = value.to_hex_literal(&mut budget);
                                *fuel = budget.steps;
                                K::Literal(n::Literal::Integer(spelling.map_err(|e| e.to_string())?))
                            }
                            _ => return Err("code contract captured value is outside the program prover's scalar fragment".into()),
                        };
                    }
                    Ok(false)
                })?;
                nir::contracts::remap_predicate(predicate, &bindings)?;
            }
            let mut inputs = BTreeMap::new();
            for predicate in preconditions.iter().chain(&actual.program.preconditions) {
                nir::visit::expression(predicate, &mut fuel, |expr, _| {
                    if let K::Local(binding) = expr.kind {
                        if let Some(previous) = inputs.insert(binding, expr.ty.clone()) {
                            if previous != expr.ty {
                                return Err("inconsistent code contract input domain".into());
                            }
                        }
                    }
                    Ok(())
                })?;
            }
            // This is a private, data-only implication obligation, NOT the
            // code method's callable type. No quote or contextual port enters
            // it, and it is never published in an execution package.
            let obligation = n::Function {
                declaration: actual.declaration,
                name: format!("{} program-domain refinement", actual.name),
                phase: n::FunctionPhase::Comptime,
                inline: false,
                pure: true,
                is_unsafe: false,
                signature: Term::Function {
                    parameters: inputs
                        .values()
                        .map(|ty| Parameter {
                            sort: Sort::Runtime,
                            ty: ty.clone(),
                        })
                        .collect(),
                    result: Box::new(p(P::Void)),
                },
                parameters: inputs.keys().copied().collect(),
                preconditions,
                postconditions: actual.program.preconditions.clone(),
                result_binding: None,
                entry_snapshots: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                defaults: vec![],
                body: n::Expr {
                    source: source(span),
                    ty: p(P::Void),
                    kind: K::Unit,
                },
            };
            let mut limits = crate::verification::Limits::default();
            limits.arithmetic.steps = limits.arithmetic.steps.min(fuel);
            let available = limits.arithmetic.steps;
            let checked = crate::verification::flow::analyze(&obligation, &mut limits);
            fuel -= available - limits.arithmetic.steps;
            checked
                .map_err(|e| format!("method `{}` strengthens trait program preconditions or their definedness is not proven: {e}", actual.name))?;
            Ok(())
        })();
        self.required_steps = fuel as u64;
        result.map_err(|message| error(span, message))
    }
}
