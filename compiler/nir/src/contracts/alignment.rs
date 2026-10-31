//! Alignment of an already type-instantiated interface contract to a body frame.
use super::*;
use std::collections::BTreeSet;

/// A substitution plan, not evidence that the program predicates hold.
/// Consumers rewrite predicates on a private candidate before publishing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingAlignment {
    pub aliases: BTreeMap<LocalId, LocalId>,
    pub result_binding: Option<LocalId>,
    pub entry_snapshots: Vec<EntrySnapshot>,
}

fn frame(
    parameters: &[LocalId],
    result: Option<LocalId>,
    snapshots: &[EntrySnapshot],
) -> Result<BTreeSet<LocalId>, &'static str> {
    let parameters_set: BTreeSet<_> = parameters.iter().copied().collect();
    if parameters_set.len() != parameters.len() {
        return Err("duplicate program contract parameter binding");
    }
    let mut used = parameters_set.clone();
    if result.is_some_and(|id| !used.insert(id)) {
        return Err("program contract result binding collides with a parameter");
    }
    let mut observed = BTreeSet::new();
    for snapshot in snapshots {
        if !parameters_set.contains(&snapshot.parameter) {
            return Err("program contract snapshot does not refer to a parameter");
        }
        if !observed.insert(snapshot.parameter) || !used.insert(snapshot.binding) {
            return Err("duplicate or colliding program contract snapshot binding");
        }
    }
    Ok(used)
}

impl ProgramContract {
    /// Parameter identities follow positional correspondence. Old-value
    /// observations reuse the body's snapshot for that parameter, if present;
    /// otherwise their interface identity remains distinct and is retained.
    pub fn binding_alignment(
        &self,
        source_parameters: &[LocalId],
        target_parameters: &[LocalId],
        target_result: Option<LocalId>,
        target_snapshots: &[EntrySnapshot],
        fuel: &mut usize,
    ) -> Result<BindingAlignment, &'static str> {
        let cost = source_parameters
            .len()
            .checked_add(target_parameters.len())
            .and_then(|n| n.checked_add(self.entry_snapshots.len()))
            .and_then(|n| n.checked_add(target_snapshots.len()))
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(2))
            .ok_or("program contract alignment budget exhausted")?;
        *fuel = fuel.checked_sub(cost).ok_or_else(|| {
            *fuel = 0;
            "program contract alignment budget exhausted"
        })?;
        if source_parameters.len() != target_parameters.len() {
            return Err("program contract parameter count mismatch");
        }
        frame(
            source_parameters,
            self.result_binding,
            &self.entry_snapshots,
        )?;
        let mut target_used = frame(target_parameters, target_result, target_snapshots)?;
        let mut aliases: BTreeMap<_, _> = source_parameters
            .iter()
            .copied()
            .zip(target_parameters.iter().copied())
            .collect();
        let result_binding = if let Some(source) = self.result_binding {
            let target = if let Some(target) = target_result {
                target
            } else {
                if !target_used.insert(source) {
                    return Err("retained interface result collides with the body frame");
                }
                source
            };
            aliases.insert(source, target);
            Some(target)
        } else {
            None
        };
        let snapshots: BTreeMap<_, _> = target_snapshots
            .iter()
            .map(|s| (s.parameter, s.binding))
            .collect();
        let mut entry_snapshots = Vec::with_capacity(self.entry_snapshots.len());
        for source in &self.entry_snapshots {
            let parameter = aliases[&source.parameter];
            let binding = if let Some(binding) = snapshots.get(&parameter) {
                *binding
            } else {
                if !target_used.insert(source.binding) {
                    return Err("retained interface snapshot collides with the body frame");
                }
                source.binding
            };
            aliases.insert(source.binding, binding);
            entry_snapshots.push(EntrySnapshot { parameter, binding });
        }
        Ok(BindingAlignment {
            aliases,
            result_binding,
            entry_snapshots,
        })
    }
}

impl fmt::Display for BindingAlignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(contract-binding-alignment (aliases")?;
        for (from, to) in &self.aliases {
            write!(f, " (alias {from} {to})")?;
        }
        f.write_str(") (result-binding")?;
        if let Some(result) = self.result_binding {
            write!(f, " {result}")?;
        }
        f.write_str(") (entry-snapshots")?;
        for snapshot in &self.entry_snapshots {
            write!(f, " {snapshot}")?;
        }
        f.write_str("))")
    }
}
