//! Bind template clauses at definition formation, independently of runtime calls.
use crate::program::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn definition_input(
        &mut self,
        declaration: DeclarationId,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let Some(value) = self.definition_inputs.get(&declaration) else {
            return Ok(None);
        };
        constants::charge_copy(value, &mut self.required_steps, span)?;
        let mut value = value.clone();
        value.source = source(span);
        Ok(Some(value))
    }

    pub(in crate::program) fn prepare_template_application(
        &mut self,
        owner: OwnerId,
    ) -> Result<(), Error> {
        if self.prepared_template_applications.contains(&owner) {
            return Ok(());
        }
        let Some(application) = self.package.function_template_applications.get(&owner) else {
            return Ok(());
        };
        let span = application.template.span;
        let H::NFApplication(head, inputs) = application.template.kind else {
            return Err(error(span, "template application has no clause input list"));
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Err(error(span, "template application requires a declaration"));
        };
        let target = self
            .package
            .definition_owner(def_id)
            .ok_or_else(|| error(span, "missing template application declaration"))?;
        self.ensure_signature(target, span)?;
        self.ensure_contract(target, span)?;
        self.check_type_defaults(target)?;

        // Ordinary clause binding owns source evaluation order, dependent domains,
        // defaults, named arguments and spread handling. These are fixed inputs
        // of the definition, not optional parameters of its later runtime calls.
        // The application is outside the interface it is about to form. Its
        // required computations may use enclosing evidence, but cannot assume
        // the instance's not-yet-substituted requires clauses or runtime inputs.
        let enclosing = self
            .package
            .lexical_parents
            .get(&owner)
            .or_else(|| self.self_containers.get(&owner))
            .copied();
        if let Some(parent) = self.package.lexical_parents.get(&owner).copied() {
            self.ensure_contract(parent, span)?;
        }
        // Evaluate in the parent's parameter frame, then lift the completed
        // inputs into the nested definition's lexical telescope. Keeping Bound
        // indices here also keeps the parent's requirements in the same frame.
        let staged = if let Some(parent) = enclosing {
            self.lexical_staged_inputs(parent, span)?
        } else {
            Vec::new()
        };
        let mut declarations = Vec::new();
        let mut frame = Vec::new();
        if let Some(parent) = enclosing
            && let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
                self.package.item(parent).map(|item| &item.kind)
        {
            for (binding, id, sort, _) in &staged {
                declarations.push(*id);
                frame.push(Term::Bound {
                    depth: 0,
                    index: sig
                        .params
                        .iter()
                        .position(|p| p.hir_id == *binding)
                        .unwrap(),
                    sort: *sort,
                });
            }
        }
        let mut saved_bindings = Vec::new();
        let saved_owner = std::mem::replace(&mut self.signature_owner, enclosing);
        let saved_context = self.context.clone();
        self.context = enclosing.map_or_else(
            || self.nominal_context(),
            |owner| self.definition_context(owner),
        );
        let bound = (|| {
            let mut bindings = Vec::new();
            for ((binding, _, sort, domain), symbol) in staged.iter().zip(&frame) {
                let domain = self.instantiate_input_types(domain, &declarations, &frame, span)?;
                bindings.push((local(*binding), *sort, symbol.clone(), domain));
            }
            for (binding, sort, symbol, domain) in bindings {
                let ty = self.type_input_bindings.remove(&binding);
                let value = self.value_input_bindings.remove(&binding);
                let local = self.locals.insert(binding, domain.clone());
                saved_bindings.push((binding, ty, value, local));
                if sort == Sort::Type {
                    self.type_input_bindings.insert(binding, symbol);
                } else {
                    self.value_input_bindings.insert(binding, (symbol, domain));
                }
            }
            let arguments = self.type_clause_arguments(target, inputs, None, span)?;
            self.check_requirements(target, &arguments, span)?;
            Ok(arguments)
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        for (binding, ty, value, local) in saved_bindings {
            self.type_input_bindings.remove(&binding);
            self.value_input_bindings.remove(&binding);
            self.locals.remove(&binding);
            self.type_input_bindings.extend(ty.map(|ty| (binding, ty)));
            self.value_input_bindings
                .extend(value.map(|value| (binding, value)));
            self.locals.extend(local.map(|ty| (binding, ty)));
        }
        let mut arguments = bound?;
        if let Some(parent) = enclosing {
            for argument in &mut arguments {
                *argument = self.lift_lexical_type(parent, argument, span)?;
            }
        }
        let parameters = self.type_parameters[&target].clone();
        let domains = self.trait_domains(target)?;
        if arguments.len() != parameters.len() || domains.len() != parameters.len() {
            return Err(error(span, "template application input telescope mismatch"));
        }
        let mut completed = Vec::new();
        for clause in &application.clauses {
            let formal = DeclarationId {
                package: semantic_package(&self.package_identities, clause.formal.pkg),
                local: clause.formal.index,
            };
            let binding = DeclarationId {
                package: semantic_package(&self.package_identities, clause.binding.pkg),
                local: clause.binding.index,
            };
            let slot = parameters
                .iter()
                .position(|parameter| *parameter == formal)
                .ok_or_else(|| error(span, "template clause has no declaration input"))?;
            let ty = self.instantiate_input_types(&domains[slot], &parameters, &arguments, span)?;
            let value = self.copy_alias_type(&arguments[slot], span)?;
            completed.push((
                binding,
                n::Expr {
                    source: source(span),
                    kind: if ty == p(P::Type) {
                        K::TypeValue(value)
                    } else {
                        K::ComptimeValue(value)
                    },
                    ty,
                },
            ));
        }
        // Publish only after every input and formation requirement succeeds.
        // The enclosing Checker remains a private elaboration candidate.
        self.definition_inputs.extend(completed);
        self.prepared_template_applications.insert(owner);
        Ok(())
    }
}
