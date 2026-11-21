//! Runtime acquisition accompanies a completed Type use in the active checker.
//! Its ordered operations are independent of the normalized Type identity.
use super::*;
use std::sync::Arc;

type Plan = Arc<Vec<nir::RuntimeId>>;
#[derive(Clone, Default)]
pub(super) enum ChildPlans {
    #[default]
    Outside,
    Forming(Option<Arc<Vec<Plan>>>),
}

impl Checker<'_, '_> {
    pub(super) fn completed_type(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<type_data::CompletedType, Error> {
        self.completed_type_in(expr, false)
    }

    pub(super) fn completed_type_in(
        &mut self,
        expr: &Expr<'_>,
        describe_input: bool,
    ) -> Result<type_data::CompletedType, Error> {
        let first = self.next_runtime_id;
        let parent = std::mem::replace(
            &mut self.local_runtime_origins.child_type_inputs,
            ChildPlans::Forming(None),
        );
        let result = self.complete_type_data(expr, describe_input);
        let ChildPlans::Forming(children) =
            std::mem::replace(&mut self.local_runtime_origins.child_type_inputs, parent)
        else {
            return Err(error(expr.span, "type input formation scope was lost"));
        };
        let result = result?;
        if self.next_runtime_id == first {
            self.local_runtime_origins.type_inputs.remove(&expr.hir_id);
            return Ok(result);
        }
        let args = match expr.kind {
            H::NFApplication(_, args) => args,
            _ => &[],
        };
        let ty = match &result {
            type_data::CompletedType::Identity(ty) => Some(ty),
            type_data::CompletedType::Description(_) => None,
        };
        let inputs = self.ordered_type_inputs(
            args,
            ty,
            first,
            children.as_deref().map_or(&[], Vec::as_slice),
            expr.span,
        )?;
        if inputs.is_empty() {
            self.local_runtime_origins.type_inputs.remove(&expr.hir_id);
        } else {
            let plan = Arc::new(inputs);
            self.local_runtime_origins
                .type_inputs
                .insert(expr.hir_id, plan.clone());
            if let ChildPlans::Forming(siblings) = &mut self.local_runtime_origins.child_type_inputs
            {
                let siblings = siblings.get_or_insert_with(Default::default);
                if Arc::get_mut(siblings).is_none() {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(siblings.len() as u64)
                        .ok_or_else(|| error(expr.span, "type input scope budget exhausted"))?;
                }
                Arc::make_mut(siblings).push(plan);
            }
        }
        Ok(result)
    }

    pub(super) fn ordered_type_inputs(
        &mut self,
        args: &[Arg<'_>],
        ty: Option<&Term>,
        first: u32,
        children: &[Plan],
        span: Span,
    ) -> Result<Vec<nir::RuntimeId>, Error> {
        let mut order = Vec::new();
        let mut seen = BTreeSet::new();
        let mut append = |id: nir::RuntimeId| {
            if id.local >= first && seen.insert(id) {
                order.push(id);
            }
        };
        for arg in args {
            let (Arg::Positional(value)
            | Arg::Named(_, value)
            | Arg::Expand(value)
            | Arg::Implicit(value)
            | Arg::PostLambda(value)) = arg;
            let nested = self.local_runtime_origins.type_inputs.get(&value.hir_id);
            let cost = 1 + nested.map_or(0, Vec::len) as u64;
            self.required_steps = self
                .required_steps
                .checked_sub(cost)
                .ok_or_else(|| error(span, "type input plan budget exhausted"))?;
            if let Some(nested) = nested {
                nested.iter().copied().for_each(&mut append);
            }
            if let Some(Term::Runtime(id)) = self.local_runtime_origins.sources.get(&value.hir_id)
                && self
                    .local_runtime_deliveries
                    .get(id)
                    .is_some_and(|entry| entry.producer_site == Some(local(value.hir_id)))
            {
                append(*id);
            }
        }
        if let Some(ty) = ty
            && (!children.is_empty() || !self.local_runtime_origins.defaults.is_empty())
        {
            let roots = self.result_dependencies(ty, span)?;
            for plan in children {
                self.required_steps = self
                    .required_steps
                    .checked_sub(plan.len() as u64)
                    .ok_or_else(|| error(span, "child type input plan budget exhausted"))?;
                if plan.iter().any(|id| roots.contains(id)) {
                    plan.iter().copied().for_each(&mut append);
                }
            }
            for id in roots {
                if self.local_runtime_origins.defaults.contains_key(&id) {
                    append(id);
                }
            }
        }
        Ok(order)
    }
}
