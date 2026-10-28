//! Returned code identities belong to the factory's local environment. Factory
//! execution is counted separately by control(), never by this value query.
use super::*;

fn explicit_returns<'a>(
    root: &'a Expr,
    values: &mut Vec<&'a Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("effect usage return depth exhausted".into());
    }
    iir::visit::expression_pruned(root, fuel, |expr, fuel| {
        match &expr.kind {
            K::Return(Some(value)) => values.push(value),
            K::Closure { captures, .. } => {
                // Capture initializers execute here; returns in the nested
                // closure body belong to a different invocation.
                for capture in captures {
                    step(fuel, 1)?;
                    if let Some(value) = &capture.value {
                        explicit_returns(value, values, fuel, depth + 1)?;
                    }
                }
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    })
}

impl Analyzer<'_, '_, '_, '_> {
    pub(super) fn returned_callable(
        &mut self,
        factory: &Expr,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        self.returned_projection(factory, &[], n, fuel, depth)
    }

    pub(super) fn returned_projection(
        &mut self,
        factory: &Expr,
        path: &[callables::Projection],
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        step(fuel, n + 1)?;
        if depth >= 128 {
            return Err("effect usage return depth exhausted".into());
        }
        let key = match &factory.kind {
            K::Function(declaration) => iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            },
            K::FunctionInstance {
                declaration,
                arguments,
            } => {
                for argument in arguments {
                    super::super::memory::bound_type(argument, fuel)?;
                }
                iir::InstanceKey {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                }
            }
            _ => {
                step(fuel, path.len() + 1)?;
                let mut query = vec![callables::Projection::Returned];
                query.extend_from_slice(path);
                return self.projected_callable(factory, &query, n, fuel, depth + 1);
            }
        };
        let index = *self
            .instances
            .get(&key)
            .ok_or("effect usage factory instance missing")?;
        let function = &self.functions[index];
        let mut child = Analyzer {
            functions: self.functions,
            all_aliases: self.all_aliases,
            instances: self.instances,
            plan: self.plan,
            summaries: self.summaries,
            aliases: &self.all_aliases[index],
            instance: &function.key,
            report: None,
            dependencies: self.dependencies.as_deref_mut(),
        };
        child.body_return_projection(&function.function.body, path, n, fuel, depth + 1)
    }

    pub(super) fn closure_return_projection(
        &mut self,
        body: &Expr,
        path: &[callables::Projection],
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        let mut child = Analyzer {
            functions: self.functions,
            all_aliases: self.all_aliases,
            instances: self.instances,
            plan: self.plan,
            summaries: self.summaries,
            aliases: self.aliases,
            instance: self.instance,
            report: None,
            dependencies: self.dependencies.as_deref_mut(),
        };
        child.body_return_projection(body, path, n, fuel, depth)
    }

    fn body_return_projection(
        &mut self,
        body: &Expr,
        path: &[callables::Projection],
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        let mut values = Vec::new();
        // Conservatively retain explicit returns even in unreachable branches.
        // In particular a normal tail must never hide an earlier return.
        explicit_returns(body, &mut values, fuel, depth + 1)?;
        let mut tail = body;
        while let K::EffectComputation { body, .. } = &tail.kind {
            step(fuel, 1)?;
            tail = body;
        }
        values.push(tail);
        let mut result = vec![0; n];
        for value in values {
            step(fuel, 1)?;
            if value.ty == Term::Primitive(nir::Primitive::Never) {
                continue;
            }
            join(
                &mut result,
                &self.projected_callable(value, path, n, fuel, depth + 1)?,
                fuel,
            )?;
        }
        Ok(result)
    }
}
