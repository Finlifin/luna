use super::*;

impl Emitter<'_> {
    /// Select only this invocation's code; stored function identities remain
    /// canonical, including functions chosen at runtime and recursive calls.
    pub(super) fn specialize_named_call(
        &mut self,
        source: nir::program::SourceRange,
        code: Value,
    ) -> Result<Value, String> {
        use iir::cleanup::ContextKey;
        let (Some(parent), Some(schedule)) = (self.cleanup, self.schedules) else {
            return Ok(code);
        };
        let mut candidates = Vec::new();
        for (index, candidate) in schedule.functions.iter().enumerate() {
            *self.fuel = self
                .fuel
                .checked_sub(1 + candidate.context.len())
                .ok_or("host named dispatch budget exhausted")?;
            if candidate.instance == parent.instance
                && candidate.context.len() == parent.context.len() + 1
                && candidate.context.starts_with(&parent.context)
                && let Some(ContextKey::Function { call, callee }) = candidate.context.last()
                && *call == source
            {
                candidates.push((callee, index, candidate));
            }
        }
        for alias in &schedule.aliases {
            *self.fuel = self
                .fuel
                .checked_sub(1 + alias.context.len())
                .ok_or("host recursive dispatch budget exhausted")?;
            if alias.instance != parent.instance
                || alias.context.len() != parent.context.len() + 1
                || !alias.context.starts_with(&parent.context)
            {
                continue;
            }
            let Some(ContextKey::Function { call, callee }) = alias.context.last() else {
                continue;
            };
            if *call != source {
                continue;
            }
            let target = cleanup::validate_alias(schedule, alias, self.fuel)?;
            *self.fuel = self
                .fuel
                .checked_sub(schedule.functions.len())
                .ok_or("host recursive dispatch target budget exhausted")?;
            let index = schedule
                .functions
                .iter()
                .position(|scope| std::ptr::eq(scope, target))
                .ok_or("host recursive dispatch target missing")?;
            candidates.push((callee, index, target));
        }
        let mut routes: BTreeMap<usize, (usize, &iir::cleanup::FunctionSchedule)> = BTreeMap::new();
        for (callee, context, candidate) in candidates {
            *self.fuel = self
                .fuel
                .checked_sub(self.graph.functions.len())
                .ok_or("host named dispatch identity budget exhausted")?;
            let index = self
                .graph
                .functions
                .iter()
                .position(|function| &function.key == callee)
                .ok_or("host named dispatch function missing")?;
            if let Some((_, previous)) = routes.get(&index) {
                if !cleanup::same_frame(schedule, previous, candidate, self.fuel)? {
                    return Err("host named call has ambiguous cleanup contexts".into());
                }
            } else {
                routes.insert(index, (context, candidate));
            }
        }
        let mut selected = code.clone();
        for (function, (context, _)) in routes {
            let scopes =
                cleanup::schedules_for(schedule, &self.graph.functions[function].key, self.fuel)?;
            let target =
                if cleanup::canonical_scope(&scopes).is_some_and(|(index, _)| index == context) {
                    format!("@flurry_{function}")
                } else {
                    format!("@flurry_{function}_scope_{context}")
                };
            let matches = self.instruction(
                Ty::Bool,
                format!("icmp eq ptr {}, @flurry_{function}", code.text),
            );
            selected = self.instruction(
                Ty::FunctionPointer,
                format!(
                    "select i1 {}, ptr {target}, ptr {}",
                    matches.text, selected.text
                ),
            );
        }
        Ok(selected)
    }
}
