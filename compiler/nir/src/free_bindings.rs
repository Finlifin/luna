//! Lexical value dependencies of a checked expression. Binding scopes here
//! follow interpretation; type-level bound slots are handled by substitution.
use crate::program::{Expr, LocalId, Pattern as P};
use std::collections::{BTreeMap, BTreeSet};

mod effects;
pub use effects::{
    Access, access, remove_pattern_bindings, retain_unmodified, retain_unmodified_argument,
};
mod lexical;
mod walk;
pub use crate::visit::Error;
use lexical::{Alternatives, LexicalScope};
pub use walk::{
    collect_arm_with_dependencies, collect_pattern_with_dependencies, collect_with_dependencies,
    input_plan_bindings,
};

type Dependencies = BTreeMap<LocalId, Scope>;
type Scope = BTreeSet<LocalId>;
fn charge(fuel: &mut usize, amount: usize) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or(Error::Budget("capture analysis budget exhausted"))?;
    Ok(())
}
/// Expand one symbolic read in its use-site scope. Another use may be
/// shadowed by a different pattern, default, or closure binder.
fn read(
    mut id: LocalId,
    scope: &LexicalScope,
    free: &mut Scope,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<(), Error> {
    enum Visit {
        Active,
        Complete,
    }
    let mut visits = BTreeMap::new();
    let mut path = Vec::new();
    loop {
        charge(fuel, 1)?;
        if !scope.contains(&id) {
            if let Some(inputs) = dependencies.get(&id) {
                match visits.entry(id) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(Visit::Active);
                        path.push((id, inputs.iter()));
                    }
                    std::collections::btree_map::Entry::Occupied(slot) => {
                        if matches!(slot.get(), Visit::Active) {
                            return Err(Error::Invalid("cyclic symbolic capture dependency"));
                        }
                    }
                }
            } else {
                free.insert(id);
            }
        }
        // Retain a cursor per active node. A wide dependency list need not
        // be copied or pre-enqueued before its first edge can be charged.
        loop {
            let Some((parent, edges)) = path.last_mut() else {
                return Ok(());
            };
            if let Some(next) = edges.next() {
                id = *next;
                break;
            }
            visits.insert(*parent, Visit::Complete);
            path.pop();
        }
    }
}
pub fn collect(expr: &Expr, fuel: &mut usize) -> Result<Scope, Error> {
    collect_with_dependencies(expr, &Dependencies::new(), fuel)
}
/// Bindings of the operand schedule after the callee has been selected.
pub struct InputPlanBindings {
    pub imports: BTreeSet<LocalId>,
    pub locals: Vec<LocalId>,
}

/// Runtime dependencies of pattern observations, excluding lexical binders.
pub fn collect_pattern(pattern: &P, fuel: &mut usize) -> Result<Scope, Error> {
    collect_pattern_with_dependencies(pattern, &Dependencies::new(), fuel)
}

/// Lexical dependencies of one handler/match arm, excluding its own pattern binders.
pub fn collect_arm(arm: &crate::program::MatchArm, fuel: &mut usize) -> Result<Scope, Error> {
    collect_arm_with_dependencies(arm, &Dependencies::new(), fuel)
}
