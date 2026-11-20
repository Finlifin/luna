//! Source dispatch projects dependencies into the query's declaration world.
//! Executable function ownership belongs to IIR materialization.
use super::*;

impl Builder<'_> {
    pub(super) fn dispatch_function(
        &mut self,
        function: &n::Function,
        arguments: &[Term],
    ) -> Result<(), String> {
        let domains = self
            .world
            .input_domains(&function.type_parameters, &mut self.fuel)?;
        let frame = [
            nir::function_parameters::ordinary_frame(&function.signature, &mut self.fuel)
                .map_err(|e| e.to_string())?,
        ];
        let projection = nir::DeclarationSubstitution::bounded_domains(
            &function.type_parameters,
            &domains,
            arguments,
            nir::DeclarationInputScope::ClosedInBody(&frame),
            &mut self.fuel,
        )
        .map_err(|e| e.to_string())?;
        let signature = projection
            .instantiate_closed_signature_bounded(&function.signature, &mut self.fuel)
            .map_err(|e| e.to_string())?;
        let retained = matches!(
            signature.callable_signature(),
            Term::Function { parameters, .. }
                if parameters.iter().any(|p| p.sort != nir::Sort::Runtime)
        );
        self.queue.push_back(Task::Type(signature));
        let mut explicit_envs = Vec::with_capacity(function.env_parameters.len());
        for env in &function.env_parameters {
            let substituted = projection
                .instantiate_bounded(
                    &Term::Rigid {
                        declaration: env.declaration,
                        sort: nir::Sort::Runtime,
                    },
                    &mut self.fuel,
                )
                .map_err(|e| e.to_string())?;
            let (declaration, runtime_id) = match substituted {
                Term::Runtime(id) => (env.declaration, Some(id)),
                Term::Rigid {
                    declaration: next,
                    sort: nir::Sort::Runtime,
                } => (next, env.runtime_id),
                _ => (env.declaration, env.runtime_id),
            };
            let ty = projection
                .instantiate_bounded(&env.ty, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            self.queue.push_back(Task::Type(ty.clone()));
            explicit_envs.push(n::RuntimeEnvParameter {
                declaration,
                runtime_id,
                binding: env.binding,
                ty,
            });
        }
        if function.phase == n::FunctionPhase::Runtime {
            let implicit = nir::program::implicit_runtime_env_parameters(
                &self.package.runtime_domains,
                &self.package.global_runtime_deliveries,
                arguments,
                &explicit_envs,
            )?;
            for env in implicit {
                self.queue.push_back(Task::Type(env.ty));
            }
        }
        if retained {
            // Ordinary staged inputs defer dependency selection until invocation.
            // Still check every type operand under this declaration substitution.
            projection
                .check_retained_body(function, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            if !self.retain_templates {
                return Err(
                    "requested dispatch function still requires ordinary invocation input staging"
                        .into(),
                );
            }
            return Ok(());
        }
        for predicate in &function.requirements {
            let predicate = projection
                .instantiate_predicate_bounded(predicate, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            self.predicate_dependencies(&predicate, 0)?;
        }
        for capability in &function.capabilities {
            let capability = projection
                .instantiate_requirement_bounded(capability, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            self.queue.push_back(Task::Interface(
                n::InterfaceInstance {
                    declaration: capability.interface,
                    arguments: capability.interface_arguments,
                },
                capability.subject,
            ));
        }
        for expr in std::iter::once(&function.body)
            .chain(&function.preconditions)
            .chain(&function.postconditions)
            .chain(function.defaults.iter().map(|(_, supplier)| supplier))
            .chain(function.runtime_deliveries.values())
        {
            let expr = projection
                .instantiate_expression_bounded(expr, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            self.expr(&expr, 0, function.phase == n::FunctionPhase::Comptime)?;
        }
        Ok(())
    }
}
