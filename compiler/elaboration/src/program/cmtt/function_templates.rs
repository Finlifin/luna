//! Bind definition-supplied expression code before checking a shared function body.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn function_body(
        &mut self,
        owner: OwnerId,
        body: &Expr<'_>,
        result: &Term,
    ) -> Result<n::Expr, Error> {
        self.with_function_template_body(owner, result, |checker| {
            checker.computation_body(body, Some(result))
        })
    }

    pub(in crate::program) fn with_function_template_body<R>(
        &mut self,
        owner: OwnerId,
        result: &Term,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let Some(template) = self.package.function_template_bodies.get(&owner).cloned() else {
            return check(self);
        };
        let saved_locals = self.locals.clone();
        let saved_code = self.code_values.clone();
        let saved_checking = self.checking_code_generator;
        // The supplied runtime body belongs to this generated function. Check
        // its requests in that function's declared row, not the environment of
        // whichever body was elaborated before code construction started.
        let effects = match result {
            Term::Computation { effects, .. } => Some(effects.clone()),
            _ => None,
        };
        let saved_effects = std::mem::replace(&mut self.effect_environment, effects);
        let checked = (|| {
            let formal = self.code_interface(template.interface.ty)?;
            let code = if let Some(supplied) = template.supplied {
                let mut actual = formal.clone();
                actual.context.clear();
                actual.ports.clear();
                actual.defaults.clear();
                for port in &template.ports {
                    let from = local(
                        port.formal
                            .ok_or_else(|| error(port.name.span, "unlinked template port"))?,
                    );
                    let to = local(port.binding);
                    let ty = formal
                        .context
                        .get(&from)
                        .ok_or_else(|| error(port.name.span, "unknown formal template port"))?
                        .clone();
                    actual.context.insert(to, ty.clone());
                    actual.ports.insert(port.name.name.to_string(), to);
                    self.locals.insert(to, ty);
                }
                let supplied = self.package.body(supplied).ok_or_else(|| {
                    error(template.interface.ty.span, "missing supplied template body")
                })?;
                self.checking_code_generator = false;
                // Return targets belong to the resulting function; ordinary
                // quote construction continues to have no outer return target.
                let return_type = match result {
                    Term::Computation { result, .. } => result.as_ref().clone(),
                    result => result.clone(),
                };
                let code = self.quote_expression(
                    supplied.value,
                    supplied.value,
                    &[],
                    Some(&actual),
                    None,
                    Some(return_type),
                )?;
                self.ascribe_code(code, Some(&formal), supplied.value.span)?
            } else {
                Arc::new(ExpressionCode {
                    body: None,
                    result: formal.result,
                    context: formal.context,
                    ports: formal.ports,
                    defaults: formal.defaults,
                })
            };
            // Contextual ports are virtual while constructing the code. Setup
            // must establish real locals and satisfy each splice connection.
            self.locals = saved_locals.clone();
            self.code_values
                .insert(local(template.interface.parameter), code);
            self.checking_code_generator = template.supplied.is_none();
            check(self)
        })();
        self.locals = saved_locals;
        self.code_values = saved_code;
        self.checking_code_generator = saved_checking;
        self.effect_environment = saved_effects;
        checked
    }
}
